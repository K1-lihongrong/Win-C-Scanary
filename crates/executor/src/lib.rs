//! wcs-executor — 执行引擎（回收站/隔离/删除 + undo）。

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use wcs_guard::{check_path, GuardConfig, Verdict};

/// 执行动作
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Recycle,
    Quarantine,
    Delete,
    SystemCmd,
}

/// 执行计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub action: Action,
    pub paths: Vec<PathBuf>,
    pub reason: String,
    #[serde(default)]
    pub granularity: Granularity,
    #[serde(default)]
    pub system_cmd: Option<String>,
}

/// 回收粒度
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    #[default]
    File,
    Directory,
}

/// undo 记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoEntry {
    pub timestamp: String,
    pub session_id: String,
    pub action: Action,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub reason: String,
    pub bytes_freed: u64,
}

/// 执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub session_id: String,
    pub executed: bool,
    pub matched_count: usize,
    pub total_bytes: u64,
    pub entries: Vec<UndoEntry>,
    pub blocked: Vec<(PathBuf, String)>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn new_session_id() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

/// 执行计划（dry_run 时只记录不删除）
pub fn execute(
    plan: &Plan,
    guard_cfg: &GuardConfig,
    dry_run: bool,
    undo_log: &Path,
    quarantine_root: &Path,
) -> anyhow::Result<ExecResult> {
    let session_id = new_session_id();
    let mut allowed: Vec<PathBuf> = Vec::new();
    let mut blocked: Vec<(PathBuf, String)> = Vec::new();

    for p in &plan.paths {
        match check_path(p, guard_cfg) {
            r if r.verdict == Verdict::Block => {
                blocked.push((p.clone(), r.reason.unwrap_or_default()));
            }
            r if r.verdict == Verdict::Warn => {
                // 用户数据区：executor 不强制拦截（安全依赖调用方/Agent 遵守"先确认"铁律）。
                // 这里记录警告，便于排查；默认 dry-run + 预览先行是主要防线。
                tracing::warn!("用户数据区（需确认）: {}", p.display());
                allowed.push(p.clone());
            }
            _ => allowed.push(p.clone()),
        }
    }

    let mut entries: Vec<UndoEntry> = Vec::new();
    let mut total_bytes = 0u64;

    if dry_run {
        for p in &allowed {
            entries.push(UndoEntry {
                timestamp: now(),
                session_id: session_id.clone(),
                action: plan.action,
                source: p.clone(),
                destination: None,
                reason: format!("dry-run: {}", plan.reason),
                bytes_freed: 0,
            });
        }
        write_log(undo_log, &entries)?;
        return Ok(ExecResult {
            session_id,
            executed: false,
            matched_count: allowed.len(),
            total_bytes: 0,
            entries,
            blocked,
        });
    }

    // 若没有允许项，直接记录结果，不调用任何删除 API
    if allowed.is_empty() {
        write_log(undo_log, &entries)?;
        return Ok(ExecResult {
            session_id,
            executed: false,
            matched_count: 0,
            total_bytes: 0,
            entries,
            blocked,
        });
    }

    match plan.action {
        Action::Recycle => {
            // 删除前统计每项大小（删除后无法再算）
            let sizes: Vec<u64> = allowed.iter().map(|p| path_size(p)).collect();
            total_bytes = sizes.iter().sum();
            trash::delete_all(&allowed)?;
            for (p, freed) in allowed.iter().zip(sizes) {
                entries.push(UndoEntry {
                    timestamp: now(),
                    session_id: session_id.clone(),
                    action: Action::Recycle,
                    source: p.clone(),
                    destination: None,
                    reason: plan.reason.clone(),
                    bytes_freed: freed,
                });
            }
        }
        Action::Quarantine => {
            std::fs::create_dir_all(quarantine_root)?;
            for src in &allowed {
                let freed = path_size(src);
                total_bytes += freed;
                let stamp = chrono::Utc::now().timestamp_millis();
                let leaf = src.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "item".into());
                let dst = quarantine_root.join(format!("{}-{}", stamp, leaf));
                std::fs::rename(src, &dst).ok();
                entries.push(UndoEntry {
                    timestamp: now(),
                    session_id: session_id.clone(),
                    action: Action::Quarantine,
                    source: src.clone(),
                    destination: Some(dst),
                    reason: plan.reason.clone(),
                    bytes_freed: freed,
                });
            }
        }
        Action::Delete => {
            for p in &allowed {
                let freed = path_size(p);
                total_bytes += freed;
                if p.is_dir() {
                    std::fs::remove_dir_all(p).ok();
                } else if p.exists() {
                    std::fs::remove_file(p).ok();
                }
                entries.push(UndoEntry {
                    timestamp: now(),
                    session_id: session_id.clone(),
                    action: Action::Delete,
                    source: p.clone(),
                    destination: None,
                    reason: plan.reason.clone(),
                    bytes_freed: freed,
                });
            }
        }
        Action::SystemCmd => {
            // 系统级操作（WinSxS/Windows.old/休眠等）不在此执行：
            // 已改为"指导型"命令（scanary system / hiber / migrate），只输出命令由用户自行运行。
            // 保留此分支以兼容历史规则（mode = "system_cmd"），仅记录、不执行。
            if let Some(cmd) = &plan.system_cmd {
                tracing::warn!("system_cmd 不自动执行（请用 scanary system / hiber 获取命令）: {}", cmd);
            }
        }
    }

    write_log(undo_log, &entries)?;
    Ok(ExecResult {
        session_id,
        executed: true,
        matched_count: allowed.len(),
        total_bytes,
        entries,
        blocked,
    })
}

