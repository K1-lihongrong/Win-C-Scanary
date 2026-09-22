//! jwalk 遍历实现（无需权限，MFT 失败时的回退）。

use crate::node::{ChildInfo, DirMetadata, ExtShare, Node, ScanMode, ScanOptions, ScanProgress, ScanStats};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

const PRUNED_DIRS: &[&str] = &["$recycle.bin", "system volume information", ".trash"];

fn is_pruned(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PRUNED_DIRS.iter().any(|p| *p == lower)
}

/// 遍历扫描（简化版：递归 read_dir）
pub fn scan<F>(root: &Path, opts: &ScanOptions, on_progress: &F) -> Result<Node>
where
    F: Fn(&ScanProgress),
{
    scan_with_stats(root, opts, on_progress).map(|(n, _)| n)
}

/// 遍历扫描 + 统计
pub fn scan_with_stats<F>(root: &Path, opts: &ScanOptions, on_progress: &F) -> Result<(Node, ScanStats)>
where
    F: Fn(&ScanProgress),
{
    let mut stats = ScanStats::default();
    stats.mode = ScanMode::Walk;
    let files_seen = std::cell::Cell::new(0u64);
    let bytes_seen = std::cell::Cell::new(0u64);
    let node = build(root, opts, 0, &files_seen, &bytes_seen);
    stats.files_seen = files_seen.get();
    stats.bytes_seen = bytes_seen.get();
    on_progress(&ScanProgress {
        files_seen: stats.files_seen,
        bytes_seen: stats.bytes_seen,
        current_path: root.to_string_lossy().to_string(),
    });
    Ok((node, stats))
}

/// 文件数上限：超过则停止深入并标记截断（防止 inspect 在大目录上静默变慢）。
pub const INSPECT_FILE_LIMIT: u64 = 500_000;

fn build(path: &Path, opts: &ScanOptions, depth: usize, files: &std::cell::Cell<u64>, bytes: &std::cell::Cell<u64>) -> Node {
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
    let mut size = 0u64;
    let mut file_count = 0u64;
    let mut children = Vec::new();
    let mut ext_map: HashMap<String, (u64, u64)> = HashMap::new();

    // 规模保护：文件数超过上限则不再深入（避免大目录上静默变慢；由 inspect 标注截断）。
    let within_budget = files.get() < INSPECT_FILE_LIMIT;
    if within_budget && depth < opts.max_depth.unwrap_or(usize::MAX) {
        if let Ok(rd) = std::fs::read_dir(path) {
            for entry in rd.flatten() {
                let p = entry.path();
                let ft = match entry.file_type() { Ok(t) => t, Err(_) => continue };
                if ft.is_dir() {
                    let cname = entry.file_name().to_string_lossy().to_string();
                    if is_pruned(&cname) { continue; }
                    let child = build(&p, opts, depth + 1, files, bytes);
                    size += child.size;
                    file_count += child.file_count;
                    children.push(child);
                } else if ft.is_file() {
                    let fsize = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    size += fsize;
                    file_count += 1;
                    files.set(files.get() + 1);
                    bytes.set(bytes.get() + fsize);
                    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "(none)".into());
                    let e = ext_map.entry(ext).or_insert((0, 0));
                    e.0 += fsize;
                    e.1 += 1;
                }
            }
        }
    }

    children.sort_by_key(|c| std::cmp::Reverse(c.size));
    let mut top_extensions: Vec<ExtShare> = ext_map
        .into_iter()
        .map(|(ext, (b, c))| ExtShare { ext, bytes: b, count: c })
        .collect();
    top_extensions.sort_by_key(|e| std::cmp::Reverse(e.bytes));
    top_extensions.truncate(8);

    Node {
        name,
        path: path.to_string_lossy().to_string(),
        is_dir: true,
        size,
        file_count,
        children,
        rule_id: None,
        top_extensions,
    }
}

/// 提取目录元数据（供 Agent 判断）
pub fn inspect(path: &Path, samples: usize) -> Result<DirMetadata> {
    let files = std::cell::Cell::new(0u64);
    let bytes = std::cell::Cell::new(0u64);
    let node = build(path, &ScanOptions::default(), 0, &files, &bytes);
    let sample_paths: Vec<String> = node.children.iter().take(samples).map(|c| c.path.clone()).collect();
    let top_children: Vec<ChildInfo> = node.children.iter().take(20).map(|c| ChildInfo { name: c.name.clone(), size: c.size, is_dir: c.is_dir }).collect();
    // 达到文件数上限 → 结果不完整，明确标注（调用方应提示"改用 scan"）
    let truncated = files.get() >= INSPECT_FILE_LIMIT;
    Ok(DirMetadata {
        path: path.to_string_lossy().to_string(),
        size_bytes: node.size,
        file_count: node.file_count,
        top_extensions: node.top_extensions,
        sample_paths,
        top_children,
        rule_hint: None,
        truncated,
    })
}
