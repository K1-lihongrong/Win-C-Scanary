//! Safety test for rule `npm-cache`。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/npm-cache.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("npm-cache 规则应能解析");
    assert_eq!(rule.id, "npm-cache");
    assert_eq!(rule.scopes.len(), 2);
}

#[test]
fn passes_red_line_validation() {
    let rule = parse_toml(RULE_TOML).unwrap();
    assert!(validate_red_lines(&rule).is_ok());
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let roaming = rule
        .scopes
        .iter()
        .find(|s| s.id == "npm-cache-roaming")
        .unwrap();
    let glob = wcs_scaffold::expand_env(&roaming.glob).replace('\\', "/");
    let appdata = wcs_scaffold::expand_env("%APPDATA%").replace('\\', "/");
    let sample = format!("{}/npm-cache/_cacache/index-v5/ab/cd", appdata);
    assert!(glob_match(&glob, &sample), "npm-cache-roaming 应命中 {}", sample);
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let red_lines = [
        "C:/Users/me/AppData/Roaming/npm/node_modules/pkg/index.js",
        "C:/Users/me/Documents/myproject/node_modules/foo/index.js",
        "C:/Users/me/Documents/myproject/.git/config",
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
