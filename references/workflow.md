# 完整工作流（Workflow）

> Win-C-Scanary CLI 名：`scanary`。全局参数：`--rules-dir`、`--undo-log`、`--quarantine-root`、`--format <json|pretty>`（须放在子命令**之前**）。
> 环境变量前缀 `WCS_`（如 `WCS_RULES_DIR`）。

## 0. 权限状态

```bash
scanary permissions
```

输出当前用户是否管理员（`is_admin`）、能否打开卷设备（`can_open_volume`）、能否用 MFT 直读（`can_use_mft`），并给出可读建议。

- 非管理员：MFT 走降级 walk 路径，功能可用、速度较慢。
- 管理员：可使用 MFT 快速扫描全盘。
- **提权引导**：非管理员时额外输出 `elevate_command`（可复制执行的 `powershell Start-Process -Verb RunAs` 命令），以管理员重开后 MFT/系统级操作才可用。**本工具不自动提权**。

## 1. 扫描（scan）

```bash
scanary --format pretty scan <PATH> [--top N]
```

- 扫描磁盘或目录，输出大小/文件数/扩展名统计。
- `--top N`：列出最大的 N 个条目（默认 50）。
- 只读元数据，**不读文件内容**。
- **MFT 全盘扫描**：size/file_count 精确；但为防内存爆，Node 树有 20 万节点预算，**超出预算的深层目录只聚合大小/数量，不保留明细**（顶层 `top_dirs` 不受影响）。

示例：

```bash
scanary --format pretty scan "C:\" --top 30
scanary --format pretty scan "crates" --top 5
```

## 2. 目录深挖（inspect）

```bash
scanary inspect <PATH> [--samples N]
```

- 提取目录元数据：路径/大小/文件数/扩展名分布/样本文件。
- `--samples N`：每类返回的样本数（默认 20）。
- 用于定位"哪个文件夹占空间"，**不读文件内容**。
- **规模保护**：遍历超过 50 万文件即停止深入，结果标 `truncated: true`（此时 size/file_count 为部分值）。
- **适用边界**：适合**单个小/中目录**深挖；对整个盘或超大目录做统计请用 `scan`。

## 3. 列出规则（rules）

```bash
scanary --rules-dir rules rules
```

列出当前加载的全部规则（id/name/risk/detect/scopes）。

## 4. 预览（preview，dry-run）

```bash
scanary --rules-dir rules preview <RULE_ID> <PATH> [--scope SCOPE_ID]
```

- 展示某规则在某路径下**会命中哪些文件/目录**，不删除。
- `--scope`：只预览指定 scope。
- **执行前必须先 preview**，把结果展示给用户。

示例：

```bash
scanary --rules-dir rules preview npm-cache "%APPDATA%\npm-cache"
scanary --rules-dir rules preview system-temp "%TEMP%" --scope user-temp
```

## 5. 执行清理（execute）

```bash
scanary --rules-dir rules execute <RULE_ID> <PATH> [--scope SCOPE_ID] [--dry-run <true|false>]
```

- **默认 `--dry-run=true`**（只演示，不真删）。
- 用户确认后才传 `--dry-run=false`。
- 执行前会先对 `PATH` 做 guard 校验；受保护路径会被 **Block**。
- 默认回收站（`mode = "recycle"`），文件可恢复。
- 输出每项释放量 `bytes_freed` 与总计 `total_bytes`。

示例：

```bash
# 演示
scanary --rules-dir rules execute system-temp "%TEMP%" --dry-run=true
# 真删（用户已确认）
scanary --rules-dir rules execute system-temp "%TEMP%" --dry-run=false
# 受保护路径应被 Block
scanary --rules-dir rules execute system-temp "C:\Windows\System32" --dry-run=true
```

## 6. 重复文件检测（dedup）

```bash
scanary dedup <PATH> [--min-size N]
```

- 三阶段哈希（size → head-hash → full-hash SHA-256），**只检测不删除**。
- `--min-size N`：忽略小于 N 字节的文件。
- 输出重复组（大小/哈希/路径/可回收字节），按浪费空间降序。
- 用 `--format pretty` 看人类可读列表。

示例：

```bash
scanary dedup "C:\\Users\\me\\Downloads" --min-size 1024
scanary --format pretty dedup "D:\\Photos"
```

> 注意：`--format` 是全局参数，须放在子命令**之前**。

## 7. 软件清单（inventory）

```bash
scanary inventory
```

- 读取三个卸载视图：HKLM 64 位、HKLM Wow6432Node 32 位、HKCU per-user。
- 列出软件名/版本/发布者/安装位置/估算大小，per-user 项带标记。
- 跳过无 `DisplayName` 的项，按 (name, version) 去重。

示例：

```bash
scanary --format pretty inventory
```

## 8. 健康报告（report）

```bash
scanary report <PATH>
```

- 生成健康评分（0–100）、已用比例、可用/总空间、规则风险分级汇总。
- 自动解析盘符 + 加载规则 + 打标 + 构建报告。

示例：

```bash
scanary report "C:\"
```

## 9. 空间核算（audit）

```bash
scanary audit <PATH> [--top N]
```

- 对比**逻辑大小**与**实际分配**（NTFS 簇对齐），找出簇对齐浪费（`self_waste`）。
- 需 MFT 直读（管理员 + NTFS 卷）。
- 输出：全盘逻辑/实际/浪费 + 浪费最多的目录。

