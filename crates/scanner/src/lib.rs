//! wcs-scanner — 扫描引擎（MFT 直读 + jwalk 回退）

use std::path::Path;

#[cfg(windows)]
pub mod aligned_reader;
#[cfg(windows)]
pub mod mft;
#[cfg(windows)]
pub mod mft_record;
#[cfg(windows)]
pub mod perm;
pub mod node;
pub mod walk;

pub use node::{ChildInfo, DirMetadata, DiskSpace, ExtShare, Node, ScanMode, ScanOptions, ScanProgress, ScanStats};

/// 简单扫描（默认选项）
pub fn scan<P: AsRef<Path>>(root: P) -> anyhow::Result<Node> {
    scan_with(root, ScanOptions::default(), |_| {})
}

/// 带选项 + 进度回调
pub fn scan_with<P, F>(root: P, opts: ScanOptions, on_progress: F) -> anyhow::Result<Node>
where
    P: AsRef<Path>,
    F: Fn(&ScanProgress) + Send + Sync,
{
    walk::scan(root.as_ref(), &opts, &on_progress)
}

/// 带统计（返回树 + 阶段耗时）。优先尝试 MFT，失败降级 walk。
///
/// 不返回目录索引；需要按规则计算分级可清理量时用 `scan_with_stats_indexed`。
pub fn scan_with_stats<P, F>(
    root: P,
    opts: ScanOptions,
    on_progress: F,
) -> anyhow::Result<(Node, ScanStats)>
where
    P: AsRef<Path>,
    F: Fn(&ScanProgress) + Send + Sync,
{
    let (node, stats, _index) = scan_with_stats_indexed(root, opts, on_progress)?;
    Ok((node, stats))
}

/// 带统计 + 目录聚合索引。优先尝试 MFT，失败降级 walk。
///
/// 返回的 `DirIndex` 仅在 MFT 成功时为 `Some`（walk 模式无索引）；
/// 供 report 按规则 scope 计算分级可清理量。
pub fn scan_with_stats_indexed<P, F>(
    root: P,
    opts: ScanOptions,
    on_progress: F,
) -> anyhow::Result<(Node, ScanStats, Option<mft::DirIndex>)>
where
    P: AsRef<Path>,
    F: Fn(&ScanProgress) + Send + Sync,
{
    let root = root.as_ref();
    let mut stats = ScanStats::default();
    let total_t0 = std::time::Instant::now();

    #[cfg(windows)]
    if opts.prefer_mft {
        if let Some(letter) = drive_letter_of(root) {
            // 仅当 root 是卷根时才尝试 MFT（子目录 MFT 需额外处理，暂不支持）
            if is_drive_root(root) && can_use_mft(letter) {
                stats.mft_attempted = true;
                let subroot = None;
                let progress = &on_progress;
                let mft_t0 = std::time::Instant::now();
                // 临时静默 panic hook，避免 ntfs crate 的 panic 噪音（即使被 catch 也会打印）
                let prev_hook = std::panic::take_hook();
                std::panic::set_hook(Box::new(|_| {}));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mft::scan_volume(letter, subroot, opts.mft_max_records, |records, bytes| {
                        progress(&ScanProgress {
                            files_seen: records,
                            bytes_seen: bytes,
                            current_path: format!("MFT record {}", records),
                        });
                    })
                }))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("MFT 扫描 panic")));
                std::panic::set_hook(prev_hook);

                match result {
                    Ok((node, index)) => {
                        stats.mft_ms = mft_t0.elapsed().as_millis() as u64;
                        stats.mft_succeeded = true;
                        stats.mode = ScanMode::Mft;
                        stats.files_seen = node.file_count;
                        stats.bytes_seen = node.size;
                        stats.total_ms = total_t0.elapsed().as_millis() as u64;
                        return Ok((node, stats, Some(index)));
                    }
                    Err(e) => {
                        stats.mft_ms = mft_t0.elapsed().as_millis() as u64;
                        stats.degraded = true;
                        stats.degrade_reason = Some(format!("MFT 失败，降级 walk: {}", e));
                        tracing::warn!("{}", stats.degrade_reason.as_deref().unwrap_or(""));
                    }
                }
            } else {
                stats.degraded = true;
                stats.degrade_reason = Some("无 MFT 权限（需管理员），使用 walk".into());
            }
        }
    }

    let (node, wstats) = walk::scan_with_stats(root, &opts, &on_progress)?;
    stats.mode = wstats.mode;
    stats.walk_ms = wstats.walk_ms;
    stats.build_tree_ms = wstats.build_tree_ms;
    stats.files_seen = wstats.files_seen;
    stats.bytes_seen = wstats.bytes_seen;
    stats.total_ms = total_t0.elapsed().as_millis() as u64;
    Ok((node, stats, None))
}

#[cfg(windows)]
fn drive_letter_of(path: &Path) -> Option<char> {
    let s = path.to_string_lossy();
    let bytes: Vec<char> = s.chars().collect();
    if bytes.len() >= 2 && bytes[1] == ':' {
        let c = bytes[0];
        if c.is_ascii_alphabetic() {
            return Some(c);
        }
    }
    None
}

#[cfg(windows)]
fn is_drive_root(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let t = s.trim_end_matches(['\\', '/']);
    t.len() <= 2 && t.ends_with(':')
}

/// 权限探测：当前是否能 MFT 直读
#[cfg(windows)]
pub fn can_use_mft(volume: char) -> bool {
    perm::can_use_mft(volume)
}

/// 非 Windows 平台恒为 false。
#[cfg(not(windows))]
pub fn can_use_mft(_volume: char) -> bool {
    false
}

/// 当前是否管理员。
#[cfg(windows)]
pub fn is_admin() -> bool {
    perm::is_admin()
}

/// 卷空间信息。
#[cfg(windows)]
pub fn disk_space(volume: char) -> Option<DiskSpace> {
    perm::disk_space(volume)
}

#[cfg(not(windows))]
pub fn disk_space(_volume: char) -> Option<DiskSpace> {
    None
}

#[cfg(not(windows))]
pub fn is_admin() -> bool {
    false
}

/// 提取目录元数据（供 Agent 判断，不读文件内容）
pub fn inspect<P: AsRef<Path>>(path: P, samples: usize) -> anyhow::Result<DirMetadata> {
    walk::inspect(path.as_ref(), samples)
}

// 重新导出 serde 类型以便下游使用
pub use serde_json::Value as JsonValue;
