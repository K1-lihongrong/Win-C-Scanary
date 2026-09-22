//! Safety test for rule `wechat-pc`。
//!
//! 微信缓存规则**必须严守红线**：绝不命中 db_storage / Msg / Accounts / Favorite。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/wechat-pc.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("wechat-pc 规则应能解析");
    assert_eq!(rule.id, "wechat-pc");
    assert_eq!(rule.scopes.len(), 2, "应有 2 个 scope");
}

#[test]
fn passes_red_line_validation() {
    let rule = parse_toml(RULE_TOML).unwrap();
    assert!(validate_red_lines(&rule).is_ok());
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let profile = wcs_scaffold::expand_env("%USERPROFILE%").replace('\\', "/");
    let cache = rule.scopes.iter().find(|s| s.id == "wechat-cache").unwrap();
    let glob = wcs_scaffold::expand_env(&cache.glob).replace('\\', "/");
    let sample = format!(
        "{}/Documents/WeChat Files/wxid_abc/FileStorage/Cache/2026-09/img.dat",
        profile
    );
    assert!(glob_match(&glob, &sample), "wechat-cache 应命中 {}", sample);
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    // 微信账号/聊天/收藏数据，绝对不可命中
    let red_lines = [
        "C:/Users/me/Documents/WeChat Files/wxid_abc/FileStorage/Msg/chat.db",
        "C:/Users/me/Documents/WeChat Files/wxid_abc/db_storage/msg.db",
        "C:/Users/me/Documents/WeChat Files/wxid_abc/FileStorage/MultiMsg/foo",
        "C:/Users/me/Documents/WeChat Files/wxid_abc/FileStorage/Accounts/abc",
        "C:/Users/me/Documents/WeChat Files/wxid_abc/FileStorage/Favorite/fav.db",
        "C:/Users/me/Documents/WeChat Files/wxid_abc/FileStorage/Fav/fav.db",
        "C:/Users/me/Documents/WeChat Files/wxid_abc/FileStorage/Cache/../Msg/chat.db",
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
