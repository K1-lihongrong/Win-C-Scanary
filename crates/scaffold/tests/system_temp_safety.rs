//! Safety test for rule `system-temp`。
//!
//! 正向断言：scope 的 glob 应能命中预期的临时文件路径。
//! 红线断言：红线路径必须 zero match。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/system-temp.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("system-temp 规则应能解析");
    assert_eq!(rule.id, "system-temp");
    assert!(!rule.scopes.is_empty(), "应至少有一个 scope");
    assert!(rule.risk == wcs_scaffold::Risk::Low, "应为 low 风险");
}

#[test]
fn passes_red_line_validation() {
    let rule = parse_toml(RULE_TOML).unwrap();
    assert!(
        validate_red_lines(&rule).is_ok(),
        "system-temp 不应命中任何红线"
    );
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    // 用户临时目录下的文件应命中 user-temp scope
    let user_temp = &rule.scopes.iter().find(|s| s.id == "user-temp").unwrap();
    let glob = user_temp.glob.replace("\\", "/");
    // %TEMP% 展开后形如 C:/Users/xxx/AppData/Local/Temp
    // 用展开后的模式测试一个具体样本
    let expanded = wcs_scaffold::expand_env("%TEMP%").replace('\\', "/");
    let sample = format!("{}/some-cache-file.tmp", expanded);
    assert!(
        glob_match(&glob.replace("%TEMP%", &expanded), &sample),
        "user-temp 的 glob 应命中 {}",
        sample
    );
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let red_lines = [
        "C:/Users/me/Documents/important.db",
        "C:/Users/me/Documents/project/.git/config",
        "C:/Users/me/.vscode/extensions/foo",
        "C:/Windows/System32/kernel32.dll",
    ];
    for scope in &rule.scopes {
        let glob = wcs_scaffold::expand_env(&scope.glob).replace('\\', "/");
        for red in red_lines {
            assert!(
                !glob_match(&glob, red),
                "scope {} 的 glob 不应命中红线 {}",
                scope.id,
                red
            );
        }
    }
}
