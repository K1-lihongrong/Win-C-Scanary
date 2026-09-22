//! wcs-guard — 安全门禁（fail-closed 路径校验）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 门禁裁决
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Warn,
    Block,
    Skip,
}

/// 门禁结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardResult {
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub matched_rule: Option<String>,
}

/// 门禁配置
#[derive(Debug, Clone)]
pub struct GuardConfig {
    pub allowed_roots: Vec<PathBuf>,
    pub protect_c_drive_boundary: bool,
    pub reject_reparse_points: bool,
    pub extra_protected: Vec<String>,
}

impl Default for GuardConfig {
    fn default() -> Self {
        Self {
            allowed_roots: vec![PathBuf::from("C:\\")],
            protect_c_drive_boundary: true,
            reject_reparse_points: true,
            extra_protected: Vec::new(),
        }
    }
}

/// 永远拒绝的路径片段（系统核心 + 开发工具 + 版本控制）
const FORBIDDEN_PATTERNS: &[&str] = &[
    "\\windows\\system32",
    "\\windows\\syswow64",
    "\\program files",
    "\\program files (x86)",
    "\\$recycle.bin",
    "\\system volume information",
    ".vscode",
    ".vscode-server",
    "\\jetbrains\\",
    "\\code\\user",
    "\\.git\\",
];

/// 用户数据区（Warn）
const USER_DATA_PATTERNS: &[&str] = &[
    "\\documents\\",
    "\\pictures\\",
    "\\videos\\",
    "\\desktop\\",
    "\\music\\",
];

fn block(reason: &str) -> GuardResult {
    GuardResult { verdict: Verdict::Block, reason: Some(reason.into()), matched_rule: None }
}

fn warn(reason: &str) -> GuardResult {
    GuardResult { verdict: Verdict::Warn, reason: Some(reason.into()), matched_rule: None }
}

fn pass() -> GuardResult {
    GuardResult { verdict: Verdict::Pass, reason: None, matched_rule: None }
}

/// 校验单个路径（核心）
pub fn check_path(path: &Path, cfg: &GuardConfig) -> GuardResult {
    if path.is_relative() {
        return block("相对路径不被允许");
    }
    if is_forbidden_root(path) {
        return block("命中受保护根");
    }
    let lower = path.to_string_lossy().to_lowercase();
    for pat in FORBIDDEN_PATTERNS {
        if lower.contains(pat) {
            return block("命中保护路径");
        }
    }
    for pat in &cfg.extra_protected {
        if lower.contains(&pat.to_lowercase()) {
            return block("命中额外保护路径");
        }
    }
    if !within_allowed_roots(path, cfg) {
        return block("超出允许操作范围");
    }
    if cfg.reject_reparse_points && has_reparse_ancestor(path) {
        return block("祖先含 junction/符号链接");
    }
    for pat in USER_DATA_PATTERNS {
        if lower.contains(pat) {
            return warn("位于用户数据区，需确认");
        }
    }
    pass()
}

/// 批量校验
pub fn check_paths(paths: &[PathBuf], cfg: &GuardConfig) -> Vec<GuardResult> {
    paths.iter().map(|p| check_path(p, cfg)).collect()
}

/// 是否为危险根（盘符根/系统根/用户根）
pub fn is_forbidden_root(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    let trimmed = lower.trim_end_matches('\\');
    if trimmed == "c:" || trimmed == "c:\\" {
        return true;
    }
    let forbidden = ["c:\\windows", "c:\\users", "c:\\windows\\system32"];
    for f in forbidden {
        if trimmed == f.trim_end_matches('\\') {
            return true;
        }
    }
    if let Some(home) = dirs::home_dir() {
        let h = home.to_string_lossy().to_lowercase();
        if trimmed == h.trim_end_matches('\\') {
            return true;
        }
    }
    false
}

/// 路径祖先是否含重解析点
pub fn has_reparse_ancestor(path: &Path) -> bool {
    let mut cur = path.parent();
    while let Some(p) = cur {
        if let Ok(md) = std::fs::symlink_metadata(p) {
            if md.file_type().is_symlink() {
                return true;
            }
        }
        cur = p.parent();
    }
    false
}

fn within_allowed_roots(path: &Path, cfg: &GuardConfig) -> bool {
    if cfg.allowed_roots.is_empty() {
        return true;
    }
    cfg.allowed_roots.iter().any(|root| path.starts_with(root))
}
