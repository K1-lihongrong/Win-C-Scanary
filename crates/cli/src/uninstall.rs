//! 软件卸载指导 —— 列出已装软件 + 给出官方卸载入口。
//!
//! **安全定位（引导型，零执行）**：本模块只做两件事：
//! 1. **只读清单**：复用 `wcs_inventory::list_apps()`（读注册表三视图）。
//! 2. **输出引导**：给"设置 → 应用"等官方卸载入口文本，**绝不读取或执行 `UninstallString`**。
//!
//! 参考同类项目安全红线：不主动调 `UninstallString`，引导用户走系统设置。

use serde::{Deserialize, Serialize};

/// 一条软件记录（面向卸载场景，按大小排序用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppItem {
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    /// 估算大小（MB）
    pub estimated_size_mb: Option<u64>,
    /// 是否 per-user 安装
    pub per_user: bool,
}

/// 卸载引导入口
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallGuide {
    /// 用途
    pub purpose: String,
    /// 入口（命令或操作路径）
    pub entry: String,
}

/// 卸载指导汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallAdvice {
    /// 已装软件数
    pub app_count: usize,
    /// 软件清单（按估算大小降序，无大小者排后）
    pub apps: Vec<AppItem>,
    /// 官方卸载入口
    pub guides: Vec<UninstallGuide>,
    /// 提示
    pub notes: Vec<String>,
}

/// 生成官方卸载入口（纯文本，不执行）。
pub fn build_guides() -> Vec<UninstallGuide> {
    vec![
        UninstallGuide {
            purpose: "图形界面（推荐）".into(),
            entry: "设置 → 应用 → 已安装的应用，搜索软件名后选择「卸载」".into(),
        },
        UninstallGuide {
            purpose: "传统「程序和功能」面板".into(),
            entry: "运行 appwiz.cpl（Win+R 输入）".into(),
        },
        UninstallGuide {
            purpose: "命令行包管理器（若软件由 winget 安装）".into(),
            entry: "winget list  然后  winget uninstall <包名>".into(),
        },
    ]
}

/// 组装卸载指导。
///
/// * `apps` — 由调用方传入（来自 `wcs_inventory::list_apps()`），便于测试注入。
/// * `top_n` — 最多列出多少条（0 表示全部）。
pub fn analyze(apps: Vec<wcs_inventory::AppEntry>, top_n: usize) -> UninstallAdvice {
    let app_count = apps.len();
    let mut items: Vec<AppItem> = apps
        .into_iter()
        .map(|a| AppItem {
            name: a.name,
            version: a.version,
            publisher: a.publisher,
            estimated_size_mb: a.estimated_size_mb,
            per_user: a.per_user,
        })
        .collect();

    // 按估算大小降序（无大小者排最后），同大小按名称
    items.sort_by(|a, b| {
        b.estimated_size_mb
            .unwrap_or(0)
            .cmp(&a.estimated_size_mb.unwrap_or(0))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    if top_n > 0 {
        items.truncate(top_n);
    }

    let guides = build_guides();
    let notes = vec![
        "本工具只列出软件并给出官方卸载入口，不读取也不执行注册表中的 UninstallString。".to_string(),
        "卸载请走系统设置/控制面板，避免误删残留或系统组件。".to_string(),
    ];

    UninstallAdvice { app_count, apps: items, guides, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_app(name: &str, size: Option<u64>, per_user: bool) -> wcs_inventory::AppEntry {
        wcs_inventory::AppEntry {
            name: name.into(),
            version: Some("1.0".into()),
            publisher: None,
            install_location: None,
            estimated_size_mb: size,
            per_user,
        }
    }

    #[test]
    fn apps_sorted_desc_by_size() {
        let apps = vec![
            mk_app("small", Some(10), false),
            mk_app("big", Some(1000), false),
            mk_app("mid", Some(100), false),
        ];
        let advice = analyze(apps, 0);
        let names: Vec<&str> = advice.apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["big", "mid", "small"]);
        assert_eq!(advice.app_count, 3);
    }

    #[test]
    fn apps_without_size_sorted_last() {
        let apps = vec![
            mk_app("unknown", None, false),
            mk_app("known", Some(5), false),
        ];
        let advice = analyze(apps, 0);
        assert_eq!(advice.apps[0].name, "known");
        assert_eq!(advice.apps[1].name, "unknown");
    }

    #[test]
    fn top_n_limits_but_app_count_is_total() {
        let apps: Vec<_> = (0..5).map(|i| mk_app(&format!("a{}", i), Some(i * 10), false)).collect();
        let advice = analyze(apps, 2);
        assert_eq!(advice.apps.len(), 2);
        assert_eq!(advice.app_count, 5);
    }

    #[test]
    fn guides_include_settings_and_appwiz() {
        let advice = analyze(vec![], 0);
        assert!(advice.guides.iter().any(|g| g.entry.contains("设置")));
        assert!(advice.guides.iter().any(|g| g.entry.contains("appwiz.cpl")));
        assert!(advice.guides.iter().any(|g| g.entry.contains("winget")));
    }

    #[test]
    fn notes_state_no_uninstall_string() {
        let advice = analyze(vec![], 0);
        assert!(advice.notes.iter().any(|n| n.contains("UninstallString")));
        assert!(advice.notes.iter().any(|n| n.contains("不执行") || n.contains("不读取")));
    }

    #[test]
    fn empty_apps_ok() {
        let advice = analyze(vec![], 10);
        assert_eq!(advice.app_count, 0);
        assert!(advice.apps.is_empty());
        assert!(!advice.guides.is_empty());
    }
}
