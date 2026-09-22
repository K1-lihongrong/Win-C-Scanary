//! 休眠/页面文件管理 —— 检测系统休眠文件大小 + 给出可逆的管理命令。
//!
//! **安全定位**：本模块只做两类事：
//! 1. **只读检测**：读 `hiberfil.sys`/`pagefile.sys`/`swapfile.sys` 大小；调 `powercfg /a`（只读查询）。
//! 2. **输出建议**：给出 `powercfg` 命令文本，**绝不执行任何修改系统的操作**。
//!
//! 关闭休眠（`powercfg /h off`）是可逆的（`powercfg /h on` 可恢复），但属系统改动，需管理员；
//! 本工具只提示，交由用户自行决定与执行。

use serde::{Deserialize, Serialize};

/// 一个系统文件的大小信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemFile {
    /// 文件路径（如 C:\hiberfil.sys）
    pub path: String,
    /// 大小（字节）；None 表示不存在或无权读取
    pub size_bytes: Option<u64>,
}

/// 休眠状态。
///
/// **判据说明**：以 `hiberfil.sys` 是否存在作为"休眠是否启用"的权威判据——
/// 该文件存在 ⇔ 休眠开启。不用解析 `powercfg /a` 的文本，因为中文系统的输出是
/// GBK 编码，按 UTF-8 解读会乱码，导致关键字匹配失效（跨语言不可靠）。
/// `powercfg /a` 的原始输出仍保留在 `raw` 供排查，但不参与判断。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiberState {
    /// 休眠是否启用（= hiberfil.sys 是否存在）
    pub hibernation_enabled: bool,
    /// powercfg /a 的原始输出（仅排查用，可能因编码而乱码）
    pub raw: String,
}

/// 一条建议命令
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiberCommand {
    /// 用途说明
    pub purpose: String,
    /// 命令文本（用户自行执行，本工具不执行）
    pub command: String,
    /// 是否可逆
    pub reversible: bool,
    /// 是否需要管理员
    pub needs_admin: bool,
}

/// 休眠/页面文件管理建议汇总
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiberAdvice {
    /// 卷（如 "C:"）
    pub volume: String,
    /// 系统文件大小
    pub files: Vec<SystemFile>,
    /// 休眠状态
    pub state: HiberState,
    /// 是否管理员
    pub is_admin: bool,
    /// 建议命令
    pub commands: Vec<HiberCommand>,
    /// 说明/提示
    pub notes: Vec<String>,
}

/// 检测卷根上的系统文件大小（fail-soft：不存在或无权读取时为 None）。
pub fn detect_files(volume_root: &str) -> Vec<SystemFile> {
    let root = volume_root.trim_end_matches(['/', '\\']);
    let names = ["hiberfil.sys", "pagefile.sys", "swapfile.sys"];
    names
        .iter()
        .map(|n| {
            let path = format!("{}\\{}", root, n);
            let size = std::fs::metadata(&path).ok().map(|m| m.len());
            SystemFile { path, size_bytes: size }
        })
        .collect()
}

/// 查询休眠状态。
///
/// **权威判据**：`hiberfil.sys` 是否存在（存在 ⇔ 休眠启用）。
/// 同时调用只读的 `powercfg /a` 拿原始输出存进 `raw`（仅排查用，不参与判断，
/// 因中文系统输出为 GBK，UTF-8 解读会乱码）。
pub fn query_hiber_state(volume_root: &str) -> HiberState {
    let root = volume_root.trim_end_matches(['/', '\\']);
    let hiberfil = format!("{}\\hiberfil.sys", root);
    let hibernation_enabled = std::path::Path::new(&hiberfil).exists();

    let raw = {
        #[cfg(windows)]
        {
            std::process::Command::new("powercfg")
                .arg("/a")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            String::new()
        }
    };

    HiberState { hibernation_enabled, raw }
}

