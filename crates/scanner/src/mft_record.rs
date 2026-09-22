//! 极简 NTFS FILE 记录解析器（供 MFT 顺序读使用）。
//!
//! ntfs 0.4 的 `Record` 是 `pub(crate)`，无法从外部构造；顺序读整条 `$MFT` `$DATA` 流时
//! 需要自行解析每条 FILE 记录。本模块实现所需的最小子集：
//! - 应用更新序列（fixup）
//! - 遍历属性链，取 `$FILE_NAME`（名称/父目录 FRN/命名空间）与未命名 `$DATA`（大小/是否重解析点）
//!
//! 参考 ntfs 0.4 的 `record.rs` / `attribute.rs` 布局。

use std::path::Path;

/// 解析出的一条记录摘要
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: String,
    pub parent_frn: u64,
    /// 逻辑大小（non-resident 的 data_size；resident 的 value_length）
    pub own_size: u64,
    /// 实际分配大小（non-resident 的 allocated_size，簇对齐；resident 记 0）
    pub alloc_size: u64,
    pub is_dir: bool,
    pub is_reparse: bool,
}

const FILE_MAGIC: &[u8; 4] = b"FILE";

// 属性类型
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE_POINT: u32 = 0xC0;
const ATTR_END: u32 = 0xFFFF_FFFF;

// 命名空间
const NS_POSIX: u8 = 0;
const NS_WIN32: u8 = 1;
const NS_WIN32_AND_DOS: u8 = 3;

fn rd_u16(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn rd_u64(b: &[u8], off: usize) -> Option<u64> {
    b.get(off..off + 8).map(|s| {
        u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
    })
}

/// 应用更新序列（fixup）：把每条 512 字节扇区末尾的 USN 替换回原始字节。
fn apply_fixup(record: &mut [u8]) {
    if record.len() < 8 {
        return;
    }
    let usa_off = match rd_u16(record, 4) {
        Some(v) => v as usize,
        None => return,
    };
    let usa_count = match rd_u16(record, 6) {
        Some(v) => v as usize,
        None => return,
    };
    if usa_count == 0 || usa_off + usa_count * 2 > record.len() {
        return;
    }
    // usa[0] 是 USN，usa[1..] 是要写回各扇区末尾的真实字节
    let mut sector_end = 512 - 2;
    for i in 1..usa_count {
        if sector_end + 2 > record.len() {
            break;
        }
        let src = usa_off + i * 2;
        if src + 2 > record.len() {
            break;
        }
        record[sector_end] = record[src];
        record[sector_end + 1] = record[src + 1];
        sector_end += 512;
    }
}

/// 从 UTF-16LE 字节解析文件名
fn utf16_name(bytes: &[u8]) -> String {
    let u16s: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&u16s)
}

