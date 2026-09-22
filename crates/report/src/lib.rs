//! wcs-report — 报告引擎（JSON + pretty + 健康评分）。

use serde::{Deserialize, Serialize};
use wcs_scanner::mft::DirIndex;
use wcs_scanner::{ChildInfo, DiskSpace, Node, ScanStats};
use wcs_scaffold::{Risk, Rule};

pub mod migrate;
pub use migrate::{MigrateAdvice, MigrateMethod, MigrateSuggestion};

/// 报告视图
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// 报告 schema 版本（消费方据此判断兼容性）
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub generated_at: String,
    pub mode: String,
    pub health: HealthScore,
    pub summary: Summary,
    pub top_dirs: Vec<ChildInfo>,
    pub scan_stats: Option<ScanStats>,
}

/// 当前报告 schema 版本。
pub fn default_schema_version() -> String {
    "1".to_string()
}

/// 健康评分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthScore {
    pub score: u8,
    pub grade: String,
    pub used_percent: f32,
    pub free_gb: f64,
    pub total_gb: f64,
    pub advice: String,
}

/// 空间核算结果：逻辑大小 vs 实际分配（簇对齐浪费）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceAudit {
    /// 全盘逻辑大小（字节）
    pub logical_bytes: u64,
    /// 全盘实际分配（簇对齐，字节）
    pub allocated_bytes: u64,
    /// 浪费 = allocated - logical（簇对齐导致）
    pub waste_bytes: u64,
    /// 浪费率（waste / allocated）
    pub waste_ratio: f64,
    /// 浪费最多的目录（Top N）
    pub top_waste_dirs: Vec<WasteDir>,
}

/// 单个目录的浪费明细
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasteDir {
    pub path: String,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    /// 该目录子树的总浪费（allocated - logical）
    pub waste_bytes: u64,
    /// 本层新增浪费 = 本目录浪费 - 父目录浪费。
    /// 大值表示"浪费从这一层开始堆积"，用于定位真正的小文件重灾区。
    pub self_waste: u64,
}

/// 空间核算：全盘逻辑/实际/浪费 + top 浪费目录。
///
/// 数据来自 `DirIndex`（后序 DFS 已聚合每个目录的逻辑与分配大小）。
/// `top_n` 控制 top 浪费目录数量（按 waste 降序，仅取有浪费的直接子目录层）。
pub fn space_audit(idx: &DirIndex, top_n: usize) -> SpaceAudit {
    let (logical, allocated) = idx.space_totals();
    let waste = allocated.saturating_sub(logical);
    let waste_ratio = if allocated > 0 {
        waste as f64 / allocated as f64
    } else {
        0.0
    };

    // 建 path → waste 的映射，便于 O(1) 查父目录浪费
    let waste_of: std::collections::HashMap<&str, u64> = idx
        .all_dirs_alloc()
        .iter()
        .map(|(p, l, a)| (p.as_str(), a.saturating_sub(*l)))
        .collect();

    // top 浪费目录：按"本层新增浪费"（self_waste）排序，
    // 这样不会被层层继承的父目录链占满，而指向浪费真正开始堆积的层级。
    let mut candidates: Vec<WasteDir> = Vec::new();
    for (path, l, a) in idx.all_dirs_alloc() {
        let w = a.saturating_sub(*l);
        if w == 0 || !path.contains('/') {
            continue;
        }
        // 父目录路径：去掉最后一段
        let parent_waste = path
            .rfind('/')
            .map(|i| &path[..i])
            .and_then(|parent| waste_of.get(parent).copied())
            .unwrap_or(0);
        let self_waste = w.saturating_sub(parent_waste);
        candidates.push(WasteDir {
            path: path.clone(),
            logical_bytes: *l,
            allocated_bytes: *a,
            waste_bytes: w,
            self_waste,
        });
    }
    candidates.sort_by(|x, y| y.self_waste.cmp(&x.self_waste));
    candidates.truncate(top_n);

    SpaceAudit {
        logical_bytes: logical,
        allocated_bytes: allocated,
        waste_bytes: waste,
        waste_ratio,
        top_waste_dirs: candidates,
    }
}

