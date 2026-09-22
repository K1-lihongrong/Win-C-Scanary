//! wcs-cli — 命令行入口（二进制名 scanary）。

mod gui;
mod hiber;
mod system;
mod uninstall;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "scanary", version, about = "Win-C-Scanary 磁盘扫描清理工具")]
struct Cli {
    #[arg(long, env = "WCS_RULES_DIR")]
    rules_dir: Option<PathBuf>,
    #[arg(long, env = "WCS_UNDO_LOG")]
    undo_log: Option<PathBuf>,
    #[arg(long, env = "WCS_QUARANTINE_ROOT")]
    quarantine_root: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "json")]
    format: OutputFormat,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 扫描磁盘/目录
    Scan {
        path: String,
        #[arg(long, default_value = "50")]
        top: usize,
    },
    /// 提取目录元数据
    Inspect {
        path: String,
        #[arg(long, default_value = "20")]
        samples: usize,
    },
    /// 列出规则
    Rules,
    /// 预览规则匹配（dry-run）
    Preview {
        rule_id: String,
        path: String,
        #[arg(long)]
        scope: Option<String>,
    },
    /// 执行清理
    Execute {
        rule_id: String,
        path: String,
        #[arg(long)]
        scope: Option<String>,
        /// 默认 dry-run（true）；传 --dry-run=false 才真删
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        dry_run: bool,
    },
    /// 生成报告
    Report { path: String },
    /// 权限状态
    Permissions,
    /// 查找重复文件（只检测不删除）
    Dedup {
        path: String,
        /// 忽略小于该字节数的文件
        #[arg(long, default_value = "0")]
        min_size: u64,
    },
    /// 列出已安装软件（读注册表）
    Inventory,
    /// 启动薄 GUI（本地 HTTP 服务 + 浏览器）
    Gui,
    /// 空间核算（逻辑大小 vs 实际分配，找簇对齐浪费）
    Audit {
        path: String,
        /// 列出浪费最多的目录数
        #[arg(long, default_value = "20")]
        top: usize,
    },
    /// 采集快照（增长追踪用）
    Snapshot {
        /// 卷根，如 "C:\"
        #[arg(default_value = "C:\\")]
        path: String,
        /// 大小阈值（字节），低于此的目录不纳入
        #[arg(long, default_value = "1000000")]
        min_size: u64,
        /// 保留快照份数
        #[arg(long, default_value = "10")]
        keep: usize,
    },
    /// 对比最近两份快照，列出增长/缩小最多的目录
    Grow {
        /// 增长/缩小列表各显示多少项
        #[arg(long, default_value = "20")]
        top: usize,
    },
    /// 隔离区管理：查看/清理（默认删超过 7 天的项）
    Quarantine {
        /// 保留天数（超过则删）
        #[arg(long, default_value = "7")]
        days: u64,
        /// 只查看不删
        #[arg(long, default_value_t = false)]
        list: bool,
    },
    /// 软件卸载指导：列出已装软件 + 给官方卸载入口（引导型，不读不执行 UninstallString）
    Uninstall {
        /// 最多列出多少条（0=全部）
        #[arg(long, default_value = "30")]
        top: usize,
    },
    /// 系统级清理指导：WinSxS / Windows.old 的 DISM / cleanmgr 命令（纯建议，不执行）
    System {
        /// 卷根路径
        #[arg(default_value = "C:\\")]
        path: String,
    },
    /// 休眠/页面文件管理：检测系统文件大小 + 给出可逆的 powercfg 命令（纯建议，不执行）
    Hiber {
        /// 卷根路径
        #[arg(default_value = "C:\\")]
        path: String,
    },
    /// 迁移指导：分析适合迁移到其他盘的大缓存目录，给出迁移命令（纯建议，不执行）
    Migrate {
        /// 要分析的卷/路径
        #[arg(default_value = "C:\\")]
        path: String,
        /// 显示多少条建议
        #[arg(long, default_value = "20")]
        top: usize,
        /// 建议迁移到的目标盘符（单个字母）
        #[arg(long, default_value = "D")]
        target: char,
    },
}

