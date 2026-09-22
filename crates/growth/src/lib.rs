//! wcs-growth — 增长追踪（快照采集 / 存储 / 对比）。
//!
//! 核心用途：回答"C 盘为什么越来越满"——对比历史快照与当前状态，
//! 找出体积增长最多的目录。
//!
//! 快照策略：
//! - 只保留 size >= 阈值（默认 1MB）的目录（全盘 ~1 万条，序列化 ~1.2MB）
//! - 存于 `%APPDATA%\Win-C-Scanary\snapshots\`，保留最近 N 份（默认 10）
//! - 文件名含时间戳，便于识别

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wcs_scanner::mft::DirIndex;

/// 快照中单个目录的条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub path: String,
    pub size: u64,
    pub alloc: u64,
}

/// 一份快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// 采集时间（RFC3339）
    pub taken_at: String,
    /// 卷/根标识（如 "C:"）
    pub volume: String,
    /// 目录条目（仅 size >= 阈值）
    pub entries: Vec<SnapshotEntry>,
}

/// 单个目录的增长
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowthEntry {
    pub path: String,
    /// 旧快照中的大小
    pub old_size: u64,
    /// 新快照中的大小
    pub new_size: u64,
    /// 增量（new - old，可负）
    pub delta: i64,
    /// 本层新增增长 = 本目录 delta - 父目录 delta。
    /// 大值表示"增长从这一层开始堆积"，用于定位真正在变大的具体目录（避免被父链稀释）。
    pub self_delta: i64,
}

/// 对比结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowthReport {
    pub from_time: String,
    pub to_time: String,
    pub total_old: u64,
    pub total_new: u64,
    /// 总增量（可负）
    pub total_delta: i64,
    /// 增长最多的目录（delta 降序）
    pub top_growth: Vec<GrowthEntry>,
    /// 缩小最多的目录（delta 升序）
    pub top_shrink: Vec<GrowthEntry>,
}

/// 默认大小阈值：1 MB（低于此的目录不纳入快照）
pub const DEFAULT_MIN_SIZE: u64 = 1_000_000;

/// 默认保留快照份数
pub const DEFAULT_KEEP: usize = 10;

/// 快照存储目录：`%APPDATA%\Win-C-Scanary\snapshots`（非 Windows 用配置目录兜底）。
pub fn snapshot_dir() -> PathBuf {
    if let Some(base) = dirs::config_dir() {
        base.join("Win-C-Scanary").join("snapshots")
    } else {
        PathBuf::from(".scanary-snapshots")
    }
}

/// 从扫描得到的 DirIndex 采集快照（只保留 size >= min_size 的目录）。
pub fn take_snapshot(idx: &DirIndex, volume: &str, min_size: u64) -> Snapshot {
    let entries: Vec<SnapshotEntry> = idx
        .all_dirs_alloc()
        .iter()
        .filter(|(_, s, _)| *s >= min_size)
        .map(|(p, s, a)| SnapshotEntry {
            path: p.clone(),
            size: *s,
            alloc: *a,
        })
        .collect();
    Snapshot {
        taken_at: chrono::Utc::now().to_rfc3339(),
        volume: volume.to_string(),
        entries,
    }
}

/// 保存快照到目录，并清理超出 keep 份的旧快照。返回保存的文件路径。
pub fn save_snapshot(snap: &Snapshot, dir: &Path, keep: usize) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).context("创建快照目录失败")?;
    // 文件名：时间戳（去掉冒号，Windows 文件名不允许）
    let ts = snap.taken_at.replace([':', '.'], "-");
    let path = dir.join(format!("snap-{}.json", ts));
    let json = serde_json::to_string(snap).context("序列化快照失败")?;
    std::fs::write(&path, json).context("写快照文件失败")?;
    prune_old(dir, keep)?;
    Ok(path)
}

/// 列出所有快照文件，按文件名（即时间）升序。
pub fn list_snapshots(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .context("读取快照目录失败")?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    files.sort();
    Ok(files)
}

/// 加载指定快照文件。
pub fn load_snapshot(path: &Path) -> Result<Snapshot> {
    let text = std::fs::read_to_string(path).context("读快照文件失败")?;
    serde_json::from_str(&text).context("解析快照失败")
}

/// 清空所有快照（新机器/重置场景）。返回删除的文件数。
pub fn clear_snapshots(dir: &Path) -> Result<usize> {
    let files = list_snapshots(dir)?;
    let n = files.len();
    for f in &files {
        let _ = std::fs::remove_file(f);
    }
    Ok(n)
}