/// 解析一条 FILE 记录。无法解析（非 FILE、已删除等）时返回 None。
pub fn parse_record(record: &mut [u8]) -> Option<RawEntry> {
    if record.len() < 48 || &record[0..4] != FILE_MAGIC {
        return None;
    }

    // flags @ 0x16: bit0 = in use, bit1 = directory
    let flags = rd_u16(record, 0x16)?;
    let in_use = flags & 0x01 != 0;
    let is_dir = flags & 0x02 != 0;
    if !in_use {
        return None;
    }

    apply_fixup(record);

    let attrs_off = rd_u16(record, 0x14)? as usize;
    if attrs_off >= record.len() {
        return None;
    }

    let mut name = String::new();
    let mut parent_frn = 0u64;
    let mut own_size = 0u64;
    let mut alloc_size = 0u64;
    let mut is_reparse = false;

    let mut off = attrs_off;
    loop {
        let ty = match rd_u32(record, off) {
            Some(t) => t,
            None => break,
        };
        if ty == ATTR_END {
            break;
        }
        let attr_len = match rd_u32(record, off + 4) {
            Some(l) if l >= 16 => l as usize,
            _ => break,
        };
        if off + attr_len > record.len() {
            break;
        }
        let non_resident = record[off + 8] != 0;
        let name_len = record[off + 9] as usize; // UTF-16 code points
        let name_off = rd_u16(record, off + 10).unwrap_or(0) as usize;

        match ty {
            ATTR_FILE_NAME => {
                // $FILE_NAME 总是 resident
                if !non_resident {
                    let val_len = rd_u32(record, off + 16).unwrap_or(0) as usize;
                    let val_off = rd_u16(record, off + 20).unwrap_or(0) as usize;
                    let vstart = off + val_off;
                    // 布局：parent(8) + ... + name_len(1)@64 + namespace(1)@65 + name@66
                    if val_len >= 66 && vstart + 66 <= record.len() {
                        let ns = record[vstart + 65];
                        if ns == NS_POSIX || ns == NS_WIN32 || ns == NS_WIN32_AND_DOS {
                            if let Some(pf) = rd_u64(record, vstart) {
                                parent_frn = pf & 0x0000_FFFF_FFFF_FFFF;
                            }
                            let fname_len = record[vstart + 64] as usize;
                            let nstart = vstart + 66;
                            let nend = nstart + fname_len * 2;
                            if nend <= record.len() {
                                name = utf16_name(&record[nstart..nend]);
                            }
                        }
                    }
                }
            }
            ATTR_DATA => {
                // 只计未命名的 $DATA（name_len == 0）
                if name_len == 0 && !is_dir {
                    if non_resident {
                        // 关键：文件数据被 $ATTRIBUTE_LIST 拆成多个 $DATA 片段时，
                        // 只有 lowest_vcn == 0 的首片段携带**整个文件**的 data_size，
                        // 其余片段（lowest_vcn > 0）若也累加会重复计数（曾导致 size 虚高 2.4×）。
                        // lowest_vcn @ +16（Vcn = i64）。
                        let lowest_vcn = rd_u64(record, off + 16).unwrap_or(0);
                        if lowest_vcn == 0 {
                            // data_size @ +48；allocated_size @ +40（簇对齐后的实际占用）
                            if let Some(ds) = rd_u64(record, off + 48) {
                                own_size += ds;
                            }
                            if let Some(asize) = rd_u64(record, off + 40) {
                                alloc_size += asize;
                            }
                        }
                    } else {
                        let val_len = rd_u32(record, off + 16).unwrap_or(0) as u64;
                        own_size += val_len;
                    }
                }
            }
            ATTR_REPARSE_POINT => {
                is_reparse = true;
            }
            _ => {}
        }

        // 未使用 name_off，避免告警
        let _ = name_off;
        off += attr_len;
    }

    if name.is_empty() {
        return None;
    }

    Some(RawEntry { name, parent_frn, own_size, alloc_size, is_dir, is_reparse })
}

/// 从条目名提取扩展名（小写，无扩展名返回 "(none)"）。
pub fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "(none)".into())
}


