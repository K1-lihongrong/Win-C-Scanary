//! wcs-dedup — 重复文件检测（三阶段哈希，只检测不删除）。
//!
//! 阶段：
//! 1. **size 分组**：按文件大小分组，只保留组内 ≥2 个文件的大小。
//! 2. **head-hash 分组**：对候选文件读前 HEAD_BYTES 字节做哈希，进一步缩小候选。
//! 3. **full-hash 确认**：对仍存活的候选读全文做 SHA-256，哈希相同即为真重复。
//!
//! 只读、不删除；删除决策交由 Agent / 用户。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// head-hash 读取的字节数（8 KiB）。
const HEAD_BYTES: u64 = 8 * 1024;

/// 一组重复文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DupGroup {
    /// 文件大小
    pub size: u64,
    /// 相同的完整哈希（十六进制）
    pub hash: String,
    /// 该组内的文件路径
    pub paths: Vec<PathBuf>,
    /// 可回收字节 = size × (paths.len() - 1)
    pub waste_bytes: u64,
}

/// 重复文件检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupResult {
    pub groups: Vec<DupGroup>,
    pub total_waste_bytes: u64,
    pub files_scanned: u64,
}

/// 读取文件前 n 字节（不足 n 则读全部），返回实际读到的字节。
fn read_head(path: &Path, n: u64) -> std::io::Result<Vec<u8>> {
    let f = File::open(path)?;
    let mut buf = Vec::new();
    f.take(n).read_to_end(&mut buf)?;
    Ok(buf)
}

/// 计算文件的完整 SHA-256（流式读取，避免一次性载入大文件）。
fn full_hash(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 计算字节切片的 SHA-256（用于 head-hash）。
fn bytes_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// 查找重复文件。
///
/// - `root`：遍历起点。
/// - `min_size`：忽略小于该字节数的文件（默认 0）。
///
/// 返回按浪费空间降序排列的重复组。
pub fn find_duplicates(root: &Path, min_size: u64) -> anyhow::Result<DedupResult> {
    // ---------- 阶段 0：遍历 + 按 size 分组 ----------
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut files_scanned: u64 = 0;

    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let size = match entry.metadata() {
            Ok(m) => m.len(),
            Err(_) => continue,
        };
        if size < min_size || size == 0 {
            continue;
        }
        files_scanned += 1;
        by_size.entry(size).or_default().push(entry.path().to_path_buf());
    }

    // 只保留组内 ≥2 个文件的大小
    let candidates: Vec<(u64, Vec<PathBuf>)> = by_size
        .into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .collect();

    // ---------- 阶段 1：head-hash 分组 ----------
    // key = (size, head_hash)，把 size 相同且头部相同的文件聚在一起
    let mut by_head: HashMap<(u64, String), Vec<PathBuf>> = HashMap::new();
    for (size, paths) in candidates {
        for p in paths {
            let head = match read_head(&p, HEAD_BYTES) {
                Ok(h) => h,
                Err(_) => continue, // 读取失败（权限/占用）跳过
            };
            // 小文件（size <= HEAD_BYTES）时 head 即全文，可直接用 head 作 key；
            // 大文件则用 head 的哈希。
            let key = if size <= HEAD_BYTES {
                bytes_hash(&head)
            } else {
                bytes_hash(&head)
            };
            by_head.entry((size, key)).or_default().push(p);
        }
    }

    // ---------- 阶段 2：full-hash 确认 ----------
    let mut groups: Vec<DupGroup> = Vec::new();
    for ((size, _head), paths) in by_head {
        if paths.len() < 2 {
            continue;
        }
        // 对候选按 full-hash 再分组
        let mut by_full: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for p in paths {
            match full_hash(&p) {
                Ok(h) => by_full.entry(h).or_default().push(p),
                Err(_) => continue,
            }
        }
        for (hash, mut plist) in by_full {
            if plist.len() < 2 {
                continue;
            }
            plist.sort();
            let waste = size * (plist.len() as u64 - 1);
            groups.push(DupGroup { size, hash, paths: plist, waste_bytes: waste });
        }
    }

    // 按浪费空间降序
    groups.sort_by_key(|g| std::cmp::Reverse(g.waste_bytes));
    let total_waste_bytes = groups.iter().map(|g| g.waste_bytes).sum();

    Ok(DedupResult { groups, total_waste_bytes, files_scanned })
}
