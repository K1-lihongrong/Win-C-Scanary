//! 系统级清理 —— WinSxS / Windows.old 的**指导型**建议。
//!
//! **安全定位（最高风险，只指导不执行）**：本模块只做两件事：
//! 1. **只读检测**：量 `C:\Windows\WinSxS` 与 `C:\Windows.old` 的大小（用于提示占用）。
//! 2. **输出命令**：给出 DISM / cleanmgr 命令文本，**绝不执行任何清理或修改系统的操作**。
//!
//! WinSxS 组件存储含硬链接，资源管理器显示的大小**虚高**；真实可回收量需用
//! `DISM /Online /Cleanup-Image /AnalyzeComponentStore` 分析。手删 WinSxS 会损坏系统，
//! 必须用 DISM。`/ResetBase` 会永久删除更新回滚能力，风险最高。

use serde::{Deserialize, Serialize};

/// 系统清理项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemItem {
    /// 名称（如 "WinSxS 组件存储"）
    pub name: String,
    /// 路径
    pub path: String,
    /// 大小（字节）；None 表示不存在或无权读取
    pub size_bytes: Option<u64>,
    /// 大小是否可靠（WinSxS 因硬链接而虚高，标记 false）
    pub size_reliable: bool,
    /// 说明
    pub note: String,
}

/// 一条建议命令
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemCommand {
    /// 用途
    pub purpose: String,
    /// 命令文本（用户自行执行，本工具不执行）
    pub command: String,
    /// 是否需要管理员
    pub needs_admin: bool,
    /// 是否破坏性/不可逆
    pub destructive: bool,
    /// 风险提示（破坏性时必填）
    pub warning: Option<String>,
}

/// 系统清理建议汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAdvice {
    /// 卷（如 "C:"）
    pub volume: String,
    /// 检测到的系统项
    pub items: Vec<SystemItem>,
    /// 是否管理员
    pub is_admin: bool,
    /// 建议命令
    pub commands: Vec<SystemCommand>,
    /// 总提示
    pub notes: Vec<String>,
}

/// 检测 WinSxS 与 Windows.old 的大小（fail-soft）。
pub fn detect_items(volume_root: &str) -> Vec<SystemItem> {
    let root = volume_root.trim_end_matches(['/', '\\']);
    let items = [
        (
            "WinSxS 组件存储",
            format!("{}\\Windows\\WinSxS", root),
            false,
            "资源管理器显示的大小因硬链接而虚高，真实可回收量需用 DISM 分析。此目录绝不可手工删除。",
        ),
        (
            "Windows.old（旧系统备份）",
            format!("{}\\Windows.old", root),
            true,
            "系统升级/重装后保留的旧系统，可回滚窗口（默认 10 天）过后可安全删除。",
        ),
    ];
    items
        .iter()
        .map(|(name, path, reliable, note)| {
            let size = std::fs::metadata(path).ok().map(|m| m.len());
            SystemItem {
                name: name.to_string(),
                path: path.clone(),
                size_bytes: size,
                size_reliable: *reliable,
                note: note.to_string(),
            }
        })
        .collect()
}