#[derive(ValueEnum, Clone)]
enum OutputFormat {
    Json,
    Pretty,
}

/// 统一配置目录：`%APPDATA%\Win-C-Scanary`（非 Windows 用配置目录兜底）。
///
/// undo 日志、隔离区、快照都落在这里，避免随工作目录漂移。
fn config_dir() -> PathBuf {
    dirs::config_dir()
        .map(|b| b.join("Win-C-Scanary"))
        .unwrap_or_else(|| PathBuf::from(".win-c-scanary"))
}

/// 生成"以管理员重开"的提权命令文本；已是管理员时返回 None。
///
/// 纯函数，便于单测。命令仅供复制执行，本工具不自动提权。
fn elevate_command_for(is_admin: bool, exe: &str) -> Option<String> {
    if is_admin {
        None
    } else {
        Some(format!(
            "powershell -Command \"Start-Process -Verb RunAs -FilePath '{}'\"",
            exe
        ))
    }
}

#[cfg(test)]
mod perm_tests {
    use super::elevate_command_for;

    #[test]
    fn admin_needs_no_elevate() {
        assert!(elevate_command_for(true, "C:\\scanary.exe").is_none());
    }

    #[test]
    fn non_admin_gets_runas_command() {
        let cmd = elevate_command_for(false, "C:\\scanary.exe").unwrap();
        assert!(cmd.contains("Start-Process"));
        assert!(cmd.contains("-Verb RunAs"));
        assert!(cmd.contains("C:\\scanary.exe"));
    }
}

