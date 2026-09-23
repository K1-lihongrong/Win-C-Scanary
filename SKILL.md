---
name: win-c-scanary
description: 安全扫描并清理 Windows C 盘空间。基于规则驱动（规则即 TOML，可扩展），默认回收站、fail-closed 门禁、预览先行。覆盖系统临时文件、浏览器缓存、npm/pip/cargo/maven/gradle/conda 缓存、微信 PC 缓存；并提供重复文件检测、软件清单/卸载指导、空间核算、增长追踪、迁移指导、休眠/页面文件、系统级清理（后三者为指导型，只给命令不执行）。当用户提到 C盘清理、磁盘清理、空间不足、C盘满了、disk cleanup、free up space、clean disk、storage full、释放空间、垃圾文件、清理缓存、查重、重复文件、迁移缓存、关闭休眠、WinSxS、卸载软件，或要求查找磁盘上的垃圾/临时/缓存文件时触发。
version: 0.1.1
---

# Win-C-Scanary

> **规则驱动、安全优先的 C 盘清理引擎。** 扫描 → 预览 → （确认）→ 回收站清理 → 看释放量。
> 引擎只管"找到并安全移动"，**判断交给 Agent**（无内置 AI）。

## 何时使用

C 盘空间不足/变红、主动清理、找垃圾或缓存、释放空间；英文 "disk cleanup / free up space / storage full / clear cache"。

## 运行要求

Windows 10 / 11；普通权限即可（清理 %WINDIR%\Temp 建议管理员）；纯本地、无外部依赖。

## 核心工作流

> 默认**预览先行、回收站兜底**。执行前必须用户确认。

```bash
# 0. 权限状态（判断能否用 MFT 快速扫描）
scanary permissions

# 1. 扫描磁盘/目录（quick）
#    MFT 全盘：size/count 精确；深层明细受 20 万节点预算限制（顶层 top_dirs 不受影响）
scanary --format pretty scan "C:\" --top 30

# 2. 目录深挖（找大文件夹，只读元数据，不读文件内容）
scanary inspect "C:\Users\me\AppData"

# 3. 列出可用规则
scanary --rules-dir rules rules

# 4. 预览某规则会命中什么（dry-run，不删）
scanary --rules-dir rules preview npm-cache "%APPDATA%\npm-cache"

# 5. 执行清理（默认 dry-run；真删必须显式 --commit）
#    真删前会按规则 prompt 交互确认（非 TTY 会拒绝）
scanary --rules-dir rules execute system-temp "%TEMP%" --commit

# 6. 健康报告 + 释放量
scanary report "C:\"

# 6.5 撤销：查看历史会话 / 恢复隔离项
scanary undo                    # 列出可恢复会话
scanary restore <session_id>    # 隔离项移回原位（Recycle 用回收站，Delete 不可逆）

# 7. 进阶：重复文件检测（只检测不删）
scanary dedup "C:\Users\me\Downloads" --min-size 1024

# 8. 进阶：已安装软件清单 / 卸载指导（只列清单 + 给官方入口）
scanary inventory
scanary uninstall --top 20

# 9. 进阶：空间核算（逻辑 vs 实际分配，找簇对齐浪费）
scanary audit "C:\" --top 20

# 10. 进阶：增长追踪（快照对比，找增长目录）
scanary snapshot "C:\"    # 采集快照；scanary grow --top 20 对比最近两份

# 11. 指导型（只给命令，不执行）：
scanary hiber              # 休眠/页面文件 → powercfg 命令
scanary system             # 系统级清理 → DISM / cleanmgr 命令
scanary migrate "C:\"     # 迁移指导 → junction / 环境变量命令

# 12. 进阶：薄 GUI（浏览器界面，随机端口）
scanary --rules-dir rules gui
```

## 安全红线（摘要）

- 破坏性操作一律 **fail-closed**：不确定 → Block
- **默认回收站**，delete 模式仅在用户显式要求时使用
- 绝不清理：IDE 数据、.git、用户文档、账号/聊天/密钥
- 路径校验由 wcs-guard 强制；命中红线规则**拒绝加载**
- **指导型命令**（`hiber` / `system` / `migrate` / `uninstall`）只输出建议命令，**不执行任何系统改动或卸载**；WinSxS 绝不可手删，只能用 DISM

完整清单见 [references/safety-rules.md](references/safety-rules.md)。

## 参考文档

| 文档 | 内容 |
| --- | --- |
| [references/safety-rules.md](references/safety-rules.md) | 门禁保护路径 + 规则红线完整清单 |
| [references/workflow.md](references/workflow.md) | 完整命令工作流与参数 |
| [references/rule-format.md](references/rule-format.md) | TOML 规则格式规范 + safety test 模板 |

## 铁律

1. **先预览，后执行**；未经用户确认不真删。
2. 清理前先 permissions 判断权限，优雅降级。
3. 用户报告释放空间时，用执行结果里的 bytes_freed，不要估算。
4. 输出语言与用户一致（中文/英文）。
5. 规则即数据：加规则改 rules/*.toml + safety test，不改代码。