/// 生成建议命令（纯文本，不执行）。
pub fn build_commands() -> Vec<SystemCommand> {
    vec![
        SystemCommand {
            purpose: "分析 WinSxS 真实可回收量（只读，建议先跑）".into(),
            command: "DISM /Online /Cleanup-Image /AnalyzeComponentStore".into(),
            needs_admin: true,
            destructive: false,
            warning: None,
        },
        SystemCommand {
            purpose: "标准清理 WinSxS（删除被取代的旧组件版本，保留更新回滚能力）".into(),
            command: "DISM /Online /Cleanup-Image /StartComponentCleanup".into(),
            needs_admin: true,
            destructive: false,
            warning: None,
        },
        SystemCommand {
            purpose: "彻底清理 WinSxS（额外删除所有旧组件备份）".into(),
            command: "DISM /Online /Cleanup-Image /StartComponentCleanup /ResetBase".into(),
            needs_admin: true,
            destructive: true,
            warning: Some(
                "⚠️ 不可逆：执行后所有已安装的更新都无法卸载/回滚。Windows 11 24H2/25H2 上还可能导致后续累积更新安装失败。仅在确认系统稳定、不需回滚时使用。".into(),
            ),
        },
        SystemCommand {
            purpose: "删除 Windows.old（官方推荐：磁盘清理图形界面）".into(),
            command: "cleanmgr  # 选 C: → 清理系统文件 → 勾选「以前的 Windows 安装」→ 确定".into(),
            needs_admin: true,
            destructive: true,
            warning: Some(
                "⚠️ 不可逆：删除后无法回退到旧系统版本。请确认新系统稳定运行、数据已迁移。".into(),
            ),
        },
        SystemCommand {
            purpose: "删除 Windows.old（命令行批量方式）".into(),
            command: "cleanmgr /sageset:1  # 配置勾选项；再执行 cleanmgr /sagerun:1".into(),
            needs_admin: true,
            destructive: true,
            warning: Some("⚠️ 不可逆：同图形界面方式，删除后无法回退。".into()),
        },
    ]
}

/// 组装完整建议。
pub fn analyze(volume_root: &str, is_admin: bool) -> SystemAdvice {
    let volume: String = volume_root.chars().take(2).collect();
    let items = detect_items(volume_root);
    let commands = build_commands();

    let mut notes = Vec::new();
    if !is_admin {
        notes.push("当前非管理员：DISM 与磁盘清理的系统文件模式均需管理员权限。".to_string());
    }
    notes.push("WinSxS 目录含系统组件硬链接，手工删除会损坏系统、导致无法更新甚至无法启动。必须用 DISM。".to_string());
    notes.push("本工具仅输出命令，不执行任何清理。请逐条确认风险后自行在管理员终端运行。".to_string());

    SystemAdvice { volume, items, is_admin, commands, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_items_returns_two_entries() {
        let items = detect_items("C:\\");
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.name.contains("WinSxS")));
        assert!(items.iter().any(|i| i.name.contains("Windows.old")));
    }

    #[test]
    fn winsxs_marked_unreliable_size() {
        let items = detect_items("C:\\");
        let winsxs = items.iter().find(|i| i.name.contains("WinSxS")).unwrap();
        assert!(!winsxs.size_reliable);
    }

    #[test]
    fn nonexistent_volume_soft_fails() {
        let items = detect_items("Z:\\nonexistent-volume-xyz\\");
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.size_bytes.is_none()));
    }

    #[test]
    fn commands_include_analyze_and_cleanup() {
        let cmds = build_commands();
        assert!(cmds.iter().any(|c| c.command.contains("AnalyzeComponentStore")));
        assert!(cmds.iter().any(|c| c.command.contains("StartComponentCleanup") && !c.command.contains("ResetBase")));
    }

    #[test]
    fn resetbase_is_flagged_destructive() {
        let cmds = build_commands();
        let rb = cmds.iter().find(|c| c.command.contains("ResetBase")).unwrap();
        assert!(rb.destructive);
        assert!(rb.warning.is_some());
    }

    #[test]
    fn windows_old_commands_are_destructive() {
        let cmds = build_commands();
        let wo: Vec<&SystemCommand> = cmds.iter().filter(|c| c.command.contains("cleanmgr")).collect();
        assert!(!wo.is_empty());
        assert!(wo.iter().all(|c| c.destructive && c.warning.is_some()));
    }

    #[test]
    fn analyze_notes_mention_admin_and_no_execute() {
        let advice = analyze("C:\\", false);
        assert!(!advice.is_admin);
        assert!(advice.notes.iter().any(|n| n.contains("管理员")));
        assert!(advice.notes.iter().any(|n| n.contains("不执行")));
        assert_eq!(advice.volume, "C:");
    }
}
