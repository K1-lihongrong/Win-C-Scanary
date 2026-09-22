//! Node 及扫描相关类型定义。

use serde::{Deserialize, Serialize};

/// 目录树节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub file_count: u64,
    pub children: Vec<Node>,
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub top_extensions: Vec<ExtShare>,
}

/// 扩展名占比
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtShare {
    pub ext: String,
    pub bytes: u64,
    pub count: u64,
}

/// 子目录信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildInfo {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// 目录元数据（供 Agent 判断）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirMetadata {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub top_extensions: Vec<ExtShare>,
    pub sample_paths: Vec<String>,
    pub top_children: Vec<ChildInfo>,
    pub rule_hint: Option<String>,
    /// 是否因遍历规模上限被截断（true 表示结果不完整，应改用 scan）
    #[serde(default)]
    pub truncated: bool,
}

/// 扫描选项
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub follow_symlinks: bool,
    pub max_depth: Option<usize>,
    pub keep_files_per_dir: Option<usize>,
    pub prefer_mft: bool,
    /// MFT 扫描的最大记录数（None = 全量）。全量较慢（逐条解析），可限制以加速。
    pub mft_max_records: Option<u64>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            max_depth: None,
            keep_files_per_dir: Some(500),
            prefer_mft: true,
            mft_max_records: None,
        }
    }
}

/// 扫描模式
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    Mft,
    Walk,
}

impl Default for ScanMode {
    fn default() -> Self {
        ScanMode::Walk
    }
}

/// 扫描统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanStats {
    pub mode: ScanMode,
    pub mft_attempted: bool,
    pub mft_succeeded: bool,
    pub mft_ms: u64,
    pub walk_ms: u64,
    pub build_tree_ms: u64,
    pub total_ms: u64,
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub degraded: bool,
    pub degrade_reason: Option<String>,
}

/// 卷空间信息（字节）
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskSpace {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

/// 扫描进度
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub current_path: String,
}
