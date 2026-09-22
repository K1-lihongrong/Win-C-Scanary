//! scope_dir_prefix 单元测试。
use wcs_scaffold::scope_dir_prefix;

/// match_scope 短名回归：root 为 8.3 短名（如 %TEMP%）时也应命中规则 glob。
/// 复现 V14 发现的问题：短名 root + 长名 glob 曾导致 matched 0。
#[test]
fn match_scope_handles_short_name_root() {
    use wcs_scaffold::{match_scope, Mode, RecycleGranularity, Scope};
    // 在 %TEMP% 下建测试文件（%TEMP% 在本机是短名 ADMINI~1）
    let temp = std::env::var("TEMP").expect("TEMP 应存在");
    let dir = std::path::PathBuf::from(&temp).join("wcs-shortname-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe.tmp"), b"x").unwrap();

    let scope = Scope {
        id: "s".into(),
        label: String::new(),
        glob: "%TEMP%/wcs-shortname-test/**".into(),
        mode: Mode::Recycle,
        prompt: None,
        category: None,
        variant: None,
        recycle_granularity: RecycleGranularity::File,
    };
    let hits = match_scope(&scope, &dir).unwrap();
    assert!(!hits.is_empty(), "短名 root 下应命中 probe.tmp，实际 0");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn strips_trailing_double_star() {
    // 环境变量会被展开，这里用不含变量的字面 glob
    let p = scope_dir_prefix("C:/foo/bar/**");
    assert_eq!(p, "c:/foo/bar");
}

#[test]
fn keeps_wildcard_segment() {
    let p = scope_dir_prefix("C:/foo/*/bar/**");
    assert_eq!(p, "c:/foo/*/bar");
}

#[test]
fn normalizes_separators_and_case() {
    let p = scope_dir_prefix("C:\\Foo\\Bar\\**");
    assert_eq!(p, "c:/foo/bar");
}

#[test]
fn expands_env_var() {
    // 用 PATH 这类必存在的变量；结果应不含 '%'
    let p = scope_dir_prefix("%PATH%/**");
    assert!(!p.contains('%'));
    assert!(!p.ends_with("**"));
}

#[test]
fn temp_short_name_is_converted() {
    // %TEMP% 的值可能是 8.3 短名（如 ADMINI~1），转换后不应含 '~<数字>' 段
    let p = scope_dir_prefix("%TEMP%/**");
    assert!(!p.contains('~'), "短名未转换: {}", p);
    assert!(!p.ends_with("**"));
}