/// 单条规则的分级明细
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleGrade {
    pub rule_id: String,
    pub name: String,
    pub risk: Risk,
    /// 该规则的可清理字节数（按 scope 前缀匹配 + 去重）
    pub bytes: u64,
    /// 命中的目录数（去重前）
    pub dir_count: usize,
    /// 示例路径（规则 detect 首项），供 GUI 预填"要清理的根路径"
    pub sample_path: String,
}

/// 摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// GREEN（low 风险）可清理总量（GB）
    pub safely_cleanable_gb: f64,
    /// YELLOW（medium 风险）需确认总量（GB）
    pub needs_confirm_gb: f64,
    /// RED（high 风险）总量（GB）
    pub high_risk_gb: f64,
    pub top_dirs: Vec<ChildInfo>,
}

/// 计算健康评分（使用率 50% 时 100 分，每高 1% 扣 2 分）
pub fn health_score(used_percent: f32, free_gb: f64, total_gb: f64) -> HealthScore {
    let raw = 100.0 - (used_percent - 50.0) * 2.0;
    let score = raw.clamp(0.0, 100.0) as u8;
    let grade = match score {
        90..=100 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    }
    .to_string();
    let advice = if score >= 80 {
        "空间健康".to_string()
    } else if score >= 60 {
        "需关注".to_string()
    } else {
        "空间紧张，建议清理".to_string()
    };
    HealthScore { score, grade, used_percent, free_gb, total_gb, advice }
}

/// 构建报告
pub fn build(tree: &Node, scan_stats: Option<ScanStats>) -> Report {
    build_with_disk(tree, scan_stats, None, &[])
}

/// 构建报告（带磁盘空间 + 规则分级）
pub fn build_with_disk(
    tree: &Node,
    scan_stats: Option<ScanStats>,
    disk: Option<DiskSpace>,
    rules: &[Rule],
) -> Report {
    build_with_index(tree, scan_stats, disk, rules, None)
}

