//! wcs-cli — 薄 GUI（阶段 3）。
//!
//! 设计依据：DESIGN.md 决策 5（G3：先浏览器，本地 HTTP 服务，可升级 WebView2/Tauri）、
//! 决策 6（H1：GUI 不放 AI，复杂项引导去 Agent）。
//!
//! 形态：`scanary gui` 起本地 HTTP 服务 → 打印 URL → 自动打开浏览器。
//! 后端是薄封装，直接转调各库函数（不 shell out）。
//! 前端为内嵌静态资源（`include_str!`），不依赖外部文件。

#[path = "gui/log.rs"]
mod log;

use anyhow::{Context, Result};
use log::LogStore;
use std::path::PathBuf;
use std::sync::Arc;
use tiny_http::{Header, Method, Response, Server};

/// GUI 运行配置（从 CLI 全局参数透传）
pub struct GuiConfig {
    pub rules_dir: PathBuf,
    pub undo_log: PathBuf,
    pub quarantine_root: PathBuf,
}


/// 启动 GUI：绑定随机端口 → 打印 URL → 尝试打开浏览器 → 进入请求循环。
pub fn run(cfg: GuiConfig) -> Result<()> {
    // 绑定 127.0.0.1:0 → 由内核分配随机可用端口，避免固定端口冲突
    let server = Server::http("127.0.0.1:0")
        .map_err(|e| anyhow::anyhow!("启动 HTTP 服务失败: {}", e))?;
    let addr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow::anyhow!("无法获取监听地址"))?;
    let url = format!("http://127.0.0.1:{}", addr.port());

    println!("Win-C-Scanary GUI 已启动");
    println!("请在浏览器打开: {}", url);
    println!("（按 Ctrl+C 退出）");

    // 初始化日志：内存环形缓冲 + tracing 捕获（warn/error）
    let logs = LogStore::new();
    log::init_tracing(logs.clone());
    logs.api(format!("GUI 启动，监听 {}", url));

    open_browser(&url);

    let ctx = Arc::new(cfg);
    let logs = Arc::new(logs);
    loop {
        let request = match server.recv() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("接收请求失败: {}", e);
                continue;
            }
        };
        // 单线程处理：请求很快（扫描是唯一重活，且用户逐次触发）
        if let Err(e) = handle(request, &ctx, &logs) {
            logs.push("ERROR", "api", format!("处理请求失败: {:#}", e));
        }
    }
}

/// 尝试用系统默认浏览器打开 URL（失败不致命，用户可手动打开）。
fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        // start 是 cmd 内建命令；用 cmd /C start 打开默认浏览器
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// 构造带 UTF-8 的响应头
fn html_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap()
}
fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap()
}

/// 读取请求体（POST 的 JSON）
fn read_body(request: &mut tiny_http::Request) -> Result<String> {
    let mut body = String::new();
    request
        .as_reader()
        .read_to_string(&mut body)
        .context("读取请求体失败")?;
    Ok(body)
}

/// 路由分发
fn handle(request: tiny_http::Request, cfg: &GuiConfig, logs: &LogStore) -> Result<()> {
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("").to_string();
    let t0 = std::time::Instant::now();

    // 记录 API 访问日志（请求前先记一条，便于追踪"卡住"的请求）
    logs.api(format!("{} {} …", method, path));
    let result = route(request, cfg, logs, &method, &path);
    let ms = t0.elapsed().as_millis();
    match &result {
        Ok(_) => logs.api(format!("{} {} — {}ms", method, path, ms)),
        Err(e) => logs.push("ERROR", "api", format!("{} {} — {}ms — {:#}", method, path, ms, e)),
    }
    result
}

