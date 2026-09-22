//! wcs-report 单元测试。

use wcs_report::{health_score, Report};
use wcs_scanner::Node;
use wcs_scaffold::{Risk, Rule, Scope};

fn mk_rule(id: &str, risk: Risk) -> Rule {
    Rule {
        id: id.into(),
        name: id.into(),
        homepage: None,
        risk,
        disclaimer: "test".into(),
        detect: vec![],
        matcher: Default::default(),
        scopes: vec![Scope {
            id: "s1".into(),
            label: "S1".into(),
            glob: "**/*".into(),
            mode: wcs_scaffold::Mode::Recycle,
            prompt: None,
            category: None,
            variant: None,
            recycle_granularity: Default::default(),
        }],
    }
}

fn mk_node(name: &str, size: u64, rule_id: Option<&str>) -> Node {
    Node {
        name: name.into(),
        path: format!("C:\\{}", name),
        is_dir: true,
        size,
        file_count: 1,
        children: Vec::new(),
        rule_id: rule_id.map(|s| s.into()),
        top_extensions: Vec::new(),
    }
}

#[test]
fn health_score_full_disk_is_low() {
    let h = health_score(95.0, 10.0, 200.0);
    assert!(h.score < 20, "95% 使用率应得低分，实际 {}", h.score);
    assert_eq!(h.grade, "F");
}

#[test]
fn health_score_half_disk_is_100() {
    let h = health_score(50.0, 100.0, 200.0);
    assert_eq!(h.score, 100);
    assert_eq!(h.grade, "A");
}

#[test]
fn health_score_empty_disk_clamps_to_100() {
    let h = health_score(0.0, 200.0, 200.0);
    assert_eq!(h.score, 100, "空盘应 clamp 到 100");
}

#[test]
fn health_score_80pct_is_40() {
    // 100 - (80-50)*2 = 40
    let h = health_score(80.0, 40.0, 200.0);
    assert_eq!(h.score, 40);
}

#[test]
fn build_report_without_disk() {
    let root = mk_node("C:", 1000, None);
    let report = wcs_report::build(&root, None);
    assert_eq!(report.health.total_gb, 0.0);
    assert_eq!(report.summary.safely_cleanable_gb, 0.0);
}

#[test]
fn build_report_with_risk_grading() {
    let mut root = mk_node("C:", 3_000_000_000, None);
    root.children = vec![
        mk_node("temp", 1_000_000_000, Some("r-low")),
        mk_node("cache", 500_000_000, Some("r-med")),
        mk_node("danger", 200_000_000, Some("r-high")),
    ];
    let rules = vec![
        mk_rule("r-low", Risk::Low),
        mk_rule("r-med", Risk::Medium),
        mk_rule("r-high", Risk::High),
    ];
    let report = wcs_report::build_with_disk(&root, None, None, &rules);
    assert!((report.summary.safely_cleanable_gb - 1.0).abs() < 0.01);
    assert!((report.summary.needs_confirm_gb - 0.5).abs() < 0.01);
    assert!((report.summary.high_risk_gb - 0.2).abs() < 0.01);
}

#[test]
fn report_to_json_roundtrip() {
    let root = mk_node("C:", 1000, None);
    let report = wcs_report::build(&root, None);
    let json = wcs_report::to_json(&report).unwrap();
    let parsed: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.mode, "scan");
}

#[test]
fn report_has_schema_version() {
    let root = mk_node("C:", 1000, None);
    let report = wcs_report::build(&root, None);
    assert_eq!(report.schema_version, "1", "报告应带 schema 版本");
    // JSON 里也要出现（供消费方判断兼容性）
    let json = wcs_report::to_json(&report).unwrap();
    assert!(json.contains("\"schema_version\""));
}

#[test]
fn report_pretty_contains_score() {
    let root = mk_node("C:", 1000, None);
    let report = wcs_report::build(&root, None);
    let s = wcs_report::to_pretty(&report);
    assert!(s.contains("健康评分"));
    assert!(s.contains("Top 目录"));
}
