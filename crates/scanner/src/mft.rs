//! Windows NTFS MFT 直读。
//!
//! 直接打开卷设备，解析 NTFS 引导扇区，遍历 $MFT 的每条记录。
//! 每条记录给出 parent FRN + 文件名 + 大小，一遍拿到全盘结构。
//! 需管理员权限；非 NTFS 或权限不足时返回 Err，由调用方降级到 walk。
//!
//! 性能：顺序读整个 $MFT 的 $DATA 流（一次性 attach），手工解析 FILE 记录，
//! 避免逐条 ntfs.file() 反复重读 $MFT 元数据（T-MFT-2）。

use crate::node::{ExtShare, Node};
use anyhow::{anyhow, Context, Result};
use ntfs::{KnownNtfsFileRecordNumber, Ntfs};
use std::collections::HashMap;
use std::fs::OpenOptions;

use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
const GENERIC_READ: u32 = 0x8000_0000;

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    parent_frn: u64,
    own_size: u64,
    /// 实际分配大小（簇对齐），用于空间核算
    alloc_size: u64,
    is_dir: bool,
    is_reparse: bool,
}

/// 目录聚合索引（不透明句柄）。
///
/// MFT 扫描时把每个目录的"归一化路径 + 子树聚合大小"收集下来，供上层（report）
/// 按规则 scope 的目录前缀匹配、计算分级可清理量。
///
/// 路径格式与 scaffold 一致：`/` 分隔 + 小写 + 无尾斜杠。
/// 这是"查询能力"而非"数据导出"：内部只存聚合结果，不暴露原始 entries。
#[derive(Debug, Clone, Default)]
pub struct DirIndex {
    /// (归一化路径, 子树逻辑大小, 子树实际分配)，按路径排序便于前缀查询
    dirs: Vec<(String, u64, u64)>,
}

impl DirIndex {
    /// 从已按路径升序排序的目录列表构造索引。
    ///
    /// 输入路径应为归一化形式（`/` 分隔 + 小写）。主要用于测试与外部构造。
    /// 分配大小默认等于逻辑大小（测试场景无簇对齐信息）。
    pub fn from_sorted_dirs(dirs: Vec<(String, u64)>) -> Self {
        DirIndex {
            dirs: dirs.into_iter().map(|(p, s)| (p, s, s)).collect(),
        }
    }

    /// 供空间核算：从含分配大小的列表构造。
    pub fn from_sorted_dirs_alloc(dirs: Vec<(String, u64, u64)>) -> Self {
        DirIndex { dirs }
    }

    /// 按路径前缀查找目录，返回 (路径, 聚合大小) 列表。
    ///
    /// 前缀匹配语义：
    /// - 大小写不敏感（入参与存储均已小写）
    /// - `*` 匹配单层任意字符（不跨 `/`）
    /// - 校验目录边界：匹配到的目录名后必须是 `/` 或字符串结尾，避免 "appdata" 误配 "appdatafoo"
    pub fn find_dirs(&self, pattern: &str) -> Vec<(String, u64)> {
        self.find_dirs_alloc(pattern)
            .into_iter()
            .map(|(p, s, _)| (p, s))
            .collect()
    }

    /// 同 `find_dirs`，但额外返回实际分配大小：`(路径, 逻辑大小, 实际分配)`。
    /// 供空间核算（逻辑 vs 实际）使用。
    pub fn find_dirs_alloc(&self, pattern: &str) -> Vec<(String, u64, u64)> {
        let pat = pattern.replace('\\', "/").to_lowercase();
        let pat = pat.trim_end_matches('/');
        // 若 pattern 不含 '*'，退化为前缀匹配
        if !pat.contains('*') {
            let prefix = format!("{}/", pat);
            return self
                .dirs
                .iter()
                .filter(|(p, _, _)| p == pat || p.starts_with(&prefix))
                .cloned()
                .collect();
        }
        // 含 '*'：按 '/' 分段做逐段匹配
        let pat_segs: Vec<&str> = pat.split('/').collect();
        self.dirs
            .iter()
            .filter(|(p, _, _)| segment_match(&pat_segs, &p.split('/').collect::<Vec<_>>()))
            .cloned()
            .collect()
    }

