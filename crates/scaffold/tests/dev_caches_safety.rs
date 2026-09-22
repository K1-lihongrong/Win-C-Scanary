//! Safety test for rule `dev-caches`。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/dev-caches.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("dev-caches 规则应能解析");
    assert_eq!(rule.id, "dev-caches");
    assert_eq!(rule.scopes.len(), 4, "应有 4 个 scope");
}

#[test]
fn passes_red_line_validation() {
    let rule = parse_toml(RULE_TOML).unwrap();
    assert!(validate_red_lines(&rule).is_ok());
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let local = wcs_scaffold::expand_env("%LOCALAPPDATA%").replace('\\', "/");
    let pip = rule.scopes.iter().find(|s| s.id == "pip-cache").unwrap();
    let glob = wcs_scaffold::expand_env(&pip.glob).replace('\\', "/");
    let sample = format!("{}/pip/Cache/http/ab/cd", local);
    assert!(glob_match(&glob, &sample), "pip-cache 应命中 {}", sample);
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let red_lines = [
        "C:/Users/me/Documents/myproject/src/main.rs",
        "C:/Users/me/Documents/myproject/.git/config",
        "C:/Users/me/.vscode/extensions/foo",
        "C:/Users/me/.cargo/bin/cargo.exe",
    ];
    for scope in &rule.scopes {
        let glob = wcs_scaffold::expand_env(&scope.glob).replace('\\', "/");
        for red in red_lines {
            assert!(
                !glob_match(&glob, red),
                "scope {} 不应命中红线 {}",
                scope.id,
                red
            );
        }
    }
}
