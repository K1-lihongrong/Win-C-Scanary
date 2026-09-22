//! 迁移指导 —— 分析哪些缓存目录适合迁移到其他盘，给出迁移命令。
//!
//! **纯建议，零风险**：本模块只做分析与文本生成，不执行任何文件操作。
//!
//! 数据来源：已有的 `DirIndex`（MFT 扫描产出）+ 规则表。
//! 对每条规则的每个 scope：转目录前缀 → 在索引中查命中目录 → 去重求和，
//! 得到"该缓存目录当前占用多少"，作为是否值得迁移的依据。

use serde::{Deserialize, Serialize};
use wcs_scanner::mft::DirIndex;
use wcs_scaffold::{Risk, Rule};

/// 迁移方式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MigrateMethod {
    /// 目录联接（junction）：mklink /J，本地卷迁移首选，免管理员
    Junction,
    /// 符号链接：mklink /D，目标是网络路径时用
    Symlink,
    /// 环境变量重定向：改缓存目录的环境变量/配置，无需链接
    EnvVar,
}

/// 一条迁移建议
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateSuggestion {
    /// 来源规则 id
    pub rule_id: String,
    /// 规则名
    pub rule_name: String,
    /// 规则风险等级（迁移不改变风险，仅供参考）
    pub risk: Risk,
    /// 缓存目录当前路径（归一化后的实际命中路径）
    pub source_path: String,
    /// 当前占用字节
    pub size_bytes: u64,
    /// 建议的迁移方式
    pub method: MigrateMethod,
    /// 可直接复制执行的命令（用户自行执行，本工具不执行）
    pub command: String,
    /// 说明
    pub note: String,
}

/// 迁移建议汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateAdvice {
    /// 分析的卷（如 "C:"）
    pub volume: String,
    /// 候选目标盘提示
    pub note: String,
    /// 建议列表（按 size 降序）
    pub suggestions: Vec<MigrateSuggestion>,
    /// 建议迁移的总字节
    pub total_bytes: u64,
}

/// 已知支持"环境变量/配置重定向"的缓存规则：规则 id → (环境变量名, 说明)。
///
/// 这些缓存工具支持把缓存目录指到别的盘，比符号链接更干净（无需链接、工具原生支持）。
fn env_redirect_for(rule_id: &str, scope_id: &str) -> Option<(&'static str, &'static str)> {
    // 依据 scope_id 精确匹配，避免同一规则下不同 scope 给错建议
    let key = scope_id;
    if key.contains("cargo") {
        return Some(("CARGO_HOME", "设置 CARGO_HOME 指向新盘目录（缓存会写在新位置）"));
    }
    if key.contains("pip") {
        return Some(("PIP_CACHE_DIR", "设置 PIP_CACHE_DIR 指向新盘目录"));
    }
    if key.contains("npm") {
        return Some(("npm_config_cache", "运行 npm config set cache <新路径>"));
    }
    if key.contains("conda") {
        return Some(("CONDA_PKGS_DIRS", "设置 CONDA_PKGS_DIRS 指向新盘目录"));
    }
    if key.contains("maven") || rule_id == "dev-caches" && key.contains("m2") {
        return Some(("MAVEN_OPTS", "在 settings.xml 里改 localRepository 到新盘"));
    }
    if key.contains("gradle") {
        return Some(("GRADLE_USER_HOME", "设置 GRADLE_USER_HOME 指向新盘目录"));
    }
    None
}

/// 生成 junction 命令：把原目录搬到目标盘，再在原位建 junction。
///
/// 目标路径为 `<目标盘>:\wcs-migrate\<rule_id>\<scope_id>`，按规则/scope 命名，
/// 确保多条建议不会在目标盘撞名。命令为分步形式（用户按顺序执行），本工具不执行。
fn junction_command(source: &str, target_drive: char, rule_id: &str, scope_id: &str) -> String {
    let dst = format!("{}:\\wcs-migrate\\{}\\{}", target_drive, rule_id, scope_id);
    format!(
        "robocopy \"{src}\" \"{dst}\" /E /MOVE & mklink /J \"{src}\" \"{dst}\"",
        src = source,
        dst = dst,
    )
}

