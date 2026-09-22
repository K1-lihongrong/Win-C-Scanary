//! wcs-inventory — 已安装软件清单（读 Windows 注册表）。
//!
//! 读取三个卸载视图：
//! - HKLM \ SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall（64 位）
//! - HKLM \ SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall（32 位）
//! - HKCU \ SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall（per-user）
//!
//! 只读；跳过无 DisplayName 的项；按 (name, version) 去重。

use serde::{Deserialize, Serialize};

/// 一条已安装软件记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppEntry {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
    #[serde(default)]
    pub estimated_size_mb: Option<u64>,
    /// 是否 per-user 安装（HKCU）
    #[serde(default)]
    pub per_user: bool,
}

// ================= Windows 实现 =================

#[cfg(windows)]
mod imp {
    use super::AppEntry;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_SZ, REG_EXPAND_SZ,
    };

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 把一个注册表字符串值读到 Option<String>。
    fn read_string(root: HKEY, subkey: &str, value: &str, flags: u32) -> Option<String> {
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let sub_w = to_wide(subkey);
            if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_READ | flags, &mut hkey) != ERROR_SUCCESS
            {
                return None;
            }
            let val_w = to_wide(value);
            let mut ty: u32 = 0;
            let mut len: u32 = 0;
            // 第一次查询拿长度
            let rc = RegQueryValueExW(
                hkey,
                val_w.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                std::ptr::null_mut(),
                &mut len,
            );
            if rc != ERROR_SUCCESS || (ty != REG_SZ && ty != REG_EXPAND_SZ) || len == 0 {
                RegCloseKey(hkey);
                return None;
            }
            let mut buf = vec![0u8; len as usize];
            let rc2 = RegQueryValueExW(
                hkey,
                val_w.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                buf.as_mut_ptr(),
                &mut len,
            );
            RegCloseKey(hkey);
            if rc2 != ERROR_SUCCESS {
                return None;
            }
            // buf 是 UTF-16LE，去掉结尾 NUL
            let u16s: Vec<u16> = buf
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect();
            let s = OsString::from_wide(&u16s).to_string_lossy().into_owned();
            if s.trim().is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }

    /// 读 DWORD 值（用于 EstimatedSize，单位 KB）。
    fn read_dword(root: HKEY, subkey: &str, value: &str, flags: u32) -> Option<u32> {
        use windows_sys::Win32::System::Registry::{RegQueryValueExW, REG_DWORD};
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let sub_w = to_wide(subkey);
            if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_READ | flags, &mut hkey) != ERROR_SUCCESS
            {
                return None;
            }
            let val_w = to_wide(value);
            let mut ty: u32 = 0;
            let mut data: u32 = 0;
            let mut len: u32 = std::mem::size_of::<u32>() as u32;
            let rc = RegQueryValueExW(
                hkey,
                val_w.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                &mut data as *mut u32 as *mut u8,
                &mut len,
            );
            RegCloseKey(hkey);
            if rc == ERROR_SUCCESS && ty == REG_DWORD {
                Some(data)
            } else {
                None
            }
        }
    }

    /// 枚举一个卸载视图下的所有子键，产出 AppEntry。
    fn enum_view(root: HKEY, base: &str, flags: u32, per_user: bool) -> Vec<AppEntry> {
        let mut out = Vec::new();
        unsafe {
            let mut hkey: HKEY = std::ptr::null_mut();
            let base_w = to_wide(base);
            if RegOpenKeyExW(root, base_w.as_ptr(), 0, KEY_READ | flags, &mut hkey) != ERROR_SUCCESS
            {
                return out;
            }
            let mut idx: u32 = 0;
            loop {
                let mut name_buf = vec![0u16; 512];
                let mut name_len: u32 = name_buf.len() as u32;
                let rc = RegEnumKeyExW(
                    hkey,
                    idx,
                    name_buf.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if rc == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if rc != ERROR_SUCCESS {
                    idx += 1;
                    continue;
                }
                let sub: String = OsString::from_wide(&name_buf[..name_len as usize])
                    .to_string_lossy()
                    .into_owned();
                idx += 1;

                let mut full = String::from(base);
                full.push('\\');
                full.push_str(&sub);
                let name = match read_string(root, &full, "DisplayName", flags) {
                    Some(n) => n,
                    None => continue, // 无 DisplayName 的项跳过
                };
                let version = read_string(root, &full, "DisplayVersion", flags);
                let publisher = read_string(root, &full, "Publisher", flags);
                let install_location = read_string(root, &full, "InstallLocation", flags);
                let estimated_size_mb = read_dword(root, &full, "EstimatedSize", flags)
                    .map(|kb| (kb as u64) / 1024);

                out.push(AppEntry {
                    name,
                    version,
                    publisher,
                    install_location,
                    estimated_size_mb,
                    per_user,
                });
            }
            RegCloseKey(hkey);
        }
        out
    }

    pub fn list_apps_impl() -> anyhow::Result<Vec<AppEntry>> {
        let mut all = Vec::new();
        // 64 位视图（HKLM）
        all.extend(enum_view(HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_64KEY, false));
        // 32 位视图（HKLM Wow6432Node）
        all.extend(enum_view(HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_32KEY, false));
        // per-user（HKCU）
        all.extend(enum_view(HKEY_CURRENT_USER, UNINSTALL, 0, true));

        // 按 (name, version) 去重
        all.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.version.cmp(&b.version))
        });
        all.dedup_by(|a, b| {
            a.name.eq_ignore_ascii_case(&b.name) && a.version == b.version
        });
        Ok(all)
    }
}

#[cfg(windows)]
pub use imp::list_apps_impl as list_apps;

/// 非 Windows 平台：返回空列表（保持 API 可用）。
#[cfg(not(windows))]
pub fn list_apps() -> anyhow::Result<Vec<AppEntry>> {
    Ok(Vec::new())
}
