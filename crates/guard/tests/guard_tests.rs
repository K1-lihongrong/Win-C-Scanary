//! wcs-guard 单元测试。
//!
//! 重点覆盖 fail-closed 的各种危险路径，以及边界情况。

use std::path::{Path, PathBuf};
use wcs_guard::{check_path, is_forbidden_root, GuardConfig, Verdict};

fn cfg() -> GuardConfig {
    GuardConfig {
        allowed_roots: vec![PathBuf::from("C:\\")],
        protect_c_drive_boundary: true,
        reject_reparse_points: true,
        extra_protected: Vec::new(),
    }
}

fn blocked(p: &str) -> bool {
    check_path(Path::new(p), &cfg()).verdict == Verdict::Block
}

fn warned(p: &str) -> bool {
    check_path(Path::new(p), &cfg()).verdict == Verdict::Warn
}

fn passed(p: &str) -> bool {
    check_path(Path::new(p), &cfg()).verdict == Verdict::Pass
}

#[test]
fn relative_path_is_blocked() {
    assert!(blocked("foo/bar"));
    assert!(blocked("..\\windows"));
}

#[test]
fn drive_root_is_blocked() {
    assert!(is_forbidden_root(Path::new("C:\\")));
    assert!(blocked("C:\\"));
}

#[test]
fn system_roots_are_blocked() {
    assert!(blocked("C:\\Windows"));
    assert!(blocked("C:\\Users"));
    assert!(blocked("C:\\Windows\\System32"));
}

#[test]
fn protected_patterns_are_blocked() {
    assert!(blocked("C:\\Windows\\System32\\kernel32.dll"));
    assert!(blocked("C:\\Windows\\SysWOW64\\foo"));
    assert!(blocked("C:\\Program Files\\app\\data"));
    assert!(blocked("C:\\Program Files (x86)\\app"));
    assert!(blocked("C:\\$Recycle.Bin\\S-1-5-21\\foo"));
    assert!(blocked("C:\\System Volume Information\\tracking"));
}

#[test]
fn dev_tool_paths_are_blocked() {
    assert!(blocked("C:\\Users\\me\\.vscode\\extensions\\foo"));
    assert!(blocked("C:\\Users\\me\\.vscode-server\\data"));
    assert!(blocked("C:\\Users\\me\\AppData\\Roaming\\JetBrains\\IDEA"));
    assert!(blocked("C:\\Users\\me\\AppData\\Roaming\\Code\\User\\settings.json"));
}

#[test]
fn git_dirs_are_blocked() {
    assert!(blocked("C:\\Users\\me\\project\\.git\\config"));
}

#[test]
fn case_insensitive_blocking() {
    assert!(blocked("c:\\windows\\system32\\foo"));
    assert!(blocked("C:\\WINDOWS\\SYSTEM32\\foo"));
    assert!(blocked("C:\\PROGRAM FILES\\app"));
}

#[test]
fn user_data_is_warned_not_blocked() {
    // 用户数据区应为 Warn（不是 Block）
    assert!(warned("C:\\Users\\me\\Documents\\file.txt"));
    assert!(warned("C:\\Users\\me\\Pictures\\photo.jpg"));
    assert!(warned("C:\\Users\\me\\Desktop\\foo"));
    assert!(warned("C:\\Users\\me\\Videos\\clip.mp4"));
    assert!(warned("C:\\Users\\me\\Music\\song.mp3"));
}

#[test]
fn out_of_allowed_roots_is_blocked() {
    // D 盘不在 allowed_roots（只有 C:\）
    assert!(blocked("D:\\some\\path"));
    assert!(blocked("E:\\other"));
}

#[test]
fn normal_cache_path_is_passed() {
    // 正常的临时缓存路径应放行
    assert!(passed("C:\\Users\\me\\AppData\\Local\\Temp\\foo.tmp"));
    assert!(passed("C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache\\data_0"));
    assert!(passed("C:\\Users\\me\\AppData\\Roaming\\npm-cache\\_cacache\\index"));
}

#[test]
fn extra_protected_patterns_work() {
    let mut c = cfg();
    c.extra_protected = vec!["\\my\\secret\\".into()];
    let r = check_path(Path::new("C:\\my\\secret\\data"), &c);
    assert_eq!(r.verdict, Verdict::Block);
    assert!(r.reason.unwrap().contains("额外保护"));
}

#[test]
fn empty_allowed_roots_allows_all_non_forbidden() {
    let c = GuardConfig {
        allowed_roots: Vec::new(),
        protect_c_drive_boundary: false,
        reject_reparse_points: false,
        extra_protected: Vec::new(),
    };
    // 空 allowed_roots = 不限制盘符，但受保护路径仍 Block
    assert_eq!(check_path(Path::new("D:\\data"), &c).verdict, Verdict::Pass);
    assert_eq!(check_path(Path::new("C:\\Windows\\System32"), &c).verdict, Verdict::Block);
}

#[test]
fn root_paths_are_case_insensitive() {
    assert!(is_forbidden_root(Path::new("c:\\")));
    assert!(is_forbidden_root(Path::new("C:\\WINDOWS")));
    assert!(is_forbidden_root(Path::new("c:\\users")));
}

#[test]
fn normal_dir_not_forbidden_root() {
    assert!(!is_forbidden_root(Path::new("C:\\Windows\\Temp")));
    assert!(!is_forbidden_root(Path::new("C:\\Users\\me\\AppData")));
}