/// 分析迁移建议。
///
/// * `idx` — 目录索引（MFT 扫描产出）
/// * `rules` — 规则表
/// * `volume` — 被分析的卷（如 "C:"），用于提示目标盘
/// * `target_drive` — 建议迁移到的目标盘符字母（如 'D'）
/// * `top_n` — 最多返回多少条建议
pub fn analyze(
    idx: &DirIndex,
    rules: &[Rule],
    volume: &str,
    target_drive: char,
    top_n: usize,
) -> MigrateAdvice {
    let mut suggestions: Vec<MigrateSuggestion> = Vec::new();

    for rule in rules {
        for scope in &rule.scopes {
            let prefix = wcs_scaffold::scope_dir_prefix(&scope.glob);
            if prefix.is_empty() {
                continue;
            }
            let hits = idx.find_dirs(&prefix);
            if hits.is_empty() {
                continue;
            }
            let size = sum_dedup(&hits);
            if size == 0 {
                continue;
            }
            // 取最大的命中目录作为代表路径（迁移时通常迁整个 scope 的命中目录）
            let source_path = hits
                .iter()
                .max_by_key(|(_, s)| *s)
                .map(|(p, _)| p.clone())
                .unwrap_or_else(|| prefix.clone());

            // 优先给"环境变量重定向"（若该缓存支持），否则给 junction
            let (method, command, note) = match env_redirect_for(&rule.id, &scope.id) {
                Some((env, tip)) => (
                    MigrateMethod::EnvVar,
                    format!(
                        "# {} = {}:\\wcs-migrate\\{}\\{}（先在新盘建好该目录再设置）",
                        env, target_drive, rule.id, scope.id
                    ),
                    format!("{}；比符号链接更干净，工具原生支持", tip),
                ),
                None => (
                    MigrateMethod::Junction,
                    junction_command(&source_path, target_drive, &rule.id, &scope.id),
                    "junction（mklink /J）仅支持本地卷、免管理员；若目标是网络路径请改用 mklink /D".to_string(),
                ),
            };

            suggestions.push(MigrateSuggestion {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                risk: rule.risk,
                source_path,
                size_bytes: size,
                method,
                command,
                note,
            });
        }
    }

    // 去重：同一 rule 的多个 scope 可能都指向同一目录前缀，这里按 source_path 去重保留最大
    suggestions.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.source_path.cmp(&b.source_path)));
    let mut seen = std::collections::HashSet::new();
    suggestions.retain(|s| seen.insert(s.source_path.clone()));

    let total_bytes = suggestions.iter().map(|s| s.size_bytes).sum();
    suggestions.truncate(top_n);

    MigrateAdvice {
        volume: volume.to_string(),
        note: format!(
            "建议迁移到 {} 盘等非系统盘；命令仅供参考，请确认路径后自行执行（本工具不执行任何迁移）。",
            target_drive
        ),
        suggestions,
        total_bytes,
    }
}

/// 对"命中目录列表"求和并去重（与 report::sum_dedup 同语义，避免跨模块依赖私有函数）。
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

#[cfg(test)]
mod tests {
    use super::*;
    use wcs_scaffold::{Match, Mode, Prompt, RecycleGranularity, Scope};

    fn mk_scope(id: &str, glob: &str) -> Scope {
        Scope {
            id: id.into(),
            label: String::new(),
            glob: glob.into(),
            mode: Mode::Recycle,
            prompt: None::<Prompt>,
            category: None,
            variant: None,
            recycle_granularity: RecycleGranularity::File,
        }
    }

    fn mk_rule(id: &str, risk: Risk, scopes: Vec<Scope>) -> Rule {
        Rule {
            id: id.into(),
            name: id.into(),
            homepage: None,
            risk,
            disclaimer: String::new(),
            detect: vec![],
            matcher: Match::default(),
            scopes,
        }
    }

    #[test]
    fn empty_index_yields_no_suggestions() {
        let idx = DirIndex::default();
        let rules = vec![mk_rule("npm-cache", Risk::Low, vec![mk_scope("npm-cache-local", "c:/local/npm-cache/**")])];
        let advice = analyze(&idx, &rules, "C:", 'D', 20);
        assert!(advice.suggestions.is_empty());
        assert_eq!(advice.total_bytes, 0);
    }

    #[test]
    fn npm_gets_env_var_method() {
        let dirs = vec![("c:/local/npm-cache".to_string(), 500_000_000u64)];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![mk_rule("npm-cache", Risk::Low, vec![mk_scope("npm-cache-local", "c:/local/npm-cache/**")])];
        let advice = analyze(&idx, &rules, "C:", 'D', 20);
        assert_eq!(advice.suggestions.len(), 1);
        assert_eq!(advice.suggestions[0].method, MigrateMethod::EnvVar);
        assert!(advice.suggestions[0].command.contains("npm_config_cache"));
        assert_eq!(advice.total_bytes, 500_000_000);
    }