/// 实际路由
fn route(
    mut request: tiny_http::Request,
    cfg: &GuiConfig,
    logs: &LogStore,
    method: &Method,
    path: &str,
) -> Result<()> {
    match (method.clone(), path) {
        // 采集快照
        (Method::Post, "/api/snapshot") => {
            let out = api_snapshot()?;
            respond_json(request, &out)?;
        }
        // 快照列表
        (Method::Get, "/api/snapshot/list") => {
            let dir = wcs_growth::snapshot_dir();
            let files = wcs_growth::list_snapshots(&dir)?;
            let list: Vec<serde_json::Value> = files
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "file": f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        "bytes": std::fs::metadata(f).map(|m| m.len()).unwrap_or(0),
                    })
                })
                .collect();
            respond_json(request, &serde_json::json!({ "count": list.len(), "snapshots": list }))?;
        }
        // 清空快照
        (Method::Post, "/api/snapshot/clear") => {
            let dir = wcs_growth::snapshot_dir();
            let n = wcs_growth::clear_snapshots(&dir)?;
            logs.api(format!("清空 {} 份快照", n));
            respond_json(request, &serde_json::json!({ "cleared": n }))?;
        }
        // 增长对比
        (Method::Get, "/api/grow") => {
            let out = api_grow()?;
            respond_json(request, &out)?;
        }
        // 日志（GET 列表）
        (Method::Get, "/api/logs") => {
            let entries = logs.snapshot();
            respond_json(request, &entries)?;
        }
        // 日志清空
        (Method::Post, "/api/logs/clear") => {
            logs.clear();
            logs.api("日志已清空");
            respond_json(request, &serde_json::json!({"ok": true}))?;
        }
        // 静态首页
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            let resp = Response::from_string(INDEX_HTML).with_header(html_header());
            request.respond(resp)?;
        }
        // 权限状态
        (Method::Get, "/api/permissions") => {
            let is_admin = wcs_scanner::is_admin();
            let can_mft = wcs_scanner::can_use_mft('C');
            let out = serde_json::json!({
                "is_admin": is_admin,
                "can_use_mft": can_mft,
                "advice": if can_mft { "管理员权限已就绪，可使用 MFT 秒级扫描。" }
                          else { "当前为普通权限，全盘扫描将降级为 walk（较慢）。" },
            });
            respond_json(request, &out)?;
        }
        // 健康报告（含健康评分 + 大目录 + 三色分级）
        (Method::Get, "/api/health") => {
            let out = api_health(cfg)?;
            respond_json(request, &out)?;
        }
        // 规则列表
        (Method::Get, "/api/rules") => {
            let rules = wcs_scaffold::load_dir(&cfg.rules_dir)?;
            respond_json(request, &rules)?;
        }
        // 分级明细（每条规则的命中量与可清理字节）
        (Method::Get, "/api/grade") => {
            let out = api_grade(cfg)?;
            respond_json(request, &out)?;
        }
        // 扫描（POST { path, top }）
        (Method::Post, "/api/scan") => {
            let body = read_body(&mut request)?;
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("C:\\");
            let top = v.get("top").and_then(|x| x.as_u64()).unwrap_or(30) as usize;
            let out = api_scan(path, top)?;
            respond_json(request, &out)?;
        }
        // 预览（POST { rule_id, path, scope? }）
        (Method::Post, "/api/preview") => {
            let body = read_body(&mut request)?;
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let out = api_preview(cfg, &v)?;
            respond_json(request, &out)?;
        }
        // 执行（POST { rule_id, path, scope?, dry_run, confirm }）
        (Method::Post, "/api/execute") => {
            let body = read_body(&mut request)?;
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let out = api_execute(cfg, &v)?;
            respond_json(request, &out)?;
        }
        _ => {
            let resp = Response::from_string("Not Found").with_status_code(404);
            request.respond(resp)?;
        }
    }
    Ok(())
}

fn respond_json<T: serde::Serialize>(request: tiny_http::Request, value: &T) -> Result<()> {
    let body = serde_json::to_string(value)?;
    let resp = Response::from_string(body).with_header(json_header());
    request.respond(resp)?;
    Ok(())
}

/// POST /api/snapshot — 采集当前快照并保存。
fn api_snapshot() -> Result<serde_json::Value> {
    let (_node, _stats, dir_index) =
        wcs_scanner::scan_with_stats_indexed("C:\\", Default::default(), |_| {})
            .context("扫描 C: 失败")?;
    let Some(idx) = dir_index else {
        return Ok(serde_json::json!({
            "saved": false,
            "message": "采集快照需要 MFT 直读（管理员 + NTFS 卷）。当前不可用。",
        }));
    };
    let snap = wcs_growth::take_snapshot(&idx, "C:", wcs_growth::DEFAULT_MIN_SIZE);
    let dir = wcs_growth::snapshot_dir();
    let saved = wcs_growth::save_snapshot(&snap, &dir, wcs_growth::DEFAULT_KEEP)?;
    Ok(serde_json::json!({
        "saved": true,
        "entries": snap.entries.len(),
        "file": saved.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    }))
}

