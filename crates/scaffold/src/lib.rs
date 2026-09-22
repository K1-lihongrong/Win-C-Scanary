//! wcs-scaffold — 规则引擎（TOML 加载/匹配）。

use anyhow::Result;
use std::path::Path;

pub mod model;

pub use model::{Match, Mode, Prompt, RecycleGranularity, Risk, Rule, Scope};

/// 危险 glob 红线片段
pub const RED_LINE_PATTERNS: &[&str] = &[
    "*.db",
    "*.db-wal",
    "*.db-shm",
    "**/db_storage/**",
    "**/Msg/**",
    "**/MultiMsg/**",
    "**/Accounts/**",
    "**/login/**",
    "**/Favorite*/**",
    "**/Fav/**",
    "**/key/**",
    "**/crypto/**",
    "**/.git/**",
    "**/.vscode/**",
    "**/JetBrains/**",
];

/// 解析单条 TOML 规则
pub fn parse_toml(s: &str) -> Result<Rule> {
    let rule: Rule = toml::from_str(s)?;
    validate_red_lines(&rule)?;
    Ok(rule)
}

/// 校验规则的 glob 是否命中红线
pub fn validate_red_lines(rule: &Rule) -> Result<()> {
    for scope in &rule.scopes {
        let g = scope.glob.to_lowercase();
        for red in RED_LINE_PATTERNS {
            let r = red.to_lowercase().replace("**/", "").replace("/*", "");
            let needle = r.trim_end_matches('/');
            if !needle.is_empty() && g.contains(needle) {
                anyhow::bail!("规则 {} 的 scope {} 命中红线 {:?}", rule.id, scope.id, red);
            }
        }
    }
    Ok(())
}

/// 加载目录下所有规则
pub fn load_dir(dir: &Path) -> Result<Vec<Rule>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().map(|e| e == "toml").unwrap_or(false) {
            let text = std::fs::read_to_string(&p)?;
            match parse_toml(&text) {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!("规则解析失败 {:?}: {}", p, e),
            }
        }
    }
    Ok(out)
}

/// 展开环境变量（$VAR / ${VAR} / %VAR%）并规范化路径。
///
/// 规范化包含一步 Windows 特有问题：8.3 短名 → 长名。
/// 例：`%TEMP%` 的值常为 `C:\Users\ADMINI~1\AppData\Local\Temp`（短名），
/// 而 MFT 扫描给出的路径是长名 `C:\Users\Administrator\...`。不转换会导致前缀匹配失败。
/// 非 Windows 或路径不存在时保持原值（fail-soft）。
pub fn expand_env(s: &str) -> String {
    let unix = shellexpand::env(s).map(|c| c.into_owned()).unwrap_or_else(|_| s.to_string());
    let expanded = expand_winpct(&unix);
    shorten_to_long(&expanded)
}

/// 把 8.3 短名路径转为长名（Windows）；失败或非 Windows 时原样返回。
///
/// 若路径含通配段（如 `.../Temp/**` 或 `.../User Data/*/Cache`），
/// GetLongPathNameW 会因路径不存在而失败。此时只转换"第一个通配段之前"的目录部分，
/// 通配段及其之后原样保留（短名只可能出现在已存在的真实目录前缀里）。
fn shorten_to_long(s: &str) -> String {
    #[cfg(windows)]
    {
        // 先按原样尝试
        if let Some(long) = long_path_exact(s) {
            return long;
        }
        // 含通配：找到第一个含 '*' 的 '/' 段，只转换其之前部分
        let segs: Vec<&str> = s.split('/').collect();
        if let Some(pos) = segs.iter().position(|seg| seg.contains('*')) {
            if pos > 0 {
                let prefix = segs[..pos].join("/");
                let suffix = segs[pos..].join("/");
                if let Some(long_prefix) = long_path_exact(&prefix) {
                    return format!("{}/{}", long_prefix, suffix);
                }
            }
        }
        s.to_string()
    }
    #[cfg(not(windows))]
    {
        s.to_string()
    }
}