    #[test]
    fn unknown_cache_gets_junction_method() {
        let dirs = vec![("c:/some/mystery-cache".to_string(), 100_000_000u64)];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![mk_rule("mystery", Risk::Low, vec![mk_scope("mystery-scope", "c:/some/mystery-cache/**")])];
        let advice = analyze(&idx, &rules, "C:", 'D', 20);
        assert_eq!(advice.suggestions.len(), 1);
        assert_eq!(advice.suggestions[0].method, MigrateMethod::Junction);
        assert!(advice.suggestions[0].command.contains("mklink /J"));
    }

    #[test]
    fn suggestions_sorted_desc_by_size() {
        let dirs = vec![
            ("c:/a/small".to_string(), 10u64),
            ("c:/b/big".to_string(), 1000u64),
            ("c:/c/mid".to_string(), 100u64),
        ];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![
            mk_rule("r-a", Risk::Low, vec![mk_scope("s-a", "c:/a/small/**")]),
            mk_rule("r-b", Risk::Low, vec![mk_scope("s-b", "c:/b/big/**")]),
            mk_rule("r-c", Risk::Low, vec![mk_scope("s-c", "c:/c/mid/**")]),
        ];
        let advice = analyze(&idx, &rules, "C:", 'D', 20);
        let sizes: Vec<u64> = advice.suggestions.iter().map(|s| s.size_bytes).collect();
        assert_eq!(sizes, vec![1000, 100, 10]);
    }

    #[test]
    fn top_n_limits_output() {
        let dirs: Vec<(String, u64)> = (0..10).map(|i| (format!("c:/d{}", i), (i as u64 + 1) * 100)).collect();
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules: Vec<Rule> = (0..10)
            .map(|i| mk_rule(&format!("r{}", i), Risk::Low, vec![mk_scope(&format!("s{}", i), &format!("c:/d{}/**", i))]))
            .collect();
        let advice = analyze(&idx, &rules, "C:", 'D', 3);
        assert_eq!(advice.suggestions.len(), 3);
        // total_bytes 是截断前的总和
        assert_eq!(advice.total_bytes, (1..=10).map(|i| i * 100).sum::<u64>());
    }

    #[test]
    fn dedup_same_path_keeps_one() {
        // 两个 scope 命中同一路径，去重后只留一条
        let dirs = vec![("c:/same".to_string(), 500u64)];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![mk_rule(
            "dup",
            Risk::Low,
            vec![mk_scope("dup-a", "c:/same/**"), mk_scope("dup-b", "c:/same/**")],
        )];
        let advice = analyze(&idx, &rules, "C:", 'D', 20);
        assert_eq!(advice.suggestions.len(), 1);
        assert_eq!(advice.suggestions[0].size_bytes, 500);
    }

    #[test]
    fn env_redirect_covers_known_tools() {
        assert!(env_redirect_for("dev-caches", "pip-cache").is_some());
        assert!(env_redirect_for("dev-caches", "cargo-registry-cache").is_some());
        assert!(env_redirect_for("npm-cache", "npm-cache-local").is_some());
        assert!(env_redirect_for("conda", "conda-pkgs-anaconda").is_some());
        assert!(env_redirect_for("dev-caches", "gradle-caches").is_some());
        assert!(env_redirect_for("wechat-pc", "wechat-cache").is_none());
    }

    #[test]
    fn junction_command_shape() {
        let cmd = junction_command("c:/users/me/.cargo/registry/cache", 'D', "dev-caches", "cargo-registry-cache");
        assert!(cmd.contains("mklink /J"));
        assert!(cmd.contains("robocopy"));
        // 目标路径按 rule_id/scope_id 命名，避免撞名
        assert!(cmd.contains("D:\\wcs-migrate\\dev-caches\\cargo-registry-cache"));
    }

    #[test]
    fn junction_targets_are_unique_per_scope() {
        let dirs = vec![
            ("c:/a/edge/cache".to_string(), 100u64),
            ("c:/a/chrome/cache".to_string(), 200u64),
        ];
        let idx = DirIndex::from_sorted_dirs(dirs);
        let rules = vec![mk_rule(
            "browser-cache",
            Risk::Low,
            vec![
                mk_scope("edge-cache", "c:/a/edge/cache/**"),
                mk_scope("chrome-cache", "c:/a/chrome/cache/**"),
            ],
        )];
        let advice = analyze(&idx, &rules, "C:", 'D', 20);
        let cmds: Vec<&str> = advice.suggestions.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(cmds.len(), 2);
        assert!(
            cmds.iter().any(|c| c.contains("edge-cache"))
                && cmds.iter().any(|c| c.contains("chrome-cache"))
        );
    }
}