示例：

```bash
scanary --format pretty audit "C:\\" --top 20
```

## 10. 增长追踪（snapshot / grow）

```bash
scanary snapshot [PATH] [--min-size N] [--keep N]   # 采集快照（默认 C:\\）
scanary grow [--top N]                               # 对比最近两份快照
```

- `snapshot`：采集目录大小快照，存到 `%APPDATA%\Win-C-Scanary\snapshots`（1MB 阈值，保留 10 份）。
- `grow`：对比最近两份快照，按 `self_delta`（本层新增）列出增长最多的目录。
- 需至少 2 份快照才能对比；建议定期采集。

示例：

```bash
scanary snapshot "C:\\"
scanary --format pretty grow --top 20
```

## 11. 迁移指导（migrate）—— 指导型

```bash
scanary migrate [PATH] [--top N] [--target D]
```

- 分析适合迁移到其他盘的大缓存目录，给出迁移命令。
- 两种方式：`EnvVar`（npm/cargo/pip/conda/gradle/maven，改环境变量）+ `Junction`（`robocopy /MOVE` + `mklink /J`）。
- 目标路径按 `<目标盘>:\wcs-migrate\<rule_id>\<scope_id>` 唯一命名。
- **纯建议，不执行任何迁移**。

## 12. 休眠/页面文件（hiber）—— 指导型

```bash
scanary hiber [PATH]
```

- 检测 `hiberfil.sys` / `pagefile.sys` / `swapfile.sys` 大小。
- 以 `hiberfil.sys` 是否存在判定休眠是否启用。
- 给出 `powercfg /h off|on|/size 40` 命令文本（可逆，需管理员）。
- **纯建议，不执行任何系统改动**。

## 13. 系统级清理（system）—— 指导型 ⚠️

```bash
scanary system [PATH]
```

- 检测 WinSxS 组件存储 / Windows.old。
- 给出 DISM（`AnalyzeComponentStore` / `StartComponentCleanup` / `ResetBase`）与 `cleanmgr` 命令文本，逐条标安全/破坏性 + 警告。
- **最高风险区域，只给命令不执行**；WinSxS 绝不可手删，只能用 DISM。

## 14. 软件卸载指导（uninstall）—— 引导型

```bash
scanary uninstall [--top N]
```

- 列出已装软件（复用 inventory，按估算大小降序）。
- 给出官方卸载入口：设置→应用 / `appwiz.cpl` / `winget uninstall`。
- **不读取也不执行 `UninstallString`**。

## 15. 薄 GUI（gui）

```bash
scanary --rules-dir rules gui
```

- 起本地 HTTP 服务（随机端口）→ 打印 URL → 自动开浏览器。
- 6 页：首页/可清理/执行/分流/增长/日志。

## 16. 隔离区管理（quarantine）

```bash
scanary quarantine --list          # 列出隔离区文件
scanary quarantine --days 7        # 删除超过 7 天的隔离项（默认）
```

- 隔离区默认在 `%APPDATA%\Win-C-Scanary\quarantine`。
- 隔离项由 `mode = "quarantine"` 的规则产生（文件被移入而非删除）。
- 清理按文件名时间戳判断；无法解析时间戳的文件**保守保留**。

## 推荐闭环

```
permissions → scan → inspect → rules → preview → (用户确认) → execute → report
```

1. `permissions`：判断权限，决定是否能用 MFT。
2. `scan` + `inspect`：找到空间占用大头。
3. `rules` + `preview`：看哪些规则能命中、能释放多少。
4. 展示给用户，确认清理范围。
5. `execute --dry-run=false`：回收站清理。
6. `report`：看释放量与健康评分变化。

> 高阶能力（可选，视用户意图）：`dedup`（查重）、`audit`（空间核算）、`snapshot`+`grow`（增长追踪）；
> 指导型（只给命令不执行）：`migrate` / `hiber` / `system` / `uninstall`。

## 参数速查

| 参数 | 适用命令 | 说明 |
| --- | --- | --- |
| `--rules-dir <DIR>` | 全局 | 规则目录（`rules/`） |
| `--undo-log <FILE>` | 全局 | undo 日志路径 |
| `--quarantine-root <DIR>` | 全局 | 隔离区根目录 |
| `--format <json|pretty>` | 全局 | 输出格式（默认 json） |
| `--top <N>` | scan | 最大条目数 |
| `--samples <N>` | inspect | 样本数 |
| `--scope <ID>` | preview/execute | 只处理指定 scope |
| `--dry-run <bool>` | execute | 默认 true |
| `--min-size <N>` | dedup / snapshot | 忽略小于 N 字节的文件/目录 |
| `--top <N>` | scan / audit / grow / migrate / uninstall | 列出/建议条数 |
| `--keep <N>` | snapshot | 保留快照份数（默认 10） |
| `--target <D>` | migrate | 建议迁移到的目标盘符（默认 D） |

## 子命令一览（18）

`scan` / `inspect` / `rules` / `preview` / `execute` / `report` / `permissions` / `dedup` / `inventory` / `gui` / `audit` / `snapshot` / `grow` / `quarantine` / `uninstall` / `system` / `hiber` / `migrate`

> 指导型（只输出命令、不执行）：`migrate` / `hiber` / `system` / `uninstall`。