/// GET /api/grow — 对比最近两份快照。
fn api_grow() -> Result<serde_json::Value> {
    let dir = wcs_growth::snapshot_dir();
    let files = wcs_growth::list_snapshots(&dir)?;
    if files.len() < 2 {
        return Ok(serde_json::json!({
            "ready": false,
            "count": files.len(),
            "message": "需要至少 2 份快照。请先点「采集快照」。",
        }));
    }
    let old = wcs_growth::load_snapshot(&files[files.len() - 2])?;
    let new = wcs_growth::load_snapshot(&files[files.len() - 1])?;
    let rep = wcs_growth::diff(&old, &new, 20);
    let mut v = serde_json::to_value(&rep)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("ready".into(), serde_json::json!(true));
        o.insert("count".into(), serde_json::json!(files.len()));
    }
    Ok(v)
}

/// GET /api/health — 生成完整报告（健康评分 + 摘要 + 大目录）
fn api_health(cfg: &GuiConfig) -> Result<serde_json::Value> {
    let disk = wcs_scanner::disk_space('C');
    let rules = wcs_scaffold::load_dir(&cfg.rules_dir)?;
    let (mut node, _stats, dir_index) =
        wcs_scanner::scan_with_stats_indexed("C:\\", Default::default(), |_| {})
            .context("扫描 C: 失败")?;
    // 有目录索引时分级走 index，无需对全盘节点打标签（tag_rules 每节点重建 globset，极慢）
    if dir_index.is_none() {
        tag_rules(&mut node, &rules);
    }
    let report = wcs_report::build_with_index(&node, None, disk, &rules, dir_index.as_ref());
    let mut v = serde_json::to_value(&report)?;
    // 空间核算（复用本次扫描的 index）
    if let Some(idx) = dir_index.as_ref() {
        let audit = wcs_report::space_audit(idx, 5);
        if let Some(obj) = v.as_object_mut() {
            obj.insert("space_audit".into(), serde_json::to_value(&audit)?);
        }
    }
    Ok(v)
}

/// GET /api/grade — 每条规则的分级明细（可清理字节 + 命中目录数）。
fn api_grade(cfg: &GuiConfig) -> Result<serde_json::Value> {
    let rules = wcs_scaffold::load_dir(&cfg.rules_dir)?;
    let (_node, _stats, dir_index) =
        wcs_scanner::scan_with_stats_indexed("C:\\", Default::default(), |_| {})
            .context("扫描 C: 失败")?;
    let detail = match dir_index.as_ref() {
        Some(idx) => wcs_report::grade_detail(idx, &rules),
        None => Vec::new(),
    };
    Ok(serde_json::to_value(&detail)?)
}

/// POST /api/scan — 扫描指定路径并返回 top 子目录
fn api_scan(path: &str, top: usize) -> Result<serde_json::Value> {
    let (node, stats) = wcs_scanner::scan_with_stats(path, Default::default(), |_| {})
        .with_context(|| format!("扫描 {} 失败", path))?;
    let top_dirs: Vec<serde_json::Value> = node
        .children
        .iter()
        .take(top)
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "size": c.size,
                "is_dir": c.is_dir,
                "file_count": c.file_count,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "root": { "name": node.name, "path": node.path, "size": node.size, "file_count": node.file_count },
        "top_dirs": top_dirs,
        "scan_stats": stats,
    }))
}

/// POST /api/preview — 预览某规则命中（dry-run，不删）
fn api_preview(cfg: &GuiConfig, v: &serde_json::Value) -> Result<serde_json::Value> {
    let rule_id = v.get("rule_id").and_then(|x| x.as_str()).unwrap_or("");
    let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
    let scope = v.get("scope").and_then(|x| x.as_str());
    let rules = wcs_scaffold::load_dir(&cfg.rules_dir)?;
    let Some(rule) = rules.iter().find(|r| r.id == rule_id) else {
        return Ok(serde_json::json!({ "error": format!("规则不存在: {}", rule_id) }));
    };
    let scope_match = match scope {
        Some(sid) => rule.scopes.iter().find(|s| s.id == sid),
        None => rule.scopes.first(),
    };
    let Some(s) = scope_match else {
        return Ok(serde_json::json!({ "error": "scope 不存在" }));
    };
    let matched = wcs_scaffold::match_scope(s, std::path::Path::new(path))?;
    Ok(serde_json::json!({
        "rule_id": rule_id,
        "rule_name": rule.name,
        "risk": rule.risk,
        "scope": s.id,
        "scope_label": s.label,
        "root_path": path,
        "matched_count": matched.len(),
        "matched_paths": matched.iter().take(200).map(|p| p.display().to_string()).collect::<Vec<_>>(),
    }))
}

