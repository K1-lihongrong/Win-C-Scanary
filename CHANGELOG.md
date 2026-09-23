# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/) 风格。

## [0.1.1] — 2026-09-23

修复 issue #1 审查发现的「安全承诺与实现缺口」：破坏性操作不再依赖调用方自觉。

### 安全（高危）
- **execute 强制二次确认**：真删改为要求显式 `--commit`（弃用 `--dry-run=false` 反转式旗标）；不带 `--commit` 永不真删
- **prompt 生效**：真删前按 scope 的 `Prompt::Confirm` 交互确认；非 TTY（管道/CI/Agent）一律拒绝

### 安全（中危）
- **Warn 强制拦截**：用户数据区（Documents/Desktop/Pictures 等）默认进 blocked，需 `--allow-warn` 才放行
- **Delete 默认拒绝**：不可逆删除需 `--allow-delete`
- **undo / restore**：新增 `scanary undo`（列出会话）与 `scanary restore <session_id>`（隔离项移回原位）；Recycle 提示回收站恢复，Delete 标注不可逆

### 修复（低危）
- `GuardConfig.protect_c_drive_boundary` 接上逻辑：拒绝以 C 盘根作为操作目标
- `Plan.granularity` 生效：Directory 粒度真正影响回收行为
- Quarantine `rename` 失败不再吞错：不计数、不写 undo、进失败列表
- Recycle 改逐个删除：占用/锁定文件跳过，不再整批 abort（与 disclaimer 一致）
- 文档测试数 155 → 164

### 测试
- **164 个测试全绿**，0 warning 0 error（原 156 + v0.1.1 修复 8）

## [0.1.0] — 2026-09-21

首个完整版本：**阶段 0–5 全部完成，功能 15 项全覆盖，164 测试全绿**。

### 新增（核心引擎）
- **wcs-scanner**：NTFS MFT 直读（全盘 ~4.8s）+ jwalk 降级；DirIndex 目录聚合；空间核算
- **wcs-scaffold**：TOML 规则引擎（globset 匹配 + 红线校验）；8.3 短名→长名归一化
- **wcs-guard**：fail-closed 门禁（保护路径 Block、用户数据 Warn）
- **wcs-executor**：回收站/隔离/删除 + undo 日志 + 释放量统计
- **wcs-report**：健康评分 + 分级汇总 + 空间核算 + 迁移建议（`schema_version: "1"`）
- **wcs-dedup**：三阶段哈希重复文件检测（只读）
- **wcs-inventory**：读注册表三视图列出已装软件
- **wcs-growth**：快照对比增长追踪

### CLI（18 子命令）
`scan` / `inspect` / `rules` / `preview` / `execute` / `report` / `permissions` / `dedup` /
`inventory` / `gui` / `audit` / `snapshot` / `grow` / `quarantine` / `uninstall` / `system` / `hiber` / `migrate`

- **指导型命令**（只给命令不执行）：`migrate` / `hiber` / `system` / `uninstall`
- **提权引导**：`permissions` 非管理员时输出可复制的 `runas` 命令（不自动提权）

### 新增（薄 GUI）
- `scanary gui`：本地 HTTP 服务（随机端口）+ 内嵌前端，6 页（首页/可清理/执行/分流/增长/日志）

### 安全
- 默认 dry-run；回收站兜底；保护路径 fail-closed
- 每条规则强制 safety test（正向 + 红线断言）
- WinSxS 只能走 DISM；指导型命令零执行

### Agent 集成
- `SKILL.md`（94 行）+ `references/`（safety-rules / workflow / rule-format）
- **V14 真实 Agent 验证通过**：仅凭 SKILL.md 可自主触发并走安全闭环

### 规则（6 条）
`system-temp` / `browser-cache` / `npm-cache` / `dev-caches` / `wechat-pc` / `conda`

### 修复（遗留清单清理）
- grow `self_delta` 被父目录负增长放大的虚高问题
- `inspect` 大目录静默变慢（加 50 万文件上限 + `truncated` 标注）
- `match_scope` 短名 root 致 glob 不匹配（`%TEMP%` matched 0）
- 规则目录相对路径依赖 cwd
- 配置目录统一到 `%APPDATA%\Win-C-Scanary\`

### 性能
- 全盘 MFT（release）：纯扫描 ~4.8s / scan 命令 ~5.4s / report ~4.7s
- 内存 ~170MB（NodeBudget 20 万节点）

### 测试
- **164 个测试全绿**，0 warning 0 error