    /// 全盘空间核算：返回 (总逻辑, 总实际)。根目录项即全盘聚合值。
    pub fn space_totals(&self) -> (u64, u64) {
        // dirs 中第一个（根）即全盘聚合
        self.dirs
            .first()
            .map(|(_, s, a)| (*s, *a))
            .unwrap_or((0, 0))
    }

    /// 返回全部目录的 (路径, 逻辑, 实际)。供空间核算遍历。
    pub fn all_dirs_alloc(&self) -> &[(String, u64, u64)] {
        &self.dirs
    }

    /// 目录总数（诊断用）
    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.dirs.is_empty()
    }
}

/// 逐段匹配：pattern 段与 path 段等长，每段用简单的 '*' 通配匹配（不跨段）。
fn segment_match(pat: &[&str], path: &[&str]) -> bool {
    if pat.len() != path.len() {
        return false;
    }
    pat.iter().zip(path.iter()).all(|(p, s)| wildcard_seg(p, s))
}

/// 单段通配匹配：`*` 匹配段内任意字符（含空），其余字面匹配。均为小写。
fn wildcard_seg(pat: &str, text: &str) -> bool {
    // 无 '*' 直接比较
    if !pat.contains('*') {
        return pat == text;
    }
    let parts: Vec<&str> = pat.split('*').collect();
    // 必须以首段开头、末段结尾，中间依次出现
    if !text.starts_with(parts[0]) || !text.ends_with(parts[parts.len() - 1]) {
        return false;
    }
    let mut pos = parts[0].len();
    for seg in &parts[1..parts.len().saturating_sub(1)] {
        if seg.is_empty() {
            continue;
        }
        match text[pos..].find(seg) {
            Some(i) => pos += i + seg.len(),
            None => return false,
        }
    }
    // 末段与已消费位置不冲突
    pos <= text.len().saturating_sub(parts[parts.len() - 1].len())
}