/// 计算单个路径的大小（文件=文件大小，目录=递归求和）。用于统计释放量。
fn path_size(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(md) => {
            if md.is_file() {
                md.len()
            } else if md.is_dir() {
                let mut total = 0u64;
                if let Ok(rd) = std::fs::read_dir(p) {
                    for entry in rd.flatten() {
                        total += path_size(&entry.path());
                    }
                }
                total
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

fn write_log(undo_log: &Path, entries: &[UndoEntry]) -> anyhow::Result<()> {
    if let Some(parent) = undo_log.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(undo_log)?;
    for e in entries {
        writeln!(f, "{}", serde_json::to_string(e)?)?;
    }
    Ok(())
}

/// 读取 undo 日志
/// 隔离区默认保留天数。
pub const QUARANTINE_MAX_AGE_DAYS: u64 = 7;

/// 清理隔离区中超过 `max_age_days` 的文件（按文件名前缀的时间戳判断）。
///
/// 隔离项由 `execute` 命名为 `<timestamp_ms>-<leaf>`；这里解析该时间戳，
/// 早于 `now - max_age_days` 的文件被删除。返回删除的文件数。
/// 无法解析时间戳的文件**保守跳过**（不删）。
pub fn prune_quarantine(root: &Path, max_age_days: u64) -> anyhow::Result<usize> {
    if !root.exists() {
        return Ok(0);
    }
    let cutoff_ms = (chrono::Utc::now().timestamp_millis() as u64)
        .saturating_sub(max_age_days.saturating_mul(24 * 3600 * 1000));
    let mut removed = 0usize;
    for entry in std::fs::read_dir(root)?.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // 取第一个 '-' 前的时间戳
        let ts = name.split('-').next().and_then(|s| s.parse::<u64>().ok());
        if let Some(ts) = ts {
            if ts < cutoff_ms {
                if std::fs::remove_file(&p).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

pub fn read_undo_log(path: &Path) -> anyhow::Result<Vec<UndoEntry>> {
    let mut out = Vec::new();
    if !path.exists() {
        return Ok(out);
    }
    let text = std::fs::read_to_string(path)?;
    for line in text.lines() {
        if let Ok(e) = serde_json::from_str::<UndoEntry>(line) {
            out.push(e);
        }
    }
    Ok(out)
}
