//! inventory 单元测试。
//!
//! 注册表内容不可控，故只断言"不报错 + 结构合理"等不变量。

use wcs_inventory::{list_apps, AppEntry};

#[test]
fn list_apps_does_not_error() {
    let apps = list_apps().expect("list_apps 不应报错");
    // Windows 上通常能读到若干软件；非 Windows 返回空列表。
    #[cfg(windows)]
    {
        // 不强制 >0（极简/沙箱环境可能为空），但不应 panic
        let _ = apps.len();
    }
    #[cfg(not(windows))]
    assert!(apps.is_empty());
}

#[test]
fn entries_have_nonempty_name() {
    let apps = list_apps().unwrap();
    for a in &apps {
        assert!(!a.name.trim().is_empty(), "DisplayName 为空的项应被过滤");
    }
}

#[test]
fn no_duplicate_name_version_pairs() {
    let apps = list_apps().unwrap();
    let mut seen = std::collections::HashSet::new();
    for a in &apps {
        let key = (a.name.to_lowercase(), a.version.clone());
        assert!(seen.insert(key), "不应有重复 (name, version): {}", a.name);
    }
}

#[test]
fn app_entry_serde_roundtrip() {
    let e = AppEntry {
        name: "Example".into(),
        version: Some("1.2.3".into()),
        publisher: Some("ACME".into()),
        install_location: Some("C:\\Apps\\Example".into()),
        estimated_size_mb: Some(42),
        per_user: false,
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: AppEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Example");
    assert_eq!(back.version.as_deref(), Some("1.2.3"));
    assert_eq!(back.estimated_size_mb, Some(42));
}

#[test]
fn app_entry_optional_fields_default() {
    // 缺省字段应能反序列化（serde default）
    let json = r#"{"name":"Minimal"}"#;
    let e: AppEntry = serde_json::from_str(json).unwrap();
    assert_eq!(e.name, "Minimal");
    assert!(e.version.is_none());
    assert!(!e.per_user);
}
