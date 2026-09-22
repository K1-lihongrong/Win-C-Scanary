# Win-C-Scanary

> 面向 Windows C 盘的安全扫描与清理工具。**Skill 优先、GUI 辅助**，用 Rust 打造。
> 既能作为 **Agent Skill** 被 AI 调用完成复杂清理决策，也能作为**薄 GUI** 引导用户日常清理。

## 特性

| 能力 | 说明 |
|---|---|
| 秒级扫描 | NTFS MFT 直读（全盘 ~4.8s，需管理员），失败自动降级 walk |
| 规则驱动 | 清理规则外置为 TOML，加规则不改代码 |
| 三层安全 | guard 门禁 + 规则红线 + safety test，默认回收站、预览先行、fail-closed |
| 全功能 | 扫描/清理/查重/软件清单/空间核算/增长追踪/迁移/休眠/系统清理/卸载指导 |
| 双形态 | Agent Skill（SKILL.md）+ 薄 GUI（浏览器） |

## 快速开始

```bash
# 构建（release 性能达标）
cargo build --release

# 权限状态（判断能否用 MFT）
scanary permissions

# 全盘扫描，看空间大头
scanary --format pretty scan "C:\" --top 30

# 健康报告
scanary --format pretty report "C:\"

# 列出规则 / 预览 / 执行（默认 dry-run）
scanary --rules-dir rules rules
scanary --rules-dir rules preview system-temp "%TEMP%"
scanary --rules-dir rules execute system-temp "%TEMP%" --dry-run=false

# 薄 GUI
scanary gui
```

> `--format` 须放在子命令**之前**。`--rules-dir` 默认查找 exe 同级 `rules/` 或当前目录 `rules/`。

## 命令一览（18）

| 类别 | 命令 |
|---|---|
| 核心 | `scan` `inspect` `rules` `preview` `execute` `report` `permissions` |
| 分析 | `dedup`（查重）`inventory`（软件清单）`audit`（空间核算）`snapshot`/`grow`（增长追踪） |
| 管理 | `quarantine`（隔离区）`gui`（界面） |
| 指导型（只给命令） | `migrate` `hiber` `system` `uninstall` |

## 安全

- **默认 dry-run**、**回收站兜底**、**预览先行**；未经用户确认不真删。
- 受保护路径（System32 / Program Files / .git 等）由 guard **fail-closed 拦截**。
- 每条规则配 safety test（正向 + 红线断言）。
- **指导型命令**（迁移/休眠/系统清理/卸载）只输出命令文本，**不执行任何系统改动**。
- WinSxS 绝不可手删，只能用 DISM。

详见 `references/safety-rules.md`。

## 作为 Agent Skill 使用

把 `SKILL.md` + `references/` 放到 Agent 的 skill 目录。触发词见 SKILL.md description
（"C盘清理 / 磁盘清理 / 空间不足 / disk cleanup / storage full / 清理缓存" 等）。

## 文档

| 文档 | 内容 |
|---|---|
| `SKILL.md` | Agent 入口（薄） |
| `references/safety-rules.md` | 安全红线完整清单 |
| `references/workflow.md` | 完整命令工作流 |
| `references/rule-format.md` | TOML 规则格式 + safety test 模板 |
| `docs/HANDOFF-PARALLEL.md` | 任务状态 + 项目现状（权威） |
| `docs/DESIGN.md` | 设计规格书 |
| `docs/VERIFY-AGENT-LOAD.md` | V14 真实 Agent 验证记录 |
| `CHANGELOG.md` | 变更日志 |

## 环境

- Windows 10 / 11（x64）
- 普通权限即可；MFT 秒级扫描与系统级操作需管理员
- 纯本地，无网络依赖

## 许可

MIT