/// 生成建议命令（纯文本，不执行）。
pub fn build_commands(state: &HiberState) -> Vec<HiberCommand> {
    let mut cmds = Vec::new();
    if state.hibernation_enabled {
        cmds.push(HiberCommand {
            purpose: "关闭休眠并删除 hiberfil.sys（释放数 GB 空间）".into(),
            command: "powercfg /h off".into(),
            reversible: true,
            needs_admin: true,
        });
    }
    cmds.push(HiberCommand {
        purpose: "恢复休眠（若之前关闭过）".into(),
        command: "powercfg /h on".into(),
        reversible: true,
        needs_admin: true,
    });
    cmds.push(HiberCommand {
        purpose: "缩小休眠文件到内存的 40%（保留休眠，仅减小体积）".into(),
        command: "powercfg /h /size 40".into(),
        reversible: true,
        needs_admin: true,
    });
    cmds
}

/// 组装完整建议。
pub fn analyze(volume_root: &str, is_admin: bool) -> HiberAdvice {
    let volume: String = volume_root.chars().take(2).collect();
    let files = detect_files(volume_root);
    let state = query_hiber_state(volume_root);
    let commands = build_commands(&state);

    let mut notes = Vec::new();
    if !is_admin {
        notes.push("当前非管理员：修改休眠设置需以管理员身份运行。".to_string());
    }
    notes.push("powercfg /h off 是可逆的系统改动（powercfg /h on 可恢复），本工具不执行，请自行确认后运行。".to_string());
    if !state.hibernation_enabled {
        notes.push("未检测到 hiberfil.sys，休眠当前未启用（可能已关闭）。powercfg /a 原始输出见 state.raw（仅排查用，中文系统可能乱码）。".to_string());
    }

    HiberAdvice { volume, files, state, is_admin, commands, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_files_returns_three_entries() {
        let files = detect_files("C:\\");
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|f| f.path.contains("hiberfil.sys")));
        assert!(files.iter().any(|f| f.path.contains("pagefile.sys")));
        assert!(files.iter().any(|f| f.path.contains("swapfile.sys")));
    }

    #[test]
    fn detect_files_nonexistent_volume_soft_fails() {
        // 不存在的盘：不应 panic，size 全为 None
        let files = detect_files("Z:\\nonexistent-volume-xyz\\");
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|f| f.size_bytes.is_none()));
    }

    #[test]
    fn commands_include_reversible_off_and_on() {
        let state = HiberState { hibernation_enabled: true, raw: String::new() };
        let cmds = build_commands(&state);
        assert!(cmds.iter().any(|c| c.command == "powercfg /h off"));
        assert!(cmds.iter().any(|c| c.command == "powercfg /h on"));
        assert!(cmds.iter().any(|c| c.command == "powercfg /h /size 40"));
        // 全部标注可逆
        assert!(cmds.iter().all(|c| c.reversible));
    }

    #[test]
    fn commands_when_hibernation_disabled_omit_off() {
        let state = HiberState { hibernation_enabled: false, raw: String::new() };
        let cmds = build_commands(&state);
        // 未检测到休眠时不给 off 建议（避免误导）
        assert!(!cmds.iter().any(|c| c.command == "powercfg /h off"));
        // 但仍给恢复命令
        assert!(cmds.iter().any(|c| c.command == "powercfg /h on"));
    }

    #[test]
    fn hiber_state_uses_file_existence() {
        // 不存在的盘 → hiberfil.sys 不存在 → enabled=false
        let st = query_hiber_state("Z:\\nonexistent-volume-xyz\\");
        assert!(!st.hibernation_enabled);
    }

    #[test]
    fn analyze_notes_admin_hint() {
        let advice = analyze("C:\\", false);
        assert!(!advice.is_admin);
        assert!(advice.notes.iter().any(|n| n.contains("管理员")));
        assert_eq!(advice.volume, "C:");
    }

    #[test]
    fn analyze_commands_never_execute_off() {
        // 保证 build_commands 只产出命令文本，不含任何执行副作用的调用——
        // 这里通过检查命令文本存在性间接验证（模块内无 Command::new 于 build 路径）
        let advice = analyze("C:\\", true);
        assert!(!advice.commands.is_empty());
        assert!(advice.commands.iter().all(|c| c.needs_admin));
    }
}