/// 调用 GetLongPathNameW 做精确转换；路径不存在或无变化时返回 None。
#[cfg(windows)]
fn long_path_exact(s: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;
    let wide: Vec<u16> = std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let need = unsafe { GetLongPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if need == 0 {
        return None;
    }
    let mut buf = vec![0u16; need as usize];
    let written = unsafe { GetLongPathNameW(wide.as_ptr(), buf.as_mut_ptr(), need) };
    if written == 0 || written > need {
        return None;
    }
    buf.truncate(written as usize);
    Some(String::from_utf16_lossy(&buf))
}

fn expand_winpct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        let mut closed = false;
        while let Some(&nc) = chars.peek() {
            chars.next();
            if nc == '%' {
                closed = true;
                break;
            }
            name.push(nc);
        }
        if closed {
            match std::env::var(&name) {
                Ok(v) => out.push_str(&v),
                Err(_) => {
                    out.push('%');
                    out.push_str(&name);
                    out.push('%');
                }
            }
        } else {
            out.push('%');
            out.push_str(&name);
        }
    }
    out
}

/// 单次匹配（每次重建 globset）
pub fn detect_for(rules: &[Rule], path: &Path) -> Option<String> {
    let p = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let base = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    for r in rules {
        for d in &r.detect {
            let pat = expand_env(d).replace('\\', "/").to_lowercase();
            if glob_match(&pat, &p) {
                return Some(r.id.clone());
            }
        }
        if !r.matcher.name_contains.is_empty()
            && r.matcher.name_contains.iter().any(|n| base.contains(&n.to_lowercase()))
            && r.matcher.must_have_child.iter().all(|c| path.join(c).exists())
        {
            return Some(r.id.clone());
        }
    }
    None
}

/// 编译单个 glob 为 GlobMatcher（大小写不敏感，分隔符不敏感）
fn compile_glob(pat: &str) -> Option<globset::GlobMatcher> {
    globset::GlobBuilder::new(pat)
        .literal_separator(false)
        .case_insensitive(true)
        .build()
        .ok()
        .map(|g| g.compile_matcher())
}

/// 对外暴露的 glob 匹配（供 safety test 用）
pub fn glob_match(pat: &str, text: &str) -> bool {
    match compile_glob(pat) {
        Some(m) => m.is_match(text),
        None => false,
    }
}

/// 把 scope 的 glob 转成"目录前缀"：展开环境变量 → 去掉尾部的文件名/通配段 → 归一化为 `/` 小写。
///
/// 例：`%LOCALAPPDATA%/Google/Chrome/User Data/*/Cache/**`
///  → `c:/users/me/appdata/local/google/chrome/user data/*/cache`
///
/// 规则：去掉结尾的 `/**`；若最后一段不含通配且像文件名（含 `.`），也去掉。
/// 返回空串表示无法提取（调用方应跳过）。
pub fn scope_dir_prefix(glob: &str) -> String {
    // expand_env 已处理通配段前的短名→长名转换
    let s = expand_env(glob).replace('\\', "/").to_lowercase();
    let s = s.trim_end_matches('/');
    // 去掉结尾的 ** 段
    let s = s.strip_suffix("/**").unwrap_or(s);
    let s = s.strip_suffix("**").unwrap_or(s);
    let s = s.trim_end_matches('/');
    s.to_string()
}

/// 匹配 scope 的文件/目录。
///
/// 以传入的 `root` 为遍历起点，只返回**位于 root 之下**且匹配 scope glob 的路径。
/// 若 root 不存在或非目录，返回空列表（fail-closed，避免越界遍历）。
pub fn match_scope(scope: &Scope, root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();

    // root 必须存在且为目录，否则不扫
    if !root.is_dir() {
        return Ok(out);
    }

    let base = expand_env(&scope.glob).replace('\\', "/");
    let matcher = match compile_glob(&base) {
        Some(m) => m,
        None => return Ok(out),
    };

    // 遍历起点限定在 root 之下。
    //
    // 注意：root 可能是 8.3 短名（如 %TEMP% 展开为 C:\Users\ADMINI~1\...），
    // 而规则 glob 展开后是长名；两者直接字符串匹配会失败（matched 0）。
    // 故对每个条目的路径做"短名→长名"归一化后再匹配（与 T-PATH-1 同源）。
    for entry in walkdir::WalkDir::new(root)
        .max_depth(16)
        .into_iter()
        .flatten()
    {
        let p = entry.path();
        if p == root {
            continue;
        }
        let p_str = p.to_string_lossy().replace('\\', "/");
        let p_norm = shorten_to_long(&p_str);
        if matcher.is_match(&p_norm) {
            out.push(p.to_path_buf());
        }
    }
    Ok(out)
}
