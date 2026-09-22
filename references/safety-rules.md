# 安全红线（Safety Rules）

> 本文件是 Win-C-Scanary 的**安全契约**。任何破坏性操作前请通读。
> 引擎通过两层防线保障安全：**门禁（wcs-guard）** + **规则红线（scaffold）**。

## 1. 门禁裁决（wcs-guard）

`check_path` 对每个路径返回四档裁决之一：

| 裁决 | 含义 | 行为 |
| --- | --- | --- |
| `Pass` | 安全 | 允许操作 |
| `Warn` | 用户数据区 | 需用户确认后再操作 |
| `Block` | 禁止 | **拒绝**，fail-closed |
| `Skip` | 跳过 | 由调用方决定 |

**fail-closed 原则**：任何不确定的情况一律 Block —— 宁可误拦，不可误删。

### 1.1 永远禁止（FORBIDDEN → Block）

以下路径片段**永远拒绝**（大小写不敏感，命中即 Block）：

| 片段 | 说明 |
| --- | --- |
| `\windows\system32` | 系统核心 |
| `\windows\syswow64` | 系统核心（32 位） |
| `\program files` | 程序安装目录 |
| `\program files (x86)` | 程序安装目录（32 位） |
| `\$recycle.bin` | 回收站 |
| `\system volume information` | 卷影/还原信息 |
| `.vscode` | VS Code 数据 |
| `.vscode-server` | VS Code Server 数据 |
| `\jetbrains\` | JetBrains 全系列数据 |
| `\code\user` | VS Code 用户数据 |
| `\.git\` | 版本控制 |

### 1.2 危险根（is_forbidden_root → Block）

以下**根路径本身**被拒绝（不允许把整盘/整目录作为清理目标）：

- `C:\`（盘符根）
- `C:\Windows`
- `C:\Users`
- `C:\Windows\System32`

### 1.3 用户数据区（USER_DATA → Warn）

位于以下目录的路径会 **Warn**，需用户确认：

- `\Documents\`、`\Pictures\`、`\Videos\`、`\Desktop\`、`\Music\`

> 这些**不再**出现在 FORBIDDEN 中，只走 Warn，避免"一律误拦"。

⚠️ **重要（实现语义）**：`wcs-executor` 目前**只强制拦截 `Block`，不拦截 `Warn`**——
Warn 项会被放行执行（`execute` 内部对 Warn 记一条 `tracing::warn` 日志，但不阻止）。
因此"用户数据区需确认"**依赖调用方/Agent 自觉遵守**，不能指望 executor 兜底。
主要防线是：**默认 dry-run** + **预览先行** + SKILL.md 铁律"未经用户确认不真删"。
Agent 遇到 Warn 项时必须**停下向用户确认**，不得直接 `--dry-run=false`。

### 1.4 其他检查

| 检查 | 裁决 |
| --- | --- |
| 相对路径 | Block（要求绝对路径） |
| 超出 `allowed_roots`（默认 `C:\`） | Block |
| 祖先含 junction / 符号链接（reparse point） | Block |

## 2. 规则红线（wcs-scaffold）

规则 TOML 的每个 `scope.glob` 在**加载时**自动校验，命中红线即**拒绝加载**（`validate_red_lines`）。

### 2.1 RED_LINE_PATTERNS

| 红线片段 | 保护对象 |
| --- | --- |
| `*.db` / `*.db-wal` / `*.db-shm` | 数据库 |
| `**/db_storage/**` | 微信数据库 |
| `**/Msg/**` | 微信聊天记录 |
| `**/MultiMsg/**` | 微信合并消息 |
| `**/Accounts/**` | 账号数据 |
| `**/login/**` | 登录凭据 |
| `**/Favorite*/**` / `**/Fav/**` | 收藏 |
| `**/key/**` | 密钥 |
| `**/crypto/**` | 加密数据 |
| `**/.git/**` | 版本控制 |
| `**/.vscode/**` | VS Code |
| `**/JetBrains/**` | JetBrains |

### 2.2 校验逻辑

- 取 scope glob 的小写形式
- 对每个红线 pattern 去壳（去掉 `**/`、`/*`）后判断是否被 glob **包含**
- 命中 → 该规则加载失败（编译期/safety test 可见）

## 3. 执行红线（wcs-executor）

| 约束 | 说明 |
| --- | --- |
| 强制过 guard | 任何破坏性操作前必须 `wcs_guard::check_path` 通过 |
| 默认回收站 | `delete` 模式仅在用户显式选择时使用 |
| undo 日志 | 每次操作写 undo 日志，可追溯 |
| 空列表保护 | `allowed` 为空时不调用 trash API，直接返回未执行 |
| 释放量统计 | 删除前统计大小，写入 `bytes_freed` / `total_bytes` |

## 4. 指导型命令（只给命令，零执行）

以下命令**只输出建议命令文本，不执行任何系统改动或卸载**。Agent 应把输出原样转达用户，由用户自行决定与操作：

| 命令 | 内容 | 风险标注 |
| --- | --- | --- |
| `migrate` | 缓存迁移命令（junction / 环境变量） | 建议型，零风险 |
| `hiber` | `powercfg /h off\|on\|/size 40` | 可逆，需管理员 |
| `system` | DISM `AnalyzeComponentStore`/`StartComponentCleanup`/`/ResetBase` + `cleanmgr` | ⚠️ `/ResetBase` 与 Windows.old 删除**不可逆** |
| `uninstall` | 已装软件清单 + 官方卸载入口 | 引导型，**不读不执行 `UninstallString`** |

**关键红线**：
- **WinSxS 绝不可手工删除**（含硬链接，会导致系统损坏/无法更新/无法启动），只能用 DISM。
- `/ResetBase` 会永久删除更新回滚能力；Win11 24H2/25H2 上还可能导致后续累积更新失败。
- 删除 Windows.old 不可逆，需确认新系统稳定、数据已迁移。
- 关闭休眠 `powercfg /h off` 可逆（`/h on` 恢复），但属系统改动，需管理员。

## 5. 给 Agent 的操作建议

1. **清理前**：先 `preview`（dry-run），把命中项展示给用户。
2. **执行时**：默认 `--dry-run=true`，用户确认后再 `--dry-run=false`。
3. **Warn 项**：必须向用户解释影响，获得明确同意。
4. **Block 项**：不要尝试绕过；报告"该路径受保护"。
5. **新增规则**：glob 不得命中任何红线，且必须配 safety test（正向 + 红线断言）。
6. **指导型命令**：原样转达输出的命令文本，**不要代替用户执行**。

## 6. 路径短名（8.3）注意

Windows 的 `%TEMP%` 等变量在部分机器上展开为 **8.3 短名**（如 `C:\Users\ADMINI~1\...`）。
规则 glob 展开为**长名**，直接字符串匹配会失败（`matched 0`）。

- 规则侧：`expand_env` 已做短名→长名转换（T-PATH-1）。
- **用户传入的 root 路径**：`match_scope` 现也会对条目路径做短名→长名归一化（2026-09-21 修），
  故 `preview/execute "%TEMP%" ...` 即使 `%TEMP%` 是短名也能正确命中。
- 若仍遇到 `matched 0`，可显式用长路径（`GetFullPath`）重试。

## 7. 红线清单的权威来源

| 防线 | 代码位置 |
| --- | --- |
| 门禁禁止路径 | `crates/guard/src/lib.rs` → `FORBIDDEN_PATTERNS` |
| 门禁用户数据区 | `crates/guard/src/lib.rs` → `USER_DATA_PATTERNS` |
| 危险根 | `crates/guard/src/lib.rs` → `is_forbidden_root` |
| 规则红线 | `crates/scaffold/src/lib.rs` → `RED_LINE_PATTERNS` |

> 文档与代码不一致时，**以代码为准**。