/// 递归遍历目录树，为每个目录节点按规则打标（填充 rule_id）。
fn tag_rules(node: &mut wcs_scanner::Node, rules: &[wcs_scaffold::Rule]) {
    if node.is_dir {
        if let Some(rid) = wcs_scaffold::detect_for(rules, std::path::Path::new(&node.path)) {
            node.rule_id = Some(rid);
        }
    }
    for c in &mut node.children {
        tag_rules(c, rules);
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // 规则目录解析顺序：显式 --rules-dir > exe 同级的 rules/ > 当前目录的 rules/。
    // 优先 exe 同级，避免"换目录运行 scanary 就找不到规则"（Agent 常见坑）。
    let rules_dir = cli.rules_dir.clone().unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("rules")))
            .filter(|d| d.exists())
            .unwrap_or_else(|| PathBuf::from("rules"))
    });

    match &cli.command {
        Commands::Scan { path, top } => {
            let (node, stats) = wcs_scanner::scan_with_stats(path, Default::default(), |_| {})?;
            match cli.format {
                OutputFormat::Json => {
                    let out = serde_json::json!({
                        "root": { "name": node.name, "path": node.path, "size": node.size, "file_count": node.file_count },
                        "top_dirs": node.children.iter().take(*top).map(|c| serde_json::json!({ "name": c.name, "size": c.size, "is_dir": c.is_dir })).collect::<Vec<_>>(),
                        "scan_stats": stats,
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Pretty => {
                    println!("扫描: {}", path);
                    println!("模式: {:?}  文件数: {}  大小: {:.2} GB", stats.mode, stats.files_seen, stats.bytes_seen as f64 / 1e9);
                    for c in node.children.iter().take(*top) {
                        println!("  {:<40} {:>10.2} GB", c.name, c.size as f64 / 1e9);
                    }
                }
            }
        }
        Commands::Inspect { path, samples } => {
            let md = wcs_scanner::inspect(path, *samples)?;
            if md.truncated {
                eprintln!(
                    "提示：该目录文件数超过 {} 万上限，结果已截断（size/file_count 为部分值）。大范围统计请用 scan。",
                    wcs_scanner::walk::INSPECT_FILE_LIMIT / 10_000
                );
            }
            println!("{}", serde_json::to_string_pretty(&md)?);
        }
        Commands::Rules => {
            let rules = wcs_scaffold::load_dir(&rules_dir)?;
            if rules.is_empty() {
                eprintln!(
                    "提示：规则目录 {} 为空或不存在。请用 --rules-dir <绝对路径> 指定规则目录（默认查找 exe 同级 rules/ 或当前目录 rules/）。",
                    rules_dir.display()
                );
            }
            println!("{}", serde_json::to_string_pretty(&rules)?);
        }
        Commands::Preview { rule_id, path, scope } => {
            let rules = wcs_scaffold::load_dir(&rules_dir)?;
            let rule = rules.iter().find(|r| &r.id == rule_id);
            match rule {
                Some(r) => {
                    let scope_match = match scope {
                        Some(sid) => r.scopes.iter().find(|s| &s.id == sid),
                        None => r.scopes.first(),
                    };
                    match scope_match {
                        Some(s) => {
                            let matched = wcs_scaffold::match_scope(s, std::path::Path::new(path))?;
                            let out = serde_json::json!({
                                "rule_id": rule_id,
                                "root_path": path,
                                "matched_count": matched.len(),
                                "scope": s.id,
                            });
                            println!("{}", serde_json::to_string_pretty(&out)?);
                        }
                        None => eprintln!("scope 不存在"),
                    }
                }
                None => eprintln!("规则不存在: {}", rule_id),
            }
        }
        Commands::Execute { rule_id, path, scope, dry_run } => {
            let rules = wcs_scaffold::load_dir(&rules_dir)?;
            let rule = rules.iter().find(|r| &r.id == rule_id);
            match rule {
                Some(r) => {
                    let scope_match = match scope {
                        Some(sid) => r.scopes.iter().find(|s| &s.id == sid),
                        None => r.scopes.first(),
                    };
                    if let Some(s) = scope_match {
                        // 先对用户传入的根做 guard 校验：命中保护路径直接拒绝
                        let guard_cfg = wcs_guard::GuardConfig {
                            allowed_roots: vec![std::path::PathBuf::from("C:\\")],
                            ..Default::default()
                        };
                        let root_path = std::path::Path::new(path);
                        let g = wcs_guard::check_path(root_path, &guard_cfg);
                        if g.verdict == wcs_guard::Verdict::Block {
                            let out = serde_json::json!({
                                "rule_id": rule_id,
                                "root_path": path,
                                "executed": false,
                                "blocked": [[path, g.reason.clone().unwrap_or_default()]],
                                "message": "路径被 guard 拦截，拒绝执行",
                            });
                            println!("{}", serde_json::to_string_pretty(&out)?);
                            return Ok(());
                        }
                        let matched = wcs_scaffold::match_scope(s, root_path)?;
                        let action = match s.mode {
                            wcs_scaffold::Mode::Recycle => wcs_executor::Action::Recycle,
                            wcs_scaffold::Mode::Quarantine => wcs_executor::Action::Quarantine,
                            wcs_scaffold::Mode::Delete => wcs_executor::Action::Delete,
                            wcs_scaffold::Mode::SystemCmd => wcs_executor::Action::SystemCmd,
                        };
                        let plan = wcs_executor::Plan {
                            action,
                            paths: matched,
                            reason: format!("rule={} scope={}", rule_id, s.id),
                            granularity: match s.recycle_granularity {
                                wcs_scaffold::RecycleGranularity::Directory => wcs_executor::Granularity::Directory,
                                _ => wcs_executor::Granularity::File,
                            },
                            system_cmd: None,
                        };
                        let undo = cli.undo_log.clone().unwrap_or_else(|| config_dir().join("undo.jsonl"));
                        let quar = cli.quarantine_root.clone().unwrap_or_else(|| config_dir().join("quarantine"));
                        let result = wcs_executor::execute(&plan, &Default::default(), *dry_run, &undo, &quar)?;
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    }
                }
                None => eprintln!("规则不存在: {}", rule_id),
            }
        }
        Commands::Report { path } => {
            // 解析盘符（如 "C:\" → 'C'）
            let vol = path
                .chars()
                .next()
                .filter(|c| c.is_ascii_alphabetic())
                .unwrap_or('C');
            let disk = wcs_scanner::disk_space(vol);
            let rules = wcs_scaffold::load_dir(&rules_dir)?;
            // 优先 MFT 直读（卷根时秒级），失败自动降级 walk；
            // 用 indexed 版本拿目录索引，分级量按 scope 前缀精确累加（T-REPORT-1）
            let (mut node, _stats, dir_index) =
                wcs_scanner::scan_with_stats_indexed(path, Default::default(), |_| {})?;
            // 仅在没有目录索引（如降级 walk）时才给节点打规则标签——分级走 index 时不需要，
            // 且 tag_rules 对每个节点重建 globset，全盘会极慢。
            if dir_index.is_none() {
                tag_rules(&mut node, &rules);
            }
            let report = wcs_report::build_with_index(&node, None, disk, &rules, dir_index.as_ref());
            match cli.format {
                OutputFormat::Json => println!("{}", wcs_report::to_json(&report)?),
                OutputFormat::Pretty => println!("{}", wcs_report::to_pretty(&report)),
            }
        }
        Commands::Permissions => {
            let is_admin = wcs_scanner::is_admin();
            let can_mft = wcs_scanner::can_use_mft('C');
            let advice = if can_mft {
                "管理员权限已就绪，可使用 MFT 秒级扫描。"
            } else if is_admin {
                "已是管理员，但当前卷不支持 MFT 直读。"
            } else {
                "当前为普通权限。全盘秒级扫描（MFT）和系统级清理需管理员权限。"
            };
            // 非管理员时，给出如何提权重开的可复制命令（不自动执行，由用户自行运行）。
            let exe = std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "scanary.exe".to_string());
            let elevate_command = elevate_command_for(is_admin, &exe);
            match cli.format {
                OutputFormat::Json => {
                    let out = serde_json::json!({
                        "is_admin": is_admin,
                        "can_use_mft": can_mft,
                        "can_clean_system": is_admin,
                        "advice": advice,
                        "elevate_command": elevate_command,
                        "note": "提权命令仅供复制执行（本工具不自动提权）；以管理员重开后 MFT/系统级操作才可用。",
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                OutputFormat::Pretty => {
                    println!("管理员: {}", if is_admin { "是" } else { "否" });
                    println!("可 MFT 直读: {}", if can_mft { "是" } else { "否" });
                    println!("可执行系统级操作: {}", if is_admin { "是" } else { "否" });
                    println!("建议: {}", advice);
                    if let Some(cmd) = &elevate_command {
                        println!("\n提权方式（复制到终端执行，本工具不自动提权）:");
                        println!("  {}", cmd);
                        println!("  说明：以管理员重开后，MFT 秒级全盘扫描与系统级操作才可用。");
                    }
                }
            }
        }
        Commands::Dedup { path, min_size } => {
            let result = wcs_dedup::find_duplicates(std::path::Path::new(path), *min_size)?;
            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Pretty => {
                    println!(
                        "重复文件检测: {}  文件数: {}  重复组: {}  可回收: {:.2} MB",
                        path,
                        result.files_scanned,
                        result.groups.len(),
                        result.total_waste_bytes as f64 / 1e6
                    );
                    for g in result.groups.iter().take(20) {
                        println!(
                            "  [{} 字节 × {} 份] 可回收 {:.2} MB",
                            g.size,
                            g.paths.len(),
                            g.waste_bytes as f64 / 1e6
                        );
                        for p in &g.paths {
                            println!("      {}", p.display());
                        }
                    }
                }
            }
        }
        Commands::Audit { path, top } => {
            let rules = wcs_scaffold::load_dir(&rules_dir).unwrap_or_default();
            let _ = rules; // audit 不需要规则，仅为保持加载一致性
            let (_node, _stats, dir_index) =
                wcs_scanner::scan_with_stats_indexed(path, Default::default(), |_| {})?;
            match dir_index {
                Some(idx) => {
                    let audit = wcs_report::space_audit(&idx, *top);
                    match cli.format {
                        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&audit)?),
                        OutputFormat::Pretty => {
                            println!("=== 空间核算: {} ===", path);
                            println!(
                                "逻辑大小: {:.2} GB  实际分配: {:.2} GB  簇对齐浪费: {:.2} GB ({:.2}%)",
                                audit.logical_bytes as f64 / 1e9,
                                audit.allocated_bytes as f64 / 1e9,
                                audit.waste_bytes as f64 / 1e9,
                                audit.waste_ratio * 100.0,
                            );
                            println!("\n浪费集中出现的目录（按本层新增浪费排序）:");
                            for d in &audit.top_waste_dirs {
                                println!(
                                    "  {:<52} 本层 {:>7.2} MB / 累计 {:>8.2} MB",
                                    d.path,
                                    d.self_waste as f64 / 1e6,
                                    d.waste_bytes as f64 / 1e6,
                                );
                            }
                        }
                    }
                }
                None => {
                    println!("空间核算需要 MFT 直读（管理员 + NTFS 卷）。当前不可用。");
                }
            }
        }
        Commands::Snapshot { path, min_size, keep } => {
            let (_node, _stats, dir_index) =
                wcs_scanner::scan_with_stats_indexed(path, Default::default(), |_| {})?;
            match dir_index {
                Some(idx) => {
                    let vol = path.chars().take(2).collect::<String>();
                    let snap = wcs_growth::take_snapshot(&idx, &vol, *min_size);
                    let dir = wcs_growth::snapshot_dir();
                    let saved = wcs_growth::save_snapshot(&snap, &dir, *keep)?;
                    println!(
                        "快照已保存: {}（{} 个目录，阈值 {} MB）",
                        saved.display(),
                        snap.entries.len(),
                        *min_size / 1_000_000
                    );
                }
                None => println!("快照需要 MFT 直读（管理员 + NTFS 卷）。当前不可用。"),
            }
        }
        Commands::Grow { top } => {
            let dir = wcs_growth::snapshot_dir();
            let files = wcs_growth::list_snapshots(&dir)?;
            if files.len() < 2 {
                println!(
                    "需要至少 2 份快照才能对比（当前 {} 份）。先运行：scanary snapshot",
                    files.len()
                );
                return Ok(());
            }
            let old = wcs_growth::load_snapshot(&files[files.len() - 2])?;
            let new = wcs_growth::load_snapshot(&files[files.len() - 1])?;
            let rep = wcs_growth::diff(&old, &new, *top);
            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rep)?),
                OutputFormat::Pretty => {
                    println!("=== 增长追踪 ===");
                    println!("对比: {} → {}", rep.from_time, rep.to_time);
                    println!(
                        "总量: {:.2} GB → {:.2} GB（{:+.2} GB）",
                        rep.total_old as f64 / 1e9,
                        rep.total_new as f64 / 1e9,
                        rep.total_delta as f64 / 1e9
                    );
                    println!("\n增长集中出现的目录（「本层」= 本目录相对父层的额外增长，不会超过累计）:");
                    for g in &rep.top_growth {
                        println!(
                            "  本层 {:>+8} MB / 累计 {:>+9} MB  {:<50} ({:.1} → {:.1} MB)",
                            g.self_delta / 1_000_000,
                            g.delta / 1_000_000,
                            g.path,
                            g.old_size as f64 / 1e6,
                            g.new_size as f64 / 1e6,
                        );
                    }
                    if !rep.top_shrink.is_empty() {
                        println!("\n缩小/删除最多的目录:");
                        for g in &rep.top_shrink {
                            println!(
                                "  {:>+9} MB  {:<55} ({:.1} → {:.1} MB)",
                                g.delta / 1_000_000,
                                g.path,
                                g.old_size as f64 / 1e6,
                                g.new_size as f64 / 1e6,
                            );
                        }
                    }
                }
            }
        }
        Commands::Quarantine { days, list } => {
            let root = cli
                .quarantine_root
                .clone()
                .unwrap_or_else(|| config_dir().join("quarantine"));
            if *list {
                let mut items: Vec<(String, u64)> = Vec::new();
                if root.exists() {
                    for e in std::fs::read_dir(&root)?.flatten() {
                        if let Ok(md) = e.metadata() {
                            if md.is_file() {
                                items.push((e.file_name().to_string_lossy().to_string(), md.len()));
                            }
                        }
                    }
                }
                items.sort_by(|a, b| b.1.cmp(&a.1));
                println!("隔离区: {}", root.display());
                println!("共 {} 项", items.len());
                for (name, size) in &items {
                    println!("  {:<50} {:>10} KB", name, size / 1024);
                }
            } else {
                let removed = wcs_executor::prune_quarantine(&root, *days)?;
                println!(
                    "已清理隔离区 {}：删除 {} 项（保留最近 {} 天）",
                    root.display(),
                    removed,
                    days
                );
            }
        }
        Commands::Uninstall { top } => {
            let apps = wcs_inventory::list_apps()?;
            let advice = uninstall::analyze(apps, *top);
            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&advice)?),
                OutputFormat::Pretty => {
                    println!("=== 软件卸载指导 ===");
                    println!("已安装软件: {} 项（列出前 {} 项，按估算大小降序）\n", advice.app_count, advice.apps.len());
                    for a in &advice.apps {
                        let ver = a.version.as_deref().unwrap_or("-");
                        let size = a
                            .estimated_size_mb
                            .map(|m| format!("{} MB", m))
                            .unwrap_or_else(|| "-".into());
                        let who = if a.per_user { " [user]" } else { "" };
                        println!("  {:<45} {:<12} {:>10}{}", a.name, ver, size, who);
                    }
                    println!("\n官方卸载入口（本工具不执行，请自行操作）：");
                    for g in &advice.guides {
                        println!("  [{}] {}", g.purpose, g.entry);
                    }
                    println!("\n提示:");
                    for n in &advice.notes {
                        println!("  - {}", n);
                    }
                }
            }
        }
        Commands::System { path } => {
            let is_admin = wcs_scanner::is_admin();
            let advice = system::analyze(path, is_admin);
            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&advice)?),
                OutputFormat::Pretty => {
                    println!("=== 系统级清理指导: {} ===", advice.volume);
                    println!("\n⚠️  最高风险区域：本工具只给命令，绝不执行任何清理。\n");
                    println!("检测到的系统项:");
                    for it in &advice.items {
                        let size = match it.size_bytes {
                            Some(b) => format!("{:.2} GB", b as f64 / 1e9),
                            None => "（不存在或无权读取）".into(),
                        };
                        let reliable = if it.size_reliable { "" } else { "（大小虚高）" };
                        println!("  {:<28} {}{}", it.name, size, reliable);
                        println!("      {}", it.note);
                    }
                    println!(
                        "\n当前权限: {}",
                        if advice.is_admin { "管理员" } else { "非管理员（DISM/cleanmgr 系统模式需管理员）" }
                    );
                    println!("\n建议命令（仅供参考，本工具不执行）：");
                    for c in &advice.commands {
                        let tag = if c.destructive { "破坏性/不可逆" } else { "安全" };
                        println!("  [{}]", tag);
                        println!("    用途: {}", c.purpose);
                        println!("    命令: {}", c.command);
                        if let Some(w) = &c.warning {
                            println!("    {}", w);
                        }
                    }
                    println!("\n提示:");
                    for n in &advice.notes {
                        println!("  - {}", n);
                    }
                }
            }
        }
        Commands::Hiber { path } => {
            let is_admin = wcs_scanner::is_admin();
            let advice = hiber::analyze(path, is_admin);
            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&advice)?),
                OutputFormat::Pretty => {
                    println!("=== 休眠/页面文件管理: {} ===", advice.volume);
                    println!("\n系统文件:");
                    for f in &advice.files {
                        match f.size_bytes {
                            Some(b) => println!("  {:<45} {:>8.2} GB", f.path, b as f64 / 1e9),
                            None => println!("  {:<45} {}", f.path, "（不存在或无权读取）"),
                        }
                    }
                    println!(
                        "\n休眠已启用: {}{}",
                        if advice.state.hibernation_enabled { "是" } else { "否" },
                        if advice.is_admin { "" } else { "（当前非管理员）" }
                    );
                    println!("\n建议命令（仅供参考，本工具不执行）：");
                    for c in &advice.commands {
                        let tags = format!(
                            "{}{}",
                            if c.reversible { "可逆 " } else { "" },
                            if c.needs_admin { "需管理员" } else { "" }
                        );
                        println!("  [{}]", tags.trim());
                        println!("    用途: {}", c.purpose);
                        println!("    命令: {}", c.command);
                    }
                    println!("\n提示:");
                    for n in &advice.notes {
                        println!("  - {}", n);
                    }
                }
            }
        }
        Commands::Migrate { path, top, target } => {
            let rules = wcs_scaffold::load_dir(&rules_dir)?;
            let (_node, _stats, dir_index) =
                wcs_scanner::scan_with_stats_indexed(path, Default::default(), |_| {})?;
            match dir_index {
                Some(idx) => {
                    // 卷名：取路径前两字符（如 "C:"）
                    let vol: String = path.chars().take(2).collect();
                    let advice = wcs_report::migrate::analyze(&idx, &rules, &vol, *target, *top);
                    match cli.format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&advice)?)
                        }
                        OutputFormat::Pretty => {
                            println!("=== 迁移指导: {} ===", advice.volume);
                            println!("{}", advice.note);
                            if advice.suggestions.is_empty() {
                                println!("\n未发现适合迁移的缓存目录（可能本机无缓存，或需要管理员权限做 MFT 扫描）。");
                            } else {
                                println!(
                                    "\n共 {} 条建议，可迁移总量 {:.2} GB：\n",
                                    advice.suggestions.len(),
                                    advice.total_bytes as f64 / 1e9
                                );
                                for s in &advice.suggestions {
                                    println!(
                                        "[{:.2} GB] {} （{}）",
                                        s.size_bytes as f64 / 1e9,
                                        s.rule_name,
                                        s.source_path
                                    );
                                    println!("    方式: {:?}", s.method);
                                    println!("    命令: {}", s.command);
                                    println!("    说明: {}\n", s.note);
                                }
                            }
                        }
                    }
                }
                None => {
                    println!("迁移指导需要 MFT 直读（管理员 + NTFS 卷）。当前不可用。");
                }
            }
        }
        Commands::Gui => {
            let cfg = gui::GuiConfig {
                rules_dir: rules_dir.clone(),
                undo_log: cli.undo_log.clone().unwrap_or_else(|| config_dir().join("undo.jsonl")),
                quarantine_root: cli.quarantine_root.clone().unwrap_or_else(|| config_dir().join("quarantine")),
            };
            gui::run(cfg)?;
        }
        Commands::Inventory => {
            let apps = wcs_inventory::list_apps()?;
            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&apps)?),
                OutputFormat::Pretty => {
                    println!("已安装软件: {} 项", apps.len());
                    for a in &apps {
                        let ver = a.version.as_deref().unwrap_or("-");
                        let size = a
                            .estimated_size_mb
                            .map(|m| format!("{} MB", m))
                            .unwrap_or_else(|| "-".into());
                        let who = if a.per_user { " [user]" } else { "" };
                        println!("  {:<45} {:<15} {:>10}{}", a.name, ver, size, who);
                    }
                }
            }
        }
    }
    Ok(())
}