/// POST /api/execute — 执行清理。
/// 安全约束：RED 项拒绝；dry_run 默认真；真删需 confirm=true。
fn api_execute(cfg: &GuiConfig, v: &serde_json::Value) -> Result<serde_json::Value> {
    let rule_id = v.get("rule_id").and_then(|x| x.as_str()).unwrap_or("");
    let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
    let scope = v.get("scope").and_then(|x| x.as_str());
    let dry_run = v.get("dry_run").and_then(|x| x.as_bool()).unwrap_or(true);
    let confirm = v.get("confirm").and_then(|x| x.as_bool()).unwrap_or(false);

    let rules = wcs_scaffold::load_dir(&cfg.rules_dir)?;
    let Some(rule) = rules.iter().find(|r| r.id == rule_id) else {
        return Ok(serde_json::json!({ "error": format!("规则不存在: {}", rule_id) }));
    };
    // 安全策略：RED 拒绝 + 真删需确认（抽为纯函数，便于测试）
    if let Err(msg) = can_execute(rule.risk, dry_run, confirm) {
        return Ok(serde_json::json!({ "executed": false, "message": msg }));
    }
    let scope_match = match scope {
        Some(sid) => rule.scopes.iter().find(|s| s.id == sid),
        None => rule.scopes.first(),
    };
    let Some(s) = scope_match else {
        return Ok(serde_json::json!({ "error": "scope 不存在" }));
    };

    // guard 校验根路径
    let guard_cfg = wcs_guard::GuardConfig {
        allowed_roots: vec![std::path::PathBuf::from("C:\\")],
        ..Default::default()
    };
    let root_path = std::path::Path::new(path);
    let g = wcs_guard::check_path(root_path, &guard_cfg);
    if g.verdict == wcs_guard::Verdict::Block {
        return Ok(serde_json::json!({
            "executed": false,
            "message": format!("路径被 guard 拦截: {}", g.reason.clone().unwrap_or_default()),
        }));
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
    let result = wcs_executor::execute(
        &plan,
        &Default::default(),
        dry_run,
        &cfg.undo_log,
        &cfg.quarantine_root,
    )?;
    Ok(serde_json::to_value(&result)?)
}

/// 递归为目录节点按规则打标（供 report 分级）
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

/// 执行安全策略（纯函数，便于测试）：
/// - RED（high）项一律拒绝（引导去 Agent）
/// - 非 dry-run（真删）必须显式 confirm
pub fn can_execute(risk: wcs_scaffold::Risk, dry_run: bool, confirm: bool) -> Result<(), String> {
    if risk == wcs_scaffold::Risk::High {
        return Err("高风险（RED）项需用 Agent 分析，GUI 不提供清理。".to_string());
    }
    if !dry_run && !confirm {
        return Err("真实执行需 confirm=true。".to_string());
    }
    Ok(())
}

/// 内嵌前端页面
const INDEX_HTML: &str = include_str!("gui/index.html");

#[cfg(test)]
mod tests {
    use super::can_execute;
    use wcs_scaffold::Risk;

    #[test]
    fn red_is_always_rejected() {
        // RED 项：无论 dry_run/confirm 如何都拒绝
        assert!(can_execute(Risk::High, true, false).is_err());
        assert!(can_execute(Risk::High, false, true).is_err());
    }

    #[test]
    fn dry_run_allowed_without_confirm() {
        // GREEN/YELLOW 的 dry-run 允许（预览）
        assert!(can_execute(Risk::Low, true, false).is_ok());
        assert!(can_execute(Risk::Medium, true, false).is_ok());
    }

    #[test]
    fn real_delete_requires_confirm() {
        // 真删（dry_run=false）无 confirm → 拒绝
        assert!(can_execute(Risk::Low, false, false).is_err());
        // 有 confirm → 允许
        assert!(can_execute(Risk::Low, false, true).is_ok());
    }
}
