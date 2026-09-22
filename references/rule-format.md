# 规则格式（Rule Format）

> 规则是 **TOML 数据**，放 `rules/`，加规则不改代码。
> 模型定义见 `crates/scaffold/src/model.rs`。

## 1. 顶层字段（Rule）

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `id` | string | ✅ | 唯一标识（kebab-case） |
| `name` | string | ✅ | 显示名 |
| `homepage` | string? | | 官网/说明链接 |
| `risk` | `low`/`medium`/`high` | ✅ | 风险等级 |
| `disclaimer` | string | ✅ | 给用户看的影响说明 |
| `detect` | string[] | ✅ | 检测路径（用于 `rules` 识别） |
| `[match]` | table | | 匹配辅助（可选） |
| `[[scope]]` | table[] | ✅ | 清理范围（至少一个） |

## 2. 匹配辅助（[match]）

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `name_contains` | string[] | 目录名包含任一字符串即匹配 |
| `must_have_child` | string[] | 必须包含全部子目录才匹配 |

## 3. 清理范围（[[scope]]）

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `id` | string | ✅ | scope 唯一标识 |
| `label` | string | ✅ | 显示名 |
| `glob` | string | ✅ | 匹配模式（支持 `**`/`*`/`{a,b}`，大小写不敏感） |
| `mode` | `recycle`/`quarantine`/`delete`/`systemcmd` | ✅ | 执行模式 |
| `prompt` | table? | | 交互提示 |
| `category` | string? | | 分类（如 `cache`） |
| `variant` | string? | | 变体标记 |
| `recycle_granularity` | `file`/`directory` | | 回收粒度（默认 file） |

### 执行模式（mode）

| 模式 | 行为 |
| --- | --- |
| `recycle` | **默认**。进回收站，可恢复 |
| `quarantine` | 移动到隔离区 |
| `delete` | 永久删除（仅用户显式要求） |
| `systemcmd` | 执行系统命令（当前仅警告） |

### 交互提示（prompt）

```toml
# 确认
prompt = { kind = "confirm", label = "确认继续？" }
# 天数
prompt = { kind = "days", default = 30 }
# 字节数
prompt = { kind = "bytes", default = 10485760 }
# 选择
prompt = { kind = "choice", default = "a", options = ["a", "b"] }
```

## 4. 环境变量

`glob` 和 `detect` 支持环境变量展开：

- `%VAR%`（Windows 风格）
- `$VAR` / `${VAR}`（Unix 风格）

常用：`%TEMP%`、`%WINDIR%`、`%APPDATA%`、`%LOCALAPPDATA%`、`%USERPROFILE%`。

## 5. 红线约束

scope 的 glob **不得命中** `RED_LINE_PATTERNS`（见 [safety-rules.md](safety-rules.md)），否则规则**加载失败**。

## 6. 完整示例

```toml
id          = "npm-cache"
name        = "npm 缓存"
risk        = "low"
disclaimer  = "清理 npm 包缓存。下次安装依赖时会重新下载。不影响已安装的 node_modules。文件进入回收站，可恢复。"

detect = [
  "%APPDATA%/npm-cache",
  "%LOCALAPPDATA%/npm-cache",
]

[[scope]]
id       = "npm-cache-roaming"
label    = "npm 缓存 (Roaming)"
glob     = "%APPDATA%/npm-cache/**"
mode     = "recycle"
category = "cache"
recycle_granularity = "file"

[[scope]]
id       = "npm-cache-local"
label    = "npm 缓存 (Local)"
glob     = "%LOCALAPPDATA%/npm-cache/**"
mode     = "recycle"
category = "cache"
recycle_granularity = "file"
```

## 7. Safety Test 模板

每条规则**必须**配一个 safety test：`crates/scaffold/tests/<id>_safety.rs`（id 中 `-` 换成 `_`）。

```rust
//! Safety test for rule `<id>`。

use wcs_scaffold::{glob_match, parse_toml, validate_red_lines};

const RULE_TOML: &str = include_str!("../../../rules/<id>.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("<id> 规则应能解析");
    assert_eq!(rule.id, "<id>");
    assert_eq!(rule.scopes.len(), 1, "应有 1 个 scope");
}

#[test]
fn passes_red_line_validation() {
    let rule = parse_toml(RULE_TOML).unwrap();
    assert!(validate_red_lines(&rule).is_ok());
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let scope = &rule.scopes[0];
    let glob = wcs_scaffold::expand_env(&scope.glob).replace('\\', "/");
    // 用展开后的路径构造一个应命中的样本
    let sample = "...".to_string();
    assert!(glob_match(&glob, &sample), "应命中 {}", sample);
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    let red_lines = [
        "C:/Users/me/Documents/important.db",
        "C:/Users/me/Documents/project/.git/config",
    ];
    for scope in &rule.scopes {
        let glob = wcs_scaffold::expand_env(&scope.glob).replace('\\', "/");
        for red in red_lines {
            assert!(!glob_match(&glob, red), "scope {} 不应命中红线 {}", scope.id, red);
        }
    }
}
```

## 8. 新增规则清单

1. 在 `rules/` 新建 `<id>.toml`。
2. 在 `crates/scaffold/tests/` 新建 `<id>_safety.rs`（4 个测试）。
3. 运行 `cargo test -p wcs-scaffold` 确认全绿。
4. `scanary --rules-dir rules rules` 确认新规则被加载。
