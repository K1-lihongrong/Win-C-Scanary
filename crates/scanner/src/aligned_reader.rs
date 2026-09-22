//! 扇区对齐读取适配器。
//!
//! Windows 卷设备（\\.\X:）的原始读取要求**长度必须是扇区大小（通常 512）的整数倍**，
//! 否则 ReadFile 返回 os error 87（ERROR_INVALID_PARAMETER）。
//!
//! ntfs crate 内部的 binrw 会按结构体大小（可能非扇区对齐）调用 read_exact，
//! 直接对卷设备使用会触发 87。本适配器拦截 read 调用，把非对齐读取向上对齐到扇区大小。
//!
//! 设计要点：
//! - 维护一个按扇区对齐填充的窗口 buffer
//! - seek 只记录逻辑位置，标记窗口失效，不立即读
//! - read 时若窗口不含目标区间，重新按对齐边界填充窗口

use std::io::{self, Read, Seek, SeekFrom};

const SECTOR: u64 = 512;

pub struct AlignedReader<R> {
    inner: R,
    /// 窗口 buffer（按扇区对齐）
    buf: Vec<u8>,
    /// 窗口在文件中的起始偏移（扇区对齐）
    window_start: u64,
    /// 窗口有效长度
    window_len: usize,
    /// 窗口是否有效（seek 后失效，需重新填充）
    window_valid: bool,
    /// 当前逻辑读位置
    pos: u64,
}

impl<R: Read + Seek> AlignedReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buf: Vec::new(),
            window_start: 0,
            window_len: 0,
            window_valid: false,
            pos: 0,
        }
    }

    /// 判断当前窗口是否覆盖 [pos, pos+need)
    fn window_covers(&self, need: usize) -> bool {
        if !self.window_valid {
            return false;
        }
        let win_end = self.window_start + self.window_len as u64;
        self.pos >= self.window_start && (self.pos + need as u64) <= win_end
    }

    /// 按对齐边界填充窗口，使窗口覆盖 [pos, pos+need)
    fn fill_window(&mut self, need: usize) -> io::Result<()> {
        // 窗口起点对齐到扇区
        let start = self.pos / SECTOR * SECTOR;
        // 窗口终点向上对齐，且至少覆盖 pos+need
        let end = ((self.pos + need as u64).div_ceil(SECTOR)) * SECTOR;
        let mut want = (end - start) as usize;
        // 窗口至少一个扇区
        if want == 0 {
            want = SECTOR as usize;
        }
        // 关键：限制单次窗口大小上限，避免调用方传入超大 need 时 buffer 无限膨胀（OOM）。
        // 上层若需读取超过 MAX_WINDOW 的数据，会通过多次 read 完成（本适配器 read 返回 ≤ MAX_WINDOW）。
        const MAX_WINDOW: usize = 1 << 20; // 1 MiB
        if want > MAX_WINDOW {
            want = MAX_WINDOW;
        }
        if self.buf.len() < want {
            self.buf.resize(want, 0);
        }

        self.inner.seek(SeekFrom::Start(start))?;
        let mut filled = 0;
        while filled < want {
            match self.inner.read(&mut self.buf[filled..want]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => return Err(e),
            }
        }
        self.window_start = start;
        self.window_len = filled;
        self.window_valid = true;
        Ok(())
    }
}

impl<R: Read + Seek> Read for AlignedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if !self.window_covers(out.len()) {
            self.fill_window(out.len())?;
        }
        // 从窗口中取数据
        let offset = (self.pos - self.window_start) as usize;
        let avail = self.window_len.saturating_sub(offset);
        let n = avail.min(out.len());
        if n == 0 {
            return Ok(0);
        }
        out[..n].copy_from_slice(&self.buf[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for AlignedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => {
                let end = self.inner.seek(SeekFrom::End(0))? as i64;
                end + d
            }
        };
        if target < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek 到负偏移"));
        }
        self.pos = target as u64;
        // 窗口失效：不立即读，下次 read 时按需对齐填充
        if !self.window_covers(1) {
            self.window_valid = false;
        }
        Ok(self.pos)
    }
}
