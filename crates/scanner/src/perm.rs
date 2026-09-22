//! Windows 权限探测：管理员判定 + 卷设备可访问性。

use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// 当前进程是否以管理员（提升）权限运行。
pub fn is_admin() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;

/// 尝试打开卷设备（\\.\C:），成功说明可 MFT 直读。
pub fn can_open_volume(volume_letter: char) -> bool {
    let path = format!(r"\\.\{}:", volume_letter.to_ascii_uppercase());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .access_mode(GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path);
    match file {
        Ok(f) => {
            let _ = f.as_raw_handle();
            true
        }
        Err(_) => false,
    }
}

/// 综合判定：是否可对该卷做 MFT 直读。
pub fn can_use_mft(volume_letter: char) -> bool {
    if !is_admin() {
        return false;
    }
    can_open_volume(volume_letter)
}

use crate::node::DiskSpace;

/// 查询某卷的可用空间（用 GetDiskFreeSpaceExW）。
pub fn disk_space(volume_letter: char) -> Option<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;
    let root = format!("{}:\\", volume_letter.to_ascii_uppercase());
    let wide: Vec<u16> = std::ffi::OsStr::new(&root)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_avail: u64 = 0;
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;

    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_avail,
            &mut total,
            &mut total_free,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(DiskSpace {
        total_bytes: total,
        free_bytes: total_free,
        used_bytes: total.saturating_sub(total_free),
    })
}
