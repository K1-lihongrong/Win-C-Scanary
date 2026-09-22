//! Safety test for rule `browser-cache`。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/browser-cache.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("browser-cache 规则应能解析");
    assert_eq!(rule.id, "browser-cache");
    assert_eq!(rule.scopes.len(), 5, "应有 5 个 scope");
}

#[test]
fn passes_red_line_validation() {
    let rule = parse_toml(RULE_TOML).unwrap();
    assert!(validate_red_lines(&rule).is_ok());
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let chrome = rule.scopes.iter().find(|s| s.id == "chrome-cache").unwrap();
    let glob = wcs_scaffold::expand_env(&chrome.glob).replace('\\', "/");
    let local = wcs_scaffold::expand_env("%LOCALAPPDATA%").replace('\\', "/");
    let sample = format!("{}/Google/Chrome/User Data/Default/Cache/data_0", local);
    assert!(glob_match(&glob, &sample), "chrome-cache 应命中 {}", sample);
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let red_lines = [
        "C:/Users/me/AppData/Local/Google/Chrome/User Data/Default/Login Data",
        "C:/Users/me/AppData/Local/Google/Chrome/User Data/Default/Bookmarks",
        "C:/Users/me/AppData/Local/Google/Chrome/User Data/Default/History.db",
        "C:/Users/me/AppData/Local/Google/Chrome/User Data/Default/Extensions/abc/manifest.json",
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