/// 诊断用：逐条回调 (record_num, name, own_size, is_dir, is_reparse)，返回 (总字节, 总文件数)。
/// 仅用于排查 size 计数问题，不建树。
pub fn scan_volume_raw<F>(volume_letter: char, max_records: Option<u64>, mut cb: F) -> Result<(u64, u64)>
where
    F: FnMut(u64, &str, u64, bool, bool),
{
    let path = format!(r"\\.\{}:", volume_letter.to_ascii_uppercase());
    let file = OpenOptions::new()
        .read(true)
        .access_mode(GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .with_context(|| format!("打开卷 {} 失败（需管理员）", path))?;
    let mut reader = crate::aligned_reader::AlignedReader::new(file);
    let mut ntfs = Ntfs::new(&mut reader).context("解析 NTFS 引导扇区")?;
    ntfs.read_upcase_table(&mut reader).ok();
    let record_size = ntfs.file_record_size() as u64;
    let mft_frn = KnownNtfsFileRecordNumber::MFT as u64;
    let total_records = {
        let mft_file = ntfs.file(&mut reader, mft_frn).context("定位 $MFT")?;
        let mft_data = mft_file.data(&mut reader, "").ok_or_else(|| anyhow!("$MFT 无 $DATA"))?.context("读 $MFT $DATA")?;
        mft_data.to_attribute()?.value(&mut reader)?.len() / record_size
    };
    let scan_limit = max_records.map_or(total_records, |m| total_records.min(m));
    let rsize = record_size as usize;
    let mut total_bytes = 0u64;
    let mut total_files = 0u64;
    {
        let mft_file = ntfs.file(&mut reader, mft_frn).context("定位 $MFT")?;
        let mft_data = mft_file.data(&mut reader, "").ok_or_else(|| anyhow!("$MFT 无 $DATA"))?.context("读 $MFT $DATA")?;
        let mft_data_value = mft_data.to_attribute()?.value(&mut reader)?;
        let mut stream = mft_data_value.attach(&mut reader);
        let mut buf = vec![0u8; rsize];
        let mut record_num: u64 = 0;
        while record_num < scan_limit {
            if std::io::Read::read_exact(&mut stream, &mut buf).is_err() {
                break;
            }
            if record_num > 15 || record_num == 5 {
                if let Some(e) = crate::mft_record::parse_record(&mut buf) {
                    if !e.is_dir {
                        total_bytes += e.own_size;
                        total_files += 1;
                    }
                    cb(record_num, &e.name, e.own_size, e.is_dir, e.is_reparse);
                }
            }
            record_num += 1;
        }
    }
    Ok((total_bytes, total_files))
}

/// 扫描整个 NTFS 卷的指定子树。
pub fn scan_volume<F>(
    volume_letter: char,
    subroot: Option<&Path>,
    max_records: Option<u64>,
    mut on_progress: F,
) -> Result<(Node, DirIndex)>
where
    F: FnMut(u64, u64),
{
    let path = format!(r"\\.\{}:", volume_letter.to_ascii_uppercase());
    let file = OpenOptions::new()
        .read(true)
        .access_mode(GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .with_context(|| format!("打开卷 {} 失败（需管理员）", path))?;

    // 卷设备要求读取长度为扇区大小整数倍，用 AlignedReader 适配（否则 ntfs 的 binrw 会触发 os error 87）
    let mut reader = crate::aligned_reader::AlignedReader::new(file);
    let mut ntfs = Ntfs::new(&mut reader).context("解析 NTFS 引导扇区")?;
    ntfs.read_upcase_table(&mut reader).ok();

    let record_size = ntfs.file_record_size() as u64;
    let mft_frn = KnownNtfsFileRecordNumber::MFT as u64;

    // 从 $MFT 的 $DATA 属性长度推算记录总数
    let total_records = {
        let mft_file = ntfs.file(&mut reader, mft_frn).context("定位 $MFT")?;
        let mft_data = mft_file
            .data(&mut reader, "")
            .ok_or_else(|| anyhow!("$MFT 无 $DATA 属性"))?
            .context("读取 $MFT $DATA")?;
        let mft_data_attribute = mft_data.to_attribute()?;
        let mft_data_value = mft_data_attribute.value(&mut reader)?;
        mft_data_value.len() / record_size
    };

    let mut entries: HashMap<u64, Entry> = HashMap::new();
    let mut bytes_seen: u64 = 0;
    let mut file_count_total: u64 = 0;

    let scan_limit = max_records.map_or(total_records, |m| total_records.min(m));
    let record_size_usize = record_size as usize;

    // ---- 顺序读整个 $MFT $DATA 流（T-MFT-2 性能核心）----
    // 逐条 ntfs.file() 每条都重读 $MFT 元数据，极慢；
    // 改为一次性 attach 到 $MFT 的 $DATA 流，顺序读每条 FILE 记录并手工解析。
    {
        let mft_file = ntfs.file(&mut reader, mft_frn).context("定位 $MFT")?;
        let mft_data = mft_file
            .data(&mut reader, "")
            .ok_or_else(|| anyhow!("$MFT 无 $DATA 属性"))?
            .context("读取 $MFT $DATA")?;
        let mft_data_attribute = mft_data.to_attribute()?;
        let mft_data_value = mft_data_attribute.value(&mut reader)?;
        let stream_total_len = mft_data_value.len(); // 流总字节数（权威，须在 attach 前取）
        let mut stream = mft_data_value.attach(&mut reader);

        // 大块读（T-PERF-1）：流总长度已知（mft_data_value.len()），用它作为权威结束判据，
        // 而非依赖 read 返回 Ok(0)。缓冲区大小为 record_size 的整数倍，按记录切分；
        // 跨缓冲边界的残余字节搬到缓冲区头部，下次续读拼接，保证记录边界绝不错位。
        //
        // 逐条 read_exact 每次 1KB、约 100 万次系统调用 → ~52s；
        // 大块读把系统调用降到 ~40 次（4MB/次）→ 目标 <5s，且正确性与顺序累加基线一致。
        const CHUNK_RECORDS: usize = 4096; // 每块记录数，缓冲区 = 4096 * record_size（约 4MB @ 1KB）
        let mut chunk = vec![0u8; record_size_usize * CHUNK_RECORDS];
        let mut carry: Vec<u8> = Vec::new(); // 上次未凑满一条记录的残余字节
        let mut record_num: u64 = 0;
        let mut read_bytes_total: u64 = 0;
        'outer: loop {
            // 1) 用 carry（上次残余）填到缓冲区头部
            let mut filled = carry.len();
            chunk[..filled].copy_from_slice(&carry);
            carry.clear();

            // 2) 反复 read 直到缓冲区满 或 流真正耗尽（用累计字节数判定，而非 Ok(0)）
            while filled < chunk.len() {
                if read_bytes_total >= stream_total_len {
                    break; // 流真正耗尽（已读字节达到总长）
                }
                match std::io::Read::read(&mut stream, &mut chunk[filled..]) {
                    Ok(0) => {
                        // read 返回 0：仅当累计字节数已达总长才算真 EOF；
                        // 否则可能是数据流 run 边界的暂时无数据，重试一次后若仍 0 则退出。
                        if read_bytes_total >= stream_total_len {
                            break;
                        }
                        // 再试一次，避免误判 run 边界
                        match std::io::Read::read(&mut stream, &mut chunk[filled..]) {
                            Ok(0) => break,
                            Ok(n) => {
                                filled += n;
                                read_bytes_total += n as u64;
                            }
                            Err(_) => break,
                        }
                    }
                    Ok(n) => {
                        filled += n;
                        read_bytes_total += n as u64;
                    }
                    Err(_) => break,
                }
            }
            if filled == 0 {
                break;
            }

            // 3) 按 record_size 切分已填充数据，处理完整记录，剩余作为 carry
            let full_records = filled / record_size_usize;
            let mut off = 0usize;
            for _ in 0..full_records {
                if record_num >= scan_limit {
                    // 达到记录上限，剩余不再处理（carry 也无需保留）
                    break 'outer;
                }
                let rec = &mut chunk[off..off + record_size_usize];

                // 跳过 NTFS 系统元文件（记录号 0..=15），但保留根目录 $Root（记录号 5）：
                // 所有顶层项的 parent_frn 都指向 5，丢掉它会导致整棵树无法构建。
                if record_num > 15 || record_num == 5 {
                    if let Some(e) = crate::mft_record::parse_record(rec) {
                        if !e.is_dir {
                            bytes_seen += e.own_size;
                            file_count_total += 1;
                        }
                        entries.insert(
                            record_num,
                            Entry {
                                name: e.name,
                                parent_frn: e.parent_frn,
                                own_size: e.own_size,
                                alloc_size: e.alloc_size,
                                is_dir: e.is_dir,
                                is_reparse: e.is_reparse,
                            },
                        );
                    }
                }

                record_num += 1;
                if record_num % 10000 == 0 {
                    on_progress(record_num, bytes_seen);
                }
                off += record_size_usize;
            }

            // 4) 残余字节（不足一条记录）搬到 carry，下次续读拼接
            if off < filled {
                carry.extend_from_slice(&chunk[off..filled]);
            }

            // 流已读完且无残余 → 结束
            if read_bytes_total >= stream_total_len && carry.is_empty() {
                break;
            }
        }
        on_progress(record_num, bytes_seen);
    }

    let mut children_map: HashMap<u64, Vec<u64>> = HashMap::new();
    for (frn, e) in &entries {
        children_map.entry(e.parent_frn).or_default().push(*frn);
    }

    let root_frn = 5; // NTFS 根目录固定记录号 5
    let root_path = format!("{}:", volume_letter.to_ascii_uppercase());
    // 节点预算：全盘扫描时限制构建的 Node 总数，避免内存爆（仅用于顶层展示，size/count 聚合不受影响）
    let mut budget = NodeBudget { remaining: 200_000 };
    // 全局 visited：保证每个 FRN 只被计入一次，避免硬链接/共享子树导致的 size/count 重复计数
    // （raw 累加为 155GB，而修复前建树为 426GB，根因即建树缺少去重）。
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut root = build_node(root_frn, &entries, &children_map, subroot, &root_path, 0, &mut budget, &mut visited);

    // 根总量以顺序累加为准（唯一权威，与真实磁盘一致）；树的聚合值仅用于 top_dirs 展示。
    if let Some(r) = root.as_mut() {
        r.size = bytes_seen;
        r.file_count = file_count_total;
    }
    let dir_index = build_dir_index(&entries, &children_map, root_frn, &root_path);
    let tree = root.unwrap_or_else(|| Node {
        name: root_path.clone(),
        path: format!("{}\\", root_path),
        is_dir: true,
        size: bytes_seen,
        file_count: file_count_total,
        children: Vec::new(),
        rule_id: None,
        top_extensions: Vec::new(),
    });
    Ok((tree, dir_index))
}

/// 后序 DFS 构建目录聚合索引：回溯时累加子树 (逻辑大小, 实际分配)，
/// 收集每个目录的 (归一化路径, size, alloc)。
fn build_dir_index(
    entries: &HashMap<u64, Entry>,
    children_map: &HashMap<u64, Vec<u64>>,
    root_frn: u64,
    root_path: &str,
) -> DirIndex {
    let mut dirs: Vec<(String, u64, u64)> = Vec::new();
    let mut visited: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let root_norm = root_path.replace('\\', "/").to_lowercase();
    dfs_collect(root_frn, entries, children_map, &root_norm, 0, &mut visited, &mut dirs);
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    DirIndex { dirs }
}

/// 后序 DFS：返回以 frn 为根的子树聚合 (逻辑大小, 实际分配)，并把每个目录加入 dirs。
fn dfs_collect(
    frn: u64,
    entries: &HashMap<u64, Entry>,
    children_map: &HashMap<u64, Vec<u64>>,
    cur_path: &str,
    depth: usize,
    visited: &mut std::collections::HashSet<u64>,
    dirs: &mut Vec<(String, u64, u64)>,
) -> (u64, u64) {
    if depth > 512 {
        return (0, 0);
    }
    let Some(e) = entries.get(&frn) else {
        return (0, 0);
    };
    if !e.is_dir {
        return (e.own_size, e.alloc_size);
    }
    if e.is_reparse || !visited.insert(frn) {
        return (0, 0);
    }
    // 根目录（cur_path 为空）直接用它；其余目录在父路径后拼接自己的名字。
    // 注意：不能靠 `ends_with(':')` 判断——盘根 "c:" 也会被它的**子目录**当作父路径传入，
    // 那样会把子目录误判为根，导致所有路径退化成 "c:"。
    let norm = if frn == 5 {
        // 根目录：cur_path 已是归一化的盘根（如 "c:"）
        cur_path.to_string()
    } else {
        format!("{}/{}", cur_path, e.name.to_lowercase())
    };
    let mut size = 0u64;
    let mut alloc = 0u64;
    if let Some(kids) = children_map.get(&frn) {
        for kid in kids {
            let (s, a) = dfs_collect(*kid, entries, children_map, &norm, depth + 1, visited, dirs);
            size += s;
            alloc += a;
        }
    }
    dirs.push((norm, size, alloc));
    (size, alloc)
}

/// 节点预算：限制全盘扫描时构建的 Node 总数（内存保护）。
struct NodeBudget {
    remaining: usize,
}

/// 聚合子树大小与文件数（不构建 Node），用于预算耗尽时仍能正确汇总。
fn aggregate(
    frn: u64,
    entries: &HashMap<u64, Entry>,
    children_map: &HashMap<u64, Vec<u64>>,
    depth: usize,
    visited: &mut std::collections::HashSet<u64>,
) -> (u64, u64) {
    if depth > 512 {
        return (0, 0);
    }
    let Some(e) = entries.get(&frn) else {
        return (0, 0);
    };
    if e.is_reparse || !visited.insert(frn) {
        return (0, 0);
    }
    let mut size = 0u64;
    let mut count = 0u64;
    if e.is_dir {
        if let Some(kids) = children_map.get(&frn) {
            for kid in kids {
                let (s, c) = aggregate(*kid, entries, children_map, depth + 1, visited);
                size += s;
                count += c;
            }
        }
    } else {
        size = e.own_size;
        count = 1;
    }
    (size, count)
}

fn build_node(
    frn: u64,
    entries: &HashMap<u64, Entry>,
    children_map: &HashMap<u64, Vec<u64>>,
    subroot: Option<&Path>,
    root_path: &str,
    depth: usize,
    budget: &mut NodeBudget,
    visited: &mut std::collections::HashSet<u64>,
) -> Option<Node> {
    if depth > 256 {
        return None;
    }
    // 每个 FRN 只处理一次：防止硬链接/共享子树被重复计数（size/count 虚高）
    if !visited.insert(frn) {
        return None;
    }
    let over_budget = budget.remaining == 0;
    if !over_budget {
        budget.remaining -= 1;
    }
    let e = entries.get(&frn)?;
    if e.is_reparse {
        return None;
    }
    let path = format!("{}\\{}", root_path.trim_end_matches('\\'), e.name);

    if let Some(sr) = subroot {
        let sr_str = sr.to_string_lossy().to_lowercase().replace('/', "\\");
        let p_lower = path.to_lowercase();
        if !p_lower.starts_with(&sr_str) && !sr_str.starts_with(&p_lower) {
            if !p_lower.starts_with(&sr_str) {
                return None;
            }
        }
    }

    let mut size = 0u64;
    let mut file_count = 0u64;
    let mut children = Vec::new();
    let mut ext_map: HashMap<String, (u64, u64)> = HashMap::new();

    if e.is_dir {
        if let Some(kids) = children_map.get(&frn) {
            for kid in kids {
                if over_budget {
                    let (s, c) = aggregate(*kid, entries, children_map, depth + 1, visited);
                    size += s;
                    file_count += c;
                } else if let Some(child) = build_node(*kid, entries, children_map, subroot, &path, depth + 1, budget, visited) {
                    size += child.size;
                    file_count += child.file_count;
                    children.push(child);
                } else {
                    // 已 visited 或超预算：用 aggregate 汇总（共享 visited 以一致去重）
                    let (s, c) = aggregate(*kid, entries, children_map, depth + 1, visited);
                    size += s;
                    file_count += c;
                }
            }
        }
    } else {
        size = e.own_size;
        file_count = 1;
        let ext = Path::new(&e.name)
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "(none)".into());
        let slot = ext_map.entry(ext).or_insert((0, 0));
        slot.0 += e.own_size;
        slot.1 += 1;
    }

    children.sort_by_key(|c| std::cmp::Reverse(c.size));
    let mut top_extensions: Vec<ExtShare> = ext_map
        .into_iter()
        .map(|(ext, (b, c))| ExtShare { ext, bytes: b, count: c })
        .collect();
    top_extensions.sort_by_key(|x| std::cmp::Reverse(x.bytes));
    top_extensions.truncate(8);

    Some(Node {
        name: e.name.clone(),
        path,
        is_dir: e.is_dir,
        size,
        file_count,
        children,
        rule_id: None,
        top_extensions,
    })
}