/// 删除超出 keep 份的最旧快照。
fn prune_old(dir: &Path, keep: usize) -> Result<()> {
    let files = list_snapshots(dir)?;
    if files.len() > keep {
        for f in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(f);
        }
    }
    Ok(())
}

/// 对比两份快照，返回增长报告。top_n 控制增长/缩小列表长度。
pub fn diff(old: &Snapshot, new: &Snapshot, top_n: usize) -> GrowthReport {
    use std::collections::HashMap;
    let old_map: HashMap<&str, u64> = old.entries.iter().map(|e| (e.path.as_str(), e.size)).collect();

    // 第一遍：算每个目录的 delta（含新增与删除）
    let mut growth: Vec<GrowthEntry> = Vec::new();
    let mut delta_of: HashMap<String, i64> = HashMap::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for e in &new.entries {
        let old_size = old_map.get(e.path.as_str()).copied().unwrap_or(0);
        let delta = e.size as i64 - old_size as i64;
        if delta != 0 {
            growth.push(GrowthEntry {
                path: e.path.clone(),
                old_size,
                new_size: e.size,
                delta,
                self_delta: 0, // 第二遍填
            });
        }
        delta_of.insert(e.path.clone(), delta);
        seen.insert(e.path.as_str());
    }
    // 旧快照有、新快照没有的目录：视为缩小到 0（被删除）
    for e in &old.entries {
        if !seen.contains(e.path.as_str()) && e.size > 0 {
            let delta = -(e.size as i64);
            growth.push(GrowthEntry {
                path: e.path.clone(),
                old_size: e.size,
                new_size: 0,
                delta,
                self_delta: 0,
            });
            delta_of.insert(e.path.clone(), delta);
        }
    }

    // 第二遍：算 self_delta = 本层新增的正增长。
    //
    // 语义：本目录相对"父层整体增长"的额外增量，且不为负（负的是缩小，归 top_shrink）。
    // 关键修正：父 delta 只取其**正值**部分参与相减。
    // 原因：两次快照间系统在变，父子可能不自洽（父缩小、子反增），
    // 此时旧的 `delta - parent_delta` 会把父的负 delta 当成"加成"，
    // 算出"本层 +1464MB / 累计 +464MB"这类虚高误导值（本层新增 > 本目录总增长）。
    // 用 `max(parent_delta, 0)` 后，父缩小不再被算作本层增长，self_delta ≤ delta 恒成立。
    for g in &mut growth {
        let parent_delta = g
            .path
            .rfind('/')
            .map(|i| &g.path[..i])
            .and_then(|parent| delta_of.get(parent).copied())
            .unwrap_or(0);
        g.self_delta = (g.delta - parent_delta.max(0)).max(0);
    }

    // 过滤阈值：本层变化小于 1MB 的忽略（避免取整噪音）
    const MIN_SELF_DELTA: i64 = 1_000_000;

    // 增长列表按 self_delta 排序（定位"增长从哪开始"）；缩小同理
    let mut top_growth: Vec<GrowthEntry> = growth
        .iter()
        .filter(|g| g.self_delta >= MIN_SELF_DELTA)
        .cloned()
        .collect();
    top_growth.sort_by(|a, b| b.self_delta.cmp(&a.self_delta));
    top_growth.truncate(top_n);

    // 缩小列表用原始 delta 排（不套 self_delta）：
    // self_delta 在"子目录增长超过父目录"时会算出负值（两次快照间系统变化导致父子不自洽），
    // 会产生"本层 -3MB 但累计 +0MB"的误导项。用户看缩小，只想直接知道"哪些目录变小了"。
    let mut top_shrink: Vec<GrowthEntry> = growth
        .iter()
        .filter(|g| g.delta <= -MIN_SELF_DELTA)
        .cloned()
        .collect();
    top_shrink.sort_by(|a, b| a.delta.cmp(&b.delta));
    top_shrink.truncate(top_n);

    let total_old: u64 = old.entries.iter().map(|e| e.size).sum();
    let total_new: u64 = new.entries.iter().map(|e| e.size).sum();

    GrowthReport {
        from_time: old.taken_at.clone(),
        to_time: new.taken_at.clone(),
        total_old,
        total_new,
        total_delta: total_new as i64 - total_old as i64,
        top_growth,
        top_shrink,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(time: &str, entries: &[(&str, u64)]) -> Snapshot {
        Snapshot {
            taken_at: time.into(),
            volume: "C:".into(),
            entries: entries
                .iter()
                .map(|(p, s)| SnapshotEntry { path: p.to_string(), size: *s, alloc: *s })
                .collect(),
        }
    }

    #[test]
    fn diff_finds_growth() {
        let old = snap("t1", &[("c:/a", 10_000_000), ("c:/b", 20_000_000)]);
        let new = snap("t2", &[("c:/a", 15_000_000), ("c:/b", 20_000_000)]);
        let rep = diff(&old, &new, 10);
        assert_eq!(rep.top_growth.len(), 1);
        assert_eq!(rep.top_growth[0].path, "c:/a");
        assert_eq!(rep.top_growth[0].delta, 5_000_000);
        assert_eq!(rep.top_growth[0].self_delta, 5_000_000);
        assert_eq!(rep.total_delta, 5_000_000);
    }

    #[test]
    fn diff_finds_shrink_and_deletion() {
        let old = snap("t1", &[("c:/a", 10_000_000), ("c:/gone", 8_000_000)]);
        let new = snap("t2", &[("c:/a", 6_000_000)]);
        let rep = diff(&old, &new, 10);
        // a 缩小 40，gone 删除 80 → 都在 shrink
        assert_eq!(rep.top_shrink.len(), 2);
        // 最大缩小是 gone（-80）
        assert_eq!(rep.top_shrink[0].path, "c:/gone");
        assert_eq!(rep.top_shrink[0].delta, -8_000_000);
    }

    #[test]
    fn diff_self_delta_excludes_parent_chain() {
        // 父 c: 与子 c:/a 都"增长"（因 a 变大传导），但真正增长在 c:/a/b
        let old = snap("t1", &[("c:", 1_000_000_000), ("c:/a", 500_000_000), ("c:/a/b", 100_000_000)]);
        let new = snap("t2", &[("c:", 1_003_000_000), ("c:/a", 503_000_000), ("c:/a/b", 103_000_000)]);
        let rep = diff(&old, &new, 10);
        // c: delta 300, a delta 300, a/b delta 300
        // self: c:=300, a=0, a/b=0（都传导自 a/b）
        // 所以 self_delta > 0 的只有 c:（顶层，无父）
        // 注意：这里三者 delta 相同，self 只在最浅层非零
        assert_eq!(rep.top_growth[0].path, "c:");
        assert_eq!(rep.top_growth[0].self_delta, 3_000_000);
    }

    #[test]
    fn diff_new_dir_counts_as_growth() {
        let old = snap("t1", &[("c:/a", 100_000_000)]);
        let new = snap("t2", &[("c:/a", 100_000_000), ("c:/new", 500_000_000)]);
        let rep = diff(&old, &new, 10);
        assert_eq!(rep.top_growth[0].path, "c:/new");
        assert_eq!(rep.top_growth[0].old_size, 0);
        assert_eq!(rep.top_growth[0].delta, 500_000_000);
        assert_eq!(rep.top_growth[0].self_delta, 500_000_000);
    }

    #[test]
    fn self_delta_not_inflated_by_shrinking_parent() {
        // 回归：父目录缩小、子目录增长时，旧公式会算出 self_delta > delta（虚高误导）。
        // c:/temp 缩小 1GB，其子 c:/temp/sub 增长 464MB。
        let old = snap("t1", &[("c:/temp", 2_000_000_000), ("c:/temp/sub", 0)]);
        let new = snap("t2", &[("c:/temp", 1_000_000_000), ("c:/temp/sub", 464_000_000)]);
        let rep = diff(&old, &new, 10);
        let sub = rep
            .top_growth
            .iter()
            .find(|g| g.path == "c:/temp/sub")
            .expect("sub 应在增长列表");
        // 旧公式：464 - (-1000) = 1464MB（虚高）；修正后应等于本目录实际增长 464MB
        assert_eq!(sub.delta, 464_000_000);
        assert_eq!(sub.self_delta, 464_000_000);
        assert!(sub.self_delta <= sub.delta);
    }

    #[test]
    fn self_delta_never_exceeds_delta() {
        // 恒等约束：任意场景下 self_delta ≤ delta（本层新增不可能超过总增长）
        let old = snap("t1", &[("c:", 5_000_000_000), ("c:/a", 3_000_000_000), ("c:/a/b", 0)]);
        let new = snap("t2", &[("c:", 4_000_000_000), ("c:/a", 3_500_000_000), ("c:/a/b", 800_000_000)]);
        let rep = diff(&old, &new, 10);
        for g in &rep.top_growth {
            assert!(
                g.self_delta <= g.delta,
                "{} 的 self_delta({}) 不应超过 delta({})",
                g.path,
                g.self_delta,
                g.delta
            );
        }
    }
}
