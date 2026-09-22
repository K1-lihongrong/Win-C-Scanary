//! Safety test for rule `conda`。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/conda.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("conda 规则应能解析");
    assert_eq!(rule.id, "conda");
    assert_eq!(rule.scopes.len(), 3, "应有 3 个 scope");
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
    let pkgs = rule
        .scopes
        .iter()
        .find(|s| s.id == "conda-pkgs-anaconda")
        .unwrap();
    let glob = wcs_scaffold::expand_env(&pkgs.glob).replace('\\', "/");
    let sample = format!("{}/anaconda3/pkgs/numpy-1.26.0-py311.tar.bz2", profile);
    assert!(
        glob_match(&glob, &sample),
        "conda-pkgs-anaconda 应命中 {}",
        sample
    );
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let red_lines = [
        "C:/Users/me/anaconda3/envs/myenv/python.exe",
        "C:/Users/me/Documents/myproject/.git/config",
        "C:/Users/me/.vscode/extensions/foo",
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