#[cfg(test)]
mod dir_index_tests {
    use super::*;

    fn idx(entries: &[(&str, u64)]) -> DirIndex {
        let mut dirs: Vec<(String, u64, u64)> = entries
            .iter()
            .map(|(p, s)| (p.replace('\\', "/").to_lowercase(), *s, *s))
            .collect();
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        DirIndex { dirs }
    }

    #[test]
    fn prefix_match_exact_dir() {
        let d = idx(&[
            ("c:/users/me/appdata/local", 100),
            ("c:/users/me/appdata/roaming", 200),
        ]);
        let r = d.find_dirs("C:/Users/me/AppData/Local");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, 100);
    }

    #[test]
    fn prefix_match_subtree() {
        // 前缀匹配应包含子目录
        let d = idx(&[
            ("c:/users/me/appdata/local", 100),
            ("c:/users/me/appdata/local/temp", 30),
            ("c:/users/me/appdata/roaming", 200),
        ]);
        let r = d.find_dirs("c:/users/me/appdata/local");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn no_partial_name_false_match() {
        // "appdata" 不应误配 "appdatafoo"
        let d = idx(&[
            ("c:/users/me/appdata", 100),
            ("c:/users/me/appdatafoo", 999),
        ]);
        let r = d.find_dirs("c:/users/me/appdata");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, "c:/users/me/appdata");
    }

    #[test]
    fn wildcard_single_segment() {
        // .../User Data/*/Cache 应匹配 Default/Cache，不匹配更深的
        let d = idx(&[
            ("c:/local/google/chrome/user data/default/cache", 120),
            ("c:/local/google/chrome/user data/profile 1/cache", 80),
            ("c:/local/google/chrome/user data/default/cache/deep", 5),
        ]);
        let r = d.find_dirs("c:/local/google/chrome/user data/*/cache");
        assert_eq!(r.len(), 2, "应匹配两层 * 下的 Cache，不匹配更深");
    }

    #[test]
    fn case_insensitive() {
        let d = idx(&[("c:/windows/temp", 42)]);
        let r = d.find_dirs("C:/WINDOWS/TEMP");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, 42);
    }

    #[test]
    fn no_match_returns_empty() {
        let d = idx(&[("c:/foo", 1)]);
        assert!(d.find_dirs("c:/bar").is_empty());
    }
}