#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个最小可用的 FILE 记录（1024 字节，2 扇区）。
    fn build_record(flags: u16, name: &str, size: u64) -> Vec<u8> {
        let mut r = vec![0u8; 1024];
        r[0..4].copy_from_slice(b"FILE");
        r[4..6].copy_from_slice(&0x30u16.to_le_bytes());
        r[6..8].copy_from_slice(&3u16.to_le_bytes());
        r[8..16].copy_from_slice(&0u64.to_le_bytes());
        let attrs_off: u16 = 0x38;
        r[0x14..0x16].copy_from_slice(&attrs_off.to_le_bytes());
        r[0x16..0x18].copy_from_slice(&flags.to_le_bytes());

        // USN 区
        let usn: u16 = 0xAAAA;
        r[0x30..0x32].copy_from_slice(&usn.to_le_bytes());
        r[0x32..0x34].copy_from_slice(&0x1111u16.to_le_bytes());
        r[0x34..0x36].copy_from_slice(&0x2222u16.to_le_bytes());
        r[510..512].copy_from_slice(&usn.to_le_bytes());
        r[1022..1024].copy_from_slice(&usn.to_le_bytes());

        let mut off = attrs_off as usize;

        // $FILE_NAME (resident)
        let fname_value_off: u16 = 24;
        let name_utf16: Vec<u16> = name.encode_utf16().collect();
        let val_len = 66 + name_utf16.len() * 2;
        let attr_len = (fname_value_off as usize + val_len + 7) & !7;
        r[off..off + 4].copy_from_slice(&ATTR_FILE_NAME.to_le_bytes());
        r[off + 4..off + 8].copy_from_slice(&(attr_len as u32).to_le_bytes());
        r[off + 8] = 0;
        r[off + 9] = 0;
        r[off + 16..off + 20].copy_from_slice(&(val_len as u32).to_le_bytes());
        r[off + 20..off + 22].copy_from_slice(&fname_value_off.to_le_bytes());
        let vstart = off + fname_value_off as usize;
        r[vstart..vstart + 8].copy_from_slice(&5u64.to_le_bytes());
        r[vstart + 64] = name_utf16.len() as u8;
        r[vstart + 65] = NS_WIN32;
        for (i, ch) in name_utf16.iter().enumerate() {
            let b = ch.to_le_bytes();
            r[vstart + 66 + i * 2] = b[0];
            r[vstart + 67 + i * 2] = b[1];
        }
        off += attr_len;

        // $DATA (non-resident)，仅文件
        if flags & 0x02 == 0 {
            let attr_len2 = 72usize;
            r[off..off + 4].copy_from_slice(&ATTR_DATA.to_le_bytes());
            r[off + 4..off + 8].copy_from_slice(&(attr_len2 as u32).to_le_bytes());
            r[off + 8] = 1;
            r[off + 9] = 0;
            // allocated_size @ +40（测试里按 4096 簇对齐）
            let alloc = ((size + 4095) / 4096) * 4096;
            r[off + 40..off + 48].copy_from_slice(&alloc.to_le_bytes());
            r[off + 48..off + 56].copy_from_slice(&size.to_le_bytes());
            off += attr_len2;
        }

        r[off..off + 4].copy_from_slice(&ATTR_END.to_le_bytes());
        r
    }

    #[test]
    fn parses_file_name_and_size() {
        let mut rec = build_record(0x01, "hello.txt", 1234);
        let e = parse_record(&mut rec).expect("应能解析");
        assert_eq!(e.name, "hello.txt");
        assert!(!e.is_dir);
        assert_eq!(e.own_size, 1234);
        assert_eq!(e.alloc_size, 4096, "1234 字节按 4KB 簇对齐 = 4096");
        assert_eq!(e.parent_frn, 5);
    }

    #[test]
    fn parses_directory_flag() {
        let mut rec = build_record(0x03, "MyDir", 0);
        let e = parse_record(&mut rec).expect("目录应能解析");
        assert!(e.is_dir);
        assert_eq!(e.name, "MyDir");
        assert_eq!(e.own_size, 0);
    }

    #[test]
    fn rejects_deleted_record() {
        let mut rec = build_record(0x00, "gone.txt", 10);
        assert!(parse_record(&mut rec).is_none());
    }

    #[test]
    fn rejects_non_file_magic() {
        let mut rec = build_record(0x01, "x", 1);
        rec[0] = b'X';
        assert!(parse_record(&mut rec).is_none());
    }

    #[test]
    fn unicode_name_roundtrip() {
        let mut rec = build_record(0x01, "文档.txt", 42);
        let e = parse_record(&mut rec).unwrap();
        assert_eq!(e.name, "文档.txt");
        assert_eq!(e.own_size, 42);
    }
}

