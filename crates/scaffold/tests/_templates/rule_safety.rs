//! 规则 safety test 模板。
//!
//! 用法：复制本文件到 crates/scaffold/tests/<rule-id>_safety.rs，替换 RULE_ID /
//! RULE_FILE 与两条断言里的路径，即可。
//!
//! 每份规则必须配一个这样的测试——无测试不合并（见 DESIGN.md 原则 5）。

use std::path::Path;

/// 规则 id（对应 rules/<id>.toml）
const RULE_ID: &str = "<rule-id>";
/// 规则文件路径（相对 workspace 根）
const RULE_FILE: &str = "rules/<rule-id>.toml";

/// 从 workspace 根读规则文件并解析（含红线校验）。
fn load_rule() -> wcs_scaffold::Rule {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(RULE_FILE);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读规则失败 {:?}: {}", path, e));
    wcs_scaffold::parse_toml(&text).expect("规则应能解析且不命中红线")
}

#[test]
fn rule_parses() {
    let r = load_rule();
    assert_eq!(r.id, RULE_ID);
    assert!(!r.scopes.is_empty(), "规则应至少有一个 scope");
}

#[test]
fn passes_red_line_validation() {
    // parse_toml 内部已调 validate_red_lines；这里再显式确认一次
    let r = load_rule();
    wcs_scaffold::validate_red_lines(&r).expect("规则不得命中红线");
}

#[test]
fn positive_assertions() {
    // 正向：每个 scope 的 glob 应能匹配至少一条"典型命中路径"。
    // 用 glob_match 直接验证 glob 语义（不需要真实文件存在）。
    let r = load_rule();
    for scope in &r.scopes {
        // TODO: 为每个 scope 填一条应被命中的示例路径
        let sample = "<替换为该 scope 的典型命中路径>";
        let glob = wcs_scaffold::expand_env(&scope.glob).replace('\\', "/");
        let sample_norm = sample.replace('\\', "/");
        assert!(
            wcs_scaffold::glob_match(&glob, &sample_norm),
            "scope {} 的 glob 应命中示例 {}",
            scope.id,
            sample
        );
    }
}

#[test]
fn red_line_assertions() {
    // 红线：一组受保护路径必须 zero match（绝不误伤用户数据/账号/密钥）。
    let r = load_rule();
    let protected = [
        "C:/Users/me/Documents/db_storage/x.db",
        "C:/Users/me/Documents/WeChat Files/Msg/x.dat",
        "C:/Users/me/AppData/Roaming/App/Accounts/a.json",
        "C:/Users/me/.git/config",
        "C:/Users/me/.crypto/key.pem",
    ];
    for scope in &r.scopes {
        let glob = wcs_scaffold::expand_env(&scope.glob).replace('\\', "/");
        for p in &protected {
            assert!(
                !wcs_scaffold::glob_match(&glob, p),
                "scope {} 不得命中受保护路径 {}",
                scope.id,
                p
            );
        }
    }
}