/// 构建报告（带目录索引，分级量更准且稳定）。
///
/// 若提供 `dir_index`，分级量按规则 scope 的目录前缀在索引中查询、去重后累加——
/// 不受 MFT 建树预算（NodeBudget）影响，多次运行稳定。
/// 否则回退到按 tree 节点 `rule_id` 累加（受预算影响，仅作兼容）。
pub fn build_with_index(
    tree: &Node,
    scan_stats: Option<ScanStats>,
    disk: Option<DiskSpace>,
    rules: &[Rule],
    dir_index: Option<&DirIndex>,
) -> Report {
    let top_dirs: Vec<ChildInfo> = tree
        .children
        .iter()
        .take(15)
        .map(|c| ChildInfo { name: c.name.clone(), size: c.size, is_dir: c.is_dir })
        .collect();

    // 健康评分
    let health = match disk {
        Some(d) if d.total_bytes > 0 => {
            let used_percent = (d.used_bytes as f64 / d.total_bytes as f64 * 100.0) as f32;
            health_score(
                used_percent,
                d.free_bytes as f64 / 1e9,
                d.total_bytes as f64 / 1e9,
            )
        }
        _ => health_score(0.0, 0.0, 0.0),
    };

    // 按规则风险等级汇总可清理量
    let (safe, confirm, high) = match dir_index {
        // 优先：用目录索引按 scope 前缀精确累加（不受建树预算影响）
        Some(idx) => grade_from_index(idx, rules),
        // 回退：按 tree 中匹配 rule_id 的节点大小累加（受预算影响，兼容旧路径）
        None => grade_from_tree(tree, rules),
    };

    Report {
        schema_version: default_schema_version(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        mode: "scan".to_string(),
        health,
        summary: Summary {
            safely_cleanable_gb: safe as f64 / 1e9,
            needs_confirm_gb: confirm as f64 / 1e9,
            high_risk_gb: high as f64 / 1e9,
            top_dirs: top_dirs.clone(),
        },
        top_dirs,
        scan_stats,
    }
}

/// 从 tree 节点 `rule_id` 累加分级量（回退路径）。
fn grade_from_tree(tree: &Node, rules: &[Rule]) -> (u64, u64, u64) {
    let risk_of = |rule_id: &str| -> Option<Risk> {
        rules.iter().find(|r| r.id == rule_id).map(|r| r.risk)
    };
    fn walk(node: &Node, risk_of: &dyn Fn(&str) -> Option<Risk>, safe: &mut u64, confirm: &mut u64, high: &mut u64) {
        if let Some(rid) = &node.rule_id {
            match risk_of(rid) {
                Some(Risk::Low) => *safe += node.size,
                Some(Risk::Medium) => *confirm += node.size,
                Some(Risk::High) => *high += node.size,
                None => {}
            }
        }
        for c in &node.children {
            walk(c, risk_of, safe, confirm, high);
        }
    }
    let (mut safe, mut confirm, mut high) = (0u64, 0u64, 0u64);
    walk(tree, &risk_of, &mut safe, &mut confirm, &mut high);
    (safe, confirm, high)
}

/// 按规则返回分级明细（每条规则的命中量与可清理字节）。
///
/// 与 `grade_from_index` 同源，但返回逐规则明细，供 GUI 展示。
pub fn grade_detail(idx: &DirIndex, rules: &[Rule]) -> Vec<RuleGrade> {
    let mut out = Vec::new();
    for rule in rules {
        let mut bytes = 0u64;
        let mut dir_count = 0usize;
        for scope in &rule.scopes {
            let prefix = wcs_scaffold::scope_dir_prefix(&scope.glob);
            if prefix.is_empty() {
                continue;
            }
            let hits = idx.find_dirs(&prefix);
            dir_count += hits.len();
            bytes += sum_dedup(&hits);
        }
        out.push(RuleGrade {
            rule_id: rule.id.clone(),
            name: rule.name.clone(),
            risk: rule.risk,
            bytes,
            dir_count,
            sample_path: rule.detect.first().cloned().unwrap_or_default(),
        });
    }
    out
}

/// 用目录索引按规则 scope 前缀累加分级量（主路径）。
///
/// 每条规则的每个 scope：
/// 1. 把 scope.glob 转成"目录前缀"（展开环境变量、去掉尾部通配段）
/// 2. 在索引中查命中目录
/// 3. 去重（去除"父也被命中"的子目录，避免重复计入）
/// 4. 按规则风险等级累加
fn grade_from_index(idx: &DirIndex, rules: &[Rule]) -> (u64, u64, u64) {
    let (mut safe, mut confirm, mut high) = (0u64, 0u64, 0u64);
    for rule in rules {
        let mut bytes = 0u64;
        for scope in &rule.scopes {
            let prefix = wcs_scaffold::scope_dir_prefix(&scope.glob);
            if prefix.is_empty() {
                continue;
            }
            let hits = idx.find_dirs(&prefix);
            bytes += sum_dedup(&hits);
        }
        match rule.risk {
            Risk::Low => safe += bytes,
            Risk::Medium => confirm += bytes,
            Risk::High => high += bytes,
        }
    }
    (safe, confirm, high)
}

/// 对"命中目录列表"求和，去除父子包含导致的重复。
///
/// 命中列表可能很大（如某 scope 前缀命中上万个子目录），故用 O(n log n) 排序 + O(n) 扫描：
/// 路径按字典序排序后，某目录的祖先必然紧邻在它之前（中间只可能夹该祖先的其他后代）。
/// 因此只需检查"上一个被接受的路径"是否为当前路径的祖先——是则跳过（祖先的 size 已含它）。
fn sum_dedup(hits: &[(String, u64)]) -> u64 {
    let mut sorted: Vec<&(String, u64)> = hits.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut total = 0u64;
    let mut last_accepted: Option<&str> = None;
    for (path, size) in sorted {
        let covered = last_accepted
            .map(|acc| path.len() > acc.len() && path.starts_with(acc) && path.as_bytes()[acc.len()] == b'/')
            .unwrap_or(false);
        if !covered {
            total += size;
            last_accepted = Some(path);
        }
    }
    total
}

/// 输出 JSON
pub fn to_json(report: &Report) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

/// 输出人类可读
pub fn to_pretty(report: &Report) -> String {
    let mut s = String::new();
    s.push_str("=== Win-C-Scanary 报告 ===\n");
    s.push_str(&format!(
        "健康评分: {}/100 ({})  {}\n",
        report.health.score, report.health.grade, report.health.advice
    ));
    if report.health.total_gb > 0.0 {
        s.push_str(&format!(
            "磁盘: 已用 {:.1}% / 可用 {:.1} GB / 共 {:.1} GB\n",
            report.health.used_percent, report.health.free_gb, report.health.total_gb
        ));
    }
    s.push_str(&format!(
        "可清理: 安全 {:.2} GB / 需确认 {:.2} GB / 高风险 {:.2} GB\n",
        report.summary.safely_cleanable_gb,
        report.summary.needs_confirm_gb,
        report.summary.high_risk_gb
    ));
    s.push_str("\nTop 目录:\n");
    for d in &report.top_dirs {
        s.push_str(&format!("  {:<40} {:>10.2} GB\n", d.name, d.size as f64 / 1e9));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_dedup_keeps_parent_only() {
        // 父目录 size 已含子目录，去重后只计父
        let hits = vec![
            ("c:/a/cache".to_string(), 100),
            ("c:/a/cache/sub".to_string(), 30),
            ("c:/a/cache/sub/deep".to_string(), 10),
        ];
        assert_eq!(sum_dedup(&hits), 100);
    }

    #[test]
    fn sum_dedup_keeps_disjoint() {
        // 无父子关系的目录，全部累加
        let hits = vec![
            ("c:/a/x".to_string(), 100),
            ("c:/a/y".to_string(), 50),
        ];
        assert_eq!(sum_dedup(&hits), 150);
    }

    #[test]
    fn sum_dedup_no_partial_name_false_parent() {
        // "c:/a/cache" 不应被当作 "c:/a/cachefoo" 的父
        let hits = vec![
            ("c:/a/cache".to_string(), 100),
            ("c:/a/cachefoo".to_string(), 7),
        ];
        assert_eq!(sum_dedup(&hits), 107);
    }

    fn mk_scope(glob: &str) -> wcs_scaffold::Scope {
        wcs_scaffold::Scope {
            id: "s".into(),
            label: String::new(),
            glob: glob.into(),
            mode: wcs_scaffold::Mode::Recycle,
            prompt: None,
            category: None,
            variant: None,
            recycle_granularity: Default::default(),
        }
    }

    fn mk_rule(id: &str, risk: Risk, scopes: Vec<wcs_scaffold::Scope>) -> Rule {
        Rule {
            id: id.into(),
            name: id.into(),
            homepage: None,
            risk,
            disclaimer: String::new(),
            detect: vec!["%TEMP%".into()],
            matcher: Default::default(),
            scopes,
        }
    }

    #[test]
    fn grade_from_index_prefix_and_dedup() {
        // 索引含一个 Chrome Cache 目录及其子目录
        let dirs = vec![
            ("c:/local/google/chrome/user data/default/cache".to_string(), 120u64),
            ("c:/local/google/chrome/user data/default/cache/extra".to_string(), 20u64),
            ("c:/unrelated".to_string(), 999u64),
        ];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rule = mk_rule(
            "browser-cache",
            Risk::Low,
            vec![mk_scope("c:/local/google/chrome/user data/*/cache/**")],
        );
        let (safe, confirm, high) = grade_from_index(&idx, &[rule]);
        assert_eq!(safe, 120, "只计父目录（子目录去重）");
        assert_eq!(confirm, 0);
        assert_eq!(high, 0);
    }

    #[test]
    fn grade_from_index_by_risk() {
        // 两条规则，不同风险等级，各自命中不同目录
        let dirs = vec![
            ("c:/safe/cache".to_string(), 50u64),
            ("c:/warn/cache".to_string(), 80u64),
            ("c:/danger/cache".to_string(), 999u64),
        ];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![
            mk_rule("low-rule", Risk::Low, vec![mk_scope("c:/safe/cache/**")]),
            mk_rule("med-rule", Risk::Medium, vec![mk_scope("c:/warn/cache/**")]),
            mk_rule("high-rule", Risk::High, vec![mk_scope("c:/danger/cache/**")]),
        ];
        let (safe, confirm, high) = grade_from_index(&idx, &rules);
        assert_eq!(safe, 50);
        assert_eq!(confirm, 80);
        assert_eq!(high, 999);
    }

    #[test]
    fn grade_from_index_no_match_is_zero() {
        let idx = DirIndex::from_sorted_dirs(vec![("c:/a".to_string(), 1)]);
        let rule = mk_rule("r", Risk::Low, vec![mk_scope("c:/nonexistent/**")]);
        let (safe, _, _) = grade_from_index(&idx, &[rule]);
        assert_eq!(safe, 0);
    }

    #[test]
    fn grade_detail_lists_each_rule() {
        let dirs = vec![
            ("c:/safe/cache".to_string(), 50u64),
            ("c:/safe/cache/sub".to_string(), 10u64),
        ];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![
            mk_rule("r1", Risk::Low, vec![mk_scope("c:/safe/cache/**")]),
            mk_rule("r2", Risk::Medium, vec![mk_scope("c:/absent/**")]),
        ];
        let detail = grade_detail(&idx, &rules);
        assert_eq!(detail.len(), 2);
        // r1：只计父 50，命中 2 个目录
        assert_eq!(detail[0].rule_id, "r1");
        assert_eq!(detail[0].bytes, 50);
        assert_eq!(detail[0].dir_count, 2);
        assert_eq!(detail[0].sample_path, "%TEMP%");
        // r2：无命中
        assert_eq!(detail[1].bytes, 0);
        assert_eq!(detail[1].dir_count, 0);
    }

    #[test]
    fn space_audit_totals() {
        // 根 c: 逻辑1000 实际1100 → waste 100
        let dirs = vec![
            ("c:".to_string(), 1000u64, 1100u64),
            ("c:/a".to_string(), 500u64, 560u64),   // waste 60，父 c: waste 100 → self 饱和 0
            ("c:/a/b".to_string(), 100u64, 130u64), // waste 30，父 a waste 60 → self 饱和 0
            ("c:/x".to_string(), 50u64, 90u64),     // waste 40，父 c: waste 100 → self 0
        ];
        let idx = DirIndex::from_sorted_dirs_alloc(dirs);
        let audit = space_audit(&idx, 10);
        assert_eq!(audit.logical_bytes, 1000);
        assert_eq!(audit.allocated_bytes, 1100);
        assert_eq!(audit.waste_bytes, 100);
        assert!((audit.waste_ratio - 100.0 / 1100.0).abs() < 1e-9);
    }

    #[test]
    fn space_audit_self_waste_ranks_new_waste() {
        // 构造：父 waste 大但子 waste 更大的场景——self_waste 应指向"新增浪费"的目录
        // c: waste 20；c:/a waste 20（无新增）；c:/a/b waste 80（本层新增 60）→ 应排第一
        let dirs = vec![
            ("c:".to_string(), 980u64, 1000u64),        // waste 20
            ("c:/a".to_string(), 480u64, 500u64),       // waste 20, self = 20-20 = 0
            ("c:/a/b".to_string(), 100u64, 180u64),     // waste 80, self = 80-20 = 60 ← 最大
            ("c:/x".to_string(), 50u64, 60u64),         // waste 10, self = 10-20 饱和 0
        ];
        let idx = DirIndex::from_sorted_dirs_alloc(dirs);
        let audit = space_audit(&idx, 10);
        // 第一条应是 c:/a/b（self_waste 60 最大）
        assert_eq!(audit.top_waste_dirs[0].path, "c:/a/b");
        assert_eq!(audit.top_waste_dirs[0].self_waste, 60);
    }
}
