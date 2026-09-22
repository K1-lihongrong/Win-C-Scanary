# Win-C-Scanary 设计规格书

> 版本：1.1（对齐实现）
> 日期：2026-09-20
> 状态：阶段 0–5 全部完成（本文档为设计规格书，实现差异见 0 节）
> 代号：win_c_scanary

---

## 0. 实现对齐说明（v1.1 新增）

> 本文档最初写于设计阶段。阶段 1 落地后，实现与初稿存在若干偏差。
> **本节是权威的差异清单**；下文正文保留设计原貌，若与本节冲突，以本节 + 代码为准。

### 0.1 目录/文件结构差异

| 设计初稿 | 实际实现 |
|---|---|
| scanner: `lib.rs` / `mft.rs` / `walk.rs` / `node.rs` | 另增 `aligned_reader.rs`（扇区对齐读取）、`perm.rs`（权限探测），并含 `examples/mft_verify.rs` |
| scaffold: `lib.rs` / `model.rs` / `loader.rs` / `matcher.rs` | 实际仅 `lib.rs` + `model.rs`；加载/匹配逻辑内联在 `lib.rs` |
| guard: `lib.rs` / `rules.rs` | 实际仅 `lib.rs`；规则常量内联（`FORBIDDEN_PATTERNS` 等） |
| executor: `lib.rs` / `action.rs` / `undo.rs` | 实际仅 `lib.rs`；action/undo 内联 |
| report: `lib.rs` / `json.rs` / `pretty.rs` / `health.rs` | 实际仅 `lib.rs`；JSON/pretty/health 内联 |
| 测试：根 `tests/` | 实际用**每 crate 的 `tests/`**（如 `crates/scaffold/tests/`、`crates/scanner/tests/`） |
| 无 | 实际新增 `crates/dedup/`（三阶段哈希，已实现）、`crates/inventory/`（读注册表，已实现） |

### 0.2 类型/API 差异

| 设计初稿 | 实际实现 |
|---|---|
| `ScanOptions { follow_symlinks, max_depth, keep_files_per_dir, prefer_mft }` | 另增 `mft_max_records: Option<u64>`（None=全量；Some(n)=限制记录数以加速） |
| `scan_with_stats` 仅检查 `can_use_mft` | 另需 `is_drive_root(root)` 为真才尝试 MFT（子目录不支持）；调用前后临时静默 panic hook |
| scanner 无 `is_admin` / `disk_space` | 新增 `pub fn is_admin() -> bool` 与 `pub fn disk_space(volume: char) -> Option<DiskSpace>` |
| scanner 无 `DiskSpace` 类型 | 新增 `DiskSpace { total_bytes, free_bytes, used_bytes }` |
| scanner 无对齐读取 | 新增 `aligned_reader::AlignedReader`（Read+Seek 适配器，解决卷设备 os error 87） |
| scanner 无记录解析 | 新增 `mft_record::parse_record`（手工解析 FILE 记录，供顺序读 $MFT $DATA） |
| MFT 逐条 `ntfs.file()` | 改为顺序读整个 $MFT $DATA 流 + 大块读（4MB 缓冲，按 record_size 对齐）；全量 release 纯 MFT ~4.8s（scan 命令 ~5.4s） |
| MFT size 含系统元文件/ADS | 跳过记录号 ≤15 的元文件，只计未命名 $DATA；size 68TB→154GB |
| `report::build(tree, rules, scan_stats)` | 实际为 `build(tree, scan_stats)` + `build_with_disk(tree, scan_stats, disk, rules)` |
| `Report` 含 `sections` | 实际用 `top_dirs`，无 `sections` |
| `Summary` 两字段 | 实际三字段：`safely_cleanable_gb` / `needs_confirm_gb` / `high_risk_gb` + `top_dirs` |
| executor 无释放量 | `ExecResult` 含 `total_bytes`，`UndoEntry` 含 `bytes_freed`（真实统计） |
| guard 无 `has_reparse_ancestor` 公开 | 实际公开 `has_reparse_ancestor`，且 `GuardConfig` 含 `reject_reparse_points` |
| 无目录聚合能力 | 新增 `mft::DirIndex`（后序 DFS 收集目录路径+子树 size）+ `find_dirs`（前缀/通配匹配）；`scan_volume` 返回 `(Node, DirIndex)` |
| `scan_with_stats` 返回 `(Node, ScanStats)` | 新增 `scan_with_stats_indexed` 返回 `(Node, ScanStats, Option<DirIndex>)`；旧函数保留为薄包装 |
| scaffold 无目录前缀提取 | 新增 `scope_dir_prefix`（glob → 目录前缀，展开 env + 短名→长名 + 去尾部通配） |
| `expand_env` 仅展开变量 | 追加 8.3 短名→长名转换（`GetLongPathNameW`），修复 `%TEMP%` 短名与 MFT 长名不匹配 |
| report 仅 `build_with_disk` | 新增 `build_with_index`（用 `DirIndex` 按 scope 前缀精确累加分级量 + `sum_dedup` 去重） |
| 无 GUI | 新增 `scanary gui`（`crates/cli/src/gui.rs`）：本地 HTTP 服务（tiny_http，随机端口）+ 内嵌前端；6 个 API |

### 0.3 已落地的规则（6 条）

`system-temp` / `browser-cache` / `npm-cache` / `dev-caches` / `wechat-pc` / `conda`。
每条均配 safety test（正向 + 红线断言），见 `crates/scaffold/tests/*_safety.rs`。

### 0.4 测试基线

阶段 1 闭合后共 **55 个测试**全绿（scaffold 24 + guard 14 + executor 9 + report 8）；
补 scanner 测试（18）、dedup 测试（8）、inventory 测试（5）、mft_record 解析测试（5）后增至 **91 个**。
后续：GUI 安全策略（3）+ `DirIndex`（6）+ report `sum_dedup`（3）+ `scope_dir_prefix`（5）→ **108 个**。
阶段 3–4 完成后增至 **140 个**：报告分级/DirIndex 查询（+16）、短名修复（+1）、scan_volume 集成（+4）、空间核算（+2）、增长追踪（+4）、迁移指导（+8）、休眠（+7）、系统级清理（+7）等。
阶段 5 + 遗留清理后 **149 个**：软件卸载（+6）、grow 回归（+2）、inspect 截断（+1）。

---

## 目录

- [第 1 部分：概述与架构](#第-1-部分概述与架构)
- [第 2 部分：Crate 模块设计](#第-2-部分crate-模块设计)
- [第 3 部分：CLI 接口设计](#第-3-部分cli-接口设计)
- [第 4 部分：规则格式规范](#第-4-部分规则格式规范)
- [第 5 部分：安全模型与权限策略](#第-5-部分安全模型与权限策略)
- [第 6 部分：GUI 设计、功能清单与路线图](#第-6-部分gui-设计功能清单与路线图)

---

# 第 1 部分：概述与架构

## 1.1 项目定位

**Win-C-Scanary** 是一个面向 Windows C 盘的**磁盘扫描与安全清理工具**，采用"**Skill 优先、GUI 辅助**"的双形态设计。

一句话概括：**用 Rust 打造的高性能磁盘扫描引擎，既能作为 Agent Skill 被 AI 调用完成复杂清理决策，也能作为薄 GUI 引导人类用户完成日常清理。**

### 核心差异化

| 能力 | 传统工具 | Win-C-Scanary |
|---|---|---|
| 扫描速度 | 慢（遍历 + stat） | **MFT 直读秒级**（需提权时降级） |
| 决策能力 | 固定规则 | **Agent 判断**（Skill 侧）/ 引导（GUI 侧） |
| 安全机制 | 简单确认 | **三层防护 + 强制 safety test** |
| 形态 | 单一 | **Skill + GUI 双形态，引擎共享** |
| 规则扩展 | 改代码 | **TOML 数据驱动** |

### 设计理念（博采众长）

本项目刻意吸收了四个参考项目的精华：

| 借鉴点 | 来源 | 在本项目的体现 |
|---|---|---|
| 薄 SKILL.md + references 分离 | windows-disk-cleanup | SKILL.md < 100 行，细节进 references |
| 三色安全分级 | windows-disk-cleanup | 规则 risk 分级 + 报告分级 |
| scaffold TOML 数据驱动 | pinkbin | rules/*.toml |
| safety test 强制 | pinkbin | 每规则配正向+红线断言 |
| 默认回收站 + undo | pinkbin / disk-cleanup | executor crate |
| 统一删除门禁 | c-drive-cleaner | 独立 guard crate，fail-closed |
| 预览先行协议 | disk-cleanup | preview → execute 强制流程 |
| 功能广度 | c-drive-cleaner | 15 项功能分三层 |
| NTFS MFT 快速扫描 | pinkbin | scanner crate |
| 报告 + 健康评分 | c-drive-cleaner | report crate |

**同时刻意规避了它们的短板**：
- 不学 c-drive-cleaner 的"SKILL.md 混入产品文档"
- 不学 disk-cleanup 的"依赖不可见预编译 exe"
- 不学 pinkbin 的"GUI 过重"
- 不学 windows-disk-cleanup 的"功能覆盖窄"

---

## 1.2 核心设计原则

1. **Skill 优先**：引擎的首要消费者是 Agent；GUI 是次要的、可替换的壳
2. **性能优先**：MFT 直读，权限不足时优雅降级而非失败
3. **安全第一**：默认只读、预览先行、默认回收站、fail-closed 门禁
4. **数据驱动**：规则外置为 TOML，加规则不改代码
5. **测试强制**：每条规则必须配 safety test，无测试不合并
6. **职责单一**：引擎只做"扫描/匹配/门禁/执行/报告"，判断层交给 Agent
7. **解耦可替换**：引擎即 CLI，GUI 是壳，可日后升级为 Tauri

---

## 1.3 系统架构总览

```
┌──────────────────────────────────────────────────────────────┐
│                        消费者层                                │
│  ┌────────────────────┐         ┌──────────────────────────┐ │
│  │  Agent Skill        │         │  薄 GUI（浏览器/可升级）   │ │
│  │  SKILL.md + CLI     │         │  扫描/展示/引导分流        │ │
│  │  【全功能】          │         │  【仅日常清理】            │ │
│  └─────────┬──────────┘         └────────────┬─────────────┘ │
│            │ 命令行调用                        │ 命令行调用      │
├────────────┴──────────────────────────────────┴─────────────┤
│                       CLI 层（scanary）                        │
│  子命令路由 · 参数解析 · 输出格式化（JSON / pretty）            │
├──────────────────────────────────────────────────────────────┤
│                        引擎层（Rust crates）                   │
│  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐ ┌────────┐│
│  │ scanner  │ │ scaffold │ │ guard  │ │ executor │ │ report ││
│  │ MFT+jwalk│ │ TOML匹配  │ │ 门禁    │ │ 执行+undo │ │ 报告   ││
│  └──────────┘ └──────────┘ └────────┘ └──────────┘ └────────┘│
├──────────────────────────────────────────────────────────────┤
│                        数据层                                  │
│  rules/*.toml（规则）· ~/.wcs/（undo/quarantine/快照）         │
└──────────────────────────────────────────────────────────────┘
```

## 1.4 数据流

### 典型清理流程（Skill 侧）

```
1. Agent 调 `scanary scan C:\ --json`
2. scanner：尝试 MFT（提权）→ 失败降级 jwalk → 返回目录树
3. Agent 分析树，识别可疑大目录
4. Agent 调 `scanary inspect <path> --json` 获取元数据
5. Agent 判断"这是什么、能否删"（Agent 自己就是判断层）
6. Agent 调 `scanary preview <rule> <path> --json` 预览匹配
7. scaffold 匹配 + guard 校验 → 返回 matched/blocked/warnings
8. Agent 把预览结果展示给用户，获确认
9. Agent 调 `scanary execute <rule> <path> --yes`
10. executor：回收站/隔离/删除 + 写 undo.jsonl
11. Agent 调 `scanary report` 或对比前后空间
```

### 典型流程（GUI 侧）

```
1. 用户双击启动 → 引擎起本地 HTTP 服务 → 开浏览器
2. 前端调后端 API（后端转调 scanary CLI）
3. 展示：C 盘健康评分 + 大目录 + 可清理项（三色分级）
4. 用户勾选日常清理项（仅 GREEN 安全项）
5. 遇到 YELLOW/complex → 提示"建议使用 Agent 分析"
6. 执行安全清理 → 展示释放结果
```

## 1.5 Crate 依赖关系

```
              wcs-cli
                │
    ┌───────────┼───────────┬────────────┐
    ▼           ▼           ▼            ▼
wcs-scanner  wcs-scaffold  wcs-guard  wcs-report
                                │
                                ▼
                          wcs-executor
                                │
                                └──> (依赖 wcs-guard 的门禁)
```

- **wcs-scanner**：无内部依赖（最底层）
- **wcs-scaffold**：无内部依赖
- **wcs-guard**：无内部依赖（独立门禁逻辑）
- **wcs-executor**：依赖 wcs-guard（执行前必须过门禁）
- **wcs-report**：依赖 scanner 的输出类型（或定义自己的视图类型）
- **wcs-cli**：聚合上述所有

## 1.6 目录结构

```
Win-C-Scanary/
├── Cargo.toml                      # workspace 定义
├── SKILL.md                        # Agent 入口（薄，<100 行）
├── CHANGELOG.md
├── LICENSE
├── README.md
│
├── crates/
│   ├── scanner/                    # 扫描
│   │   ├── Cargo.toml
│   │   ├── examples/
│   │   │   └── mft_verify.rs       # MFT 验证程序
│   │   └── src/
│   │       ├── lib.rs              # 公共 API
│   │       ├── mft.rs              # NTFS MFT 直读（Windows）
│   │       ├── aligned_reader.rs   # 扇区对齐读取适配器（修复 os error 87）
│   │       ├── perm.rs             # 权限探测（is_admin/can_use_mft/disk_space）
│   │       ├── walk.rs             # 递归遍历回退
│   │       └── node.rs             # Node 类型定义
│   ├── scaffold/                   # 规则
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs              # 加载 + globset 匹配 + 红线校验
│   │   │   └── model.rs            # Rule/Scope/Mode/Prompt 类型
│   │   └── tests/                  # 每规则一个 *_safety.rs
│   ├── guard/                      # 门禁
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs              # 裁决 + 保护路径常量 + 重解析点检测
│   ├── executor/                   # 执行
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs              # 回收站/隔离/删除 + undo + 释放量统计
│   ├── report/                     # 报告
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs              # JSON/pretty + 健康评分
│   ├── dedup/                      # 阶段 2 空壳（重复文件检测）
│   ├── inventory/                  # 阶段 2 空壳（软件清单）
│   └── cli/                        # CLI 入口
│       ├── Cargo.toml
│       └── src/
│           └── main.rs             # 二进制名 scanary
│
├── rules/                          # 外置规则
│   ├── _templates/
│   │   └── rule.toml
│   ├── browser-cache.toml
│   ├── system-temp.toml
│   └── ...
│
├── tests/                          # 集成测试 + 规则 safety test
├── references/                     # SKILL 参考资料
│   ├── safety-rules.md
│   ├── workflow.md
│   └── rule-format.md
├── docs/                           # 设计文档
│   └── DESIGN.md                   # 本规格书
└── gui/                            # 薄 GUI（先浏览器方案）
    ├── index.html
    ├── app.js
    └── styles.css
```

## 1.7 技术选型

| 项 | 选择 | 理由 |
|---|---|---|
| 语言 | Rust (edition 2021) | 性能 + 本机就绪 |
| 扫描 | ntfs crate（MFT）+ jwalk（回退） | pinkbin 验证 |
| 规则解析 | toml + serde | 数据驱动 |
| glob 匹配 | globset | pinkbin 验证 |
| 回收站 | trash crate | 跨平台回收站 |
| 序列化 | serde_json | 输出 |
| CLI 解析 | clap | 成熟 |
| 异步 | tokio（仅 CLI 需要时） | 精简依赖 |
| GUI（薄） | 纯 HTML/JS + 本地 HTTP | 免构建链 |

```
（下接第 2 部分）
```


---

# 第 2 部分：Crate 模块设计

## 2.1 wcs-scanner — 扫描引擎

**职责**：遍历文件系统，产出带 size/file_count 的目录树。双路径：MFT 直读（快，需提权）+ jwalk 遍历（慢，无需提权）。

### 公共类型

```rust
use serde::{Deserialize, Serialize};

/// 目录树节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,          // 逻辑字节
    pub file_count: u64,
    pub children: Vec<Node>,
    #[serde(default)]
    pub rule_id: Option<String>,       // 匹配到的规则 id
    #[serde(default)]
    pub top_extensions: Vec<ExtShare>,
}

/// 扩展名占比
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtShare {
    pub ext: String,
    pub bytes: u64,
    pub count: u64,
}

/// 扫描选项
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub follow_symlinks: bool,
    pub max_depth: Option<usize>,
    pub keep_files_per_dir: Option<usize>,   // 默认 500
    pub prefer_mft: bool,                     // 是否尝试 MFT
    pub mft_max_records: Option<u64>,         // 实际实现新增：None=全量，Some(n)=限制记录数
}

/// 扫描模式（用于诊断）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    Mft,      // NTFS MFT 直读
    Walk,     // jwalk 遍历
}

/// 扫描统计（阶段耗时诊断）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanStats {
    pub mode: ScanMode,
    pub mft_attempted: bool,
    pub mft_succeeded: bool,
    pub mft_ms: u64,
    pub walk_ms: u64,
    pub build_tree_ms: u64,
    pub total_ms: u64,
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub degraded: bool,      // 是否因权限/非NTFS降级
    pub degrade_reason: Option<String>,
}

/// 扫描进度
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub files_seen: u64,
    pub bytes_seen: u64,
    pub current_path: String,
}
```

### 公共 API

```rust
/// 简单扫描（默认选项）
pub fn scan<P: AsRef<Path>>(root: P) -> anyhow::Result<Node>;

/// 带选项 + 进度回调
pub fn scan_with<P, F>(
    root: P,
    opts: ScanOptions,
    on_progress: F,
) -> anyhow::Result<Node>
where
    P: AsRef<Path>,
    F: Fn(&ScanProgress) + Send + Sync;

/// 带统计（返回树 + 阶段耗时）
pub fn scan_with_stats<P, F>(
    root: P,
    opts: ScanOptions,
    on_progress: F,
) -> anyhow::Result<(Node, ScanStats)>
where
    P: AsRef<Path>,
    F: Fn(&ScanProgress) + Send + Sync;

/// 权限探测：当前是否能 MFT 直读
pub fn can_use_mft(volume: char) -> bool;

/// 当前是否管理员（实际实现新增）
pub fn is_admin() -> bool;

/// 查询某卷空间（实际实现新增）
pub fn disk_space(volume: char) -> Option<DiskSpace>;

/// 提取目录元数据（供 Agent 判断，不读文件内容）
pub fn inspect<P: AsRef<Path>>(path: P, samples: usize) -> anyhow::Result<DirMetadata>;
```

### 内部模块

| 模块 | 职责 |
|---|---|
| `node.rs` | Node / ExtShare / ScanOptions / ScanStats / DiskSpace 类型定义 |
| `mft.rs` | Windows NTFS MFT 直读（cfg(windows)），失败返回 Err 触发降级 |
| `aligned_reader.rs` | 扇区对齐读取适配器（卷设备读取需 512 整数倍） |
| `perm.rs` | 权限探测：is_admin / can_open_volume / can_use_mft / disk_space |
| `walk.rs` | 递归遍历，收集 (path, size)，剪枝 $recycle.bin 等 |
| `lib.rs` | 编排：尝试 MFT → 失败降级 → 构建树 |

### 关键设计

**权限降级逻辑**（对应 P2+ 策略）：

```rust
pub fn scan_with_stats(...) -> Result<(Node, ScanStats)> {
    let mut stats = ScanStats::default();
    #[cfg(windows)]
    // 实际实现：MFT 仅当 root 是卷根时才尝试（子目录不支持）
    if opts.prefer_mft && is_drive_root(root) && can_use_mft(drive_letter) {
        stats.mft_attempted = true;
        // 临时静默 panic hook，避免被 catch 的 panic 仍打印噪音
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = catch_unwind(AssertUnwindSafe(|| mft::scan_volume(...)));
        std::panic::set_hook(prev);
        match result {
            Ok(Ok(node)) => {
                stats.mode = ScanMode::Mft;
                stats.mft_succeeded = true;
                return Ok((node, stats));
            }
            _ => {
                stats.degraded = true;
                stats.degrade_reason = Some("MFT 失败（权限或非 NTFS）".into());
            }
        }
    } else {
        stats.degraded = true;
        stats.degrade_reason = Some("无 MFT 权限（需管理员），使用 walk".into());
    }
    stats.mode = ScanMode::Walk;
    walk::scan_with_stats(...)
}
```

**系统目录剪枝**（借鉴 pinkbin，防止回收站伪 root 被误识别）：

```rust
const PRUNED_DIRS: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    ".trash",
];
```


---

## 2.2 wcs-scaffold — 规则引擎

**职责**：加载 TOML 规则，编译为可快速匹配的形式，对路径做匹配。

### 公共类型

```rust
/// 一条清理规则
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
    pub risk: Risk,
    pub disclaimer: String,
    pub detect: Vec<String>,           // 检测 glob
    #[serde(rename = "match", default)]
    pub matcher: Match,
    #[serde(
        rename(deserialize = "scope", serialize = "scopes"),
        alias = "scopes",
        default
    )]
    pub scopes: Vec<Scope>,
}

/// 匹配辅助
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Match {
    #[serde(default)]
    pub name_contains: Vec<String>,
    #[serde(default)]
    pub must_have_child: Vec<String>,
}

/// 一个清理范围
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scope {
    pub id: String,
    pub label: String,
    pub glob: String,
    pub mode: Mode,
    #[serde(default)]
    pub prompt: Option<Prompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,      // cache | media | backup
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,       // 版本标签
    #[serde(default)]
    pub recycle_granularity: RecycleGranularity,
}

/// 执行模式
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Recycle,      // 回收站
    Quarantine,   // 隔离区
    Delete,       // 永久删除
    SystemCmd,    // 调系统命令（如 DISM）
}

/// 回收粒度
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecycleGranularity {
    #[default]
    File,         // 逐文件
    Directory,    // 整目录
}

/// 风险等级
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,      // GREEN
    Medium,   // YELLOW
    High,     // RED
}

/// 交互提示
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Prompt {
    None,
    Days { default: u32, #[serde(default)] label: Option<String> },
    Bytes { default: u64, #[serde(default)] label: Option<String> },
    Choice { default: String, options: Vec<String>, #[serde(default)] label: Option<String> },
    Confirm { #[serde(default)] label: Option<String> },
}

/// 预编译规则（热路径匹配用）
pub struct CompiledRule {
    pub id: String,
    detect_globs: globset::GlobSet,
    name_fragments_lc: Vec<String>,
    must_have_child: Vec<String>,
}
```

### 公共 API

```rust
/// 解析单条 TOML 规则
pub fn parse_toml(s: &str) -> anyhow::Result<Rule>;

/// 加载目录下所有规则
pub fn load_dir(dir: &Path) -> anyhow::Result<Vec<Rule>>;

/// 加载规则（内置 + 外置合并，外置覆盖）
pub fn load_all(builtin_dir: Option<&Path>, external_dir: Option<&Path>) -> anyhow::Result<Vec<Rule>>;

/// 单次匹配（每次重建 globset，慢）
pub fn detect_for(rules: &[Rule], path: &Path) -> Option<String>;

/// 预编译（热路径）
pub fn compile_all(rules: &[Rule]) -> Vec<CompiledRule>;

/// 用预编译匹配
pub fn detect_compiled(compiled: &[CompiledRule], path: &Path) -> Option<String>;

/// 展开环境变量（$VAR / ${VAR} / %VAR%）
pub fn expand_env(s: &str) -> String;

/// 匹配某 scope 的文件/目录（供 preview/execute）
pub fn match_scope(scope: &Scope, root: &Path) -> anyhow::Result<Vec<PathBuf>>;
```

### 内部模块

| 模块 | 职责 |
|---|---|
| `model.rs` | Rule/Scope/Mode/Risk/Prompt 类型 |
| `loader.rs` | TOML 解析 + 目录加载 + 内置外置合并 |
| `matcher.rs` | glob 编译 + detect_compiled + match_scope |
| `lib.rs` | 对外 API 聚合 |

### 关键设计

**TOML/JSON 字段名映射**（借鉴 pinkbin）：
- TOML 作者写 `[[scope]]`（单数）
- JSON 输出给消费者用 `scopes`（复数）
- 用 `#[serde(rename(deserialize = "scope", serialize = "scopes"))]` 双向映射

**单条坏 glob 不毒化整条规则**：

```rust
for d in &rule.detect {
    match compile_glob(d) {
        Ok(g) => builder.add(g),
        Err(e) => tracing::warn!("跳过坏 pattern {:?}: {}", d, e),
    }
}
```

---

## 2.3 wcs-guard — 安全门禁

**职责**：fail-closed 的路径校验。任何破坏性操作前，必须过 guard。**这是本项目的安全核心，独立成 crate 以强化。**

### 公共类型

```rust
/// 门禁裁决
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,     // 允许
    Warn,     // 允许但警告（用户数据区）
    Block,    // 拒绝（系统保护/越界）
    Skip,     // 跳过（不存在/无法判定）
}

/// 门禁结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardResult {
    pub verdict: Verdict,
    pub reason: Option<String>,
    pub matched_rule: Option<String>,   // 命中的保护规则名
}

/// 门禁配置
#[derive(Debug, Clone)]
pub struct GuardConfig {
    pub allowed_roots: Vec<PathBuf>,    // 允许操作的根（如 C:\）
    pub protect_c_drive_boundary: bool, // 是否限制在 C 盘
    pub reject_reparse_points: bool,    // 拒绝祖先含 junction/symlink
    pub extra_protected: Vec<String>,   // 额外保护路径片段
}
```

### 公共 API

```rust
/// 校验单个路径（核心）
pub fn check_path(path: &Path, cfg: &GuardConfig) -> GuardResult;

/// 批量校验
pub fn check_paths(paths: &[PathBuf], cfg: &GuardConfig) -> Vec<GuardResult>;

/// 校验是否为"危险根"（盘符根/系统根/用户根）
pub fn is_forbidden_root(path: &Path) -> bool;

/// 校验路径祖先是否含重解析点
pub fn has_reparse_ancestor(path: &Path) -> bool;
```

### 受保护路径（内置红线）

```rust
/// 永远拒绝的路径片段
const FORBIDDEN_PATTERNS: &[&str] = &[
    // 系统核心
    r"\\Windows\\System32",
    r"\\Windows\\SysWOW64",
    r"\\Program Files",
    r"\\Program Files (x86)",
    r"\\$Recycle.Bin",
    r"\\System Volume Information",
    // 用户数据
    r"\\Documents",
    r"\\Pictures",
    r"\\Videos",
    r"\\Desktop",
    r"\\Music",
    // 开发工具（借鉴 windows-disk-cleanup）
    r"\.vscode",
    r"\.vscode-server",
    r"\\JetBrains",
    r"\\Code\\User",
    // 版本控制
    r"\.git\\",
];
```

### 关键设计

**fail-closed 原则**：任何不确定的情况，一律 Block。宁可误拦，不可误删。

```rust
pub fn check_path(path: &Path, cfg: &GuardConfig) -> GuardResult {
    if path.is_relative() { return block("相对路径不被允许"); }
    if is_forbidden_root(path) { return block("命中受保护根"); }
    if matches_forbidden_pattern(path) { return block("命中保护路径"); }
    if !within_allowed_roots(path, cfg) { return block("超出允许操作范围"); }
    if cfg.reject_reparse_points && has_reparse_ancestor(path) { return block("祖先含 junction/符号链接"); }
    if in_user_data_area(path) { return warn("位于用户数据区，需确认"); }
    pass()
}
```


---

## 2.4 wcs-executor — 执行引擎

**职责**：执行清理动作（回收站/隔离/删除/系统命令），写 undo 日志。**所有动作前必须过 guard。**

### 公共类型

```rust
/// 执行动作
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Recycle,      // 回收站
    Quarantine,   // 隔离
    Delete,       // 永久删除
    SystemCmd,    // 系统命令
}

/// 执行计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub action: Action,
    pub paths: Vec<PathBuf>,
    pub reason: String,
    pub granularity: RecycleGranularity,
    #[serde(default)]
    pub system_cmd: Option<String>,     // SystemCmd 时的命令
}

/// undo 记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoEntry {
    pub timestamp: String,
    pub session_id: String,             // 一次执行的会话 id
    pub action: Action,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,   // 隔离时的目标
    pub reason: String,
    pub bytes_freed: u64,
}

/// 执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub session_id: String,
    pub executed: bool,
    pub matched_count: usize,
    pub total_bytes: u64,
    pub entries: Vec<UndoEntry>,
    pub blocked: Vec<(PathBuf, String)>,   // 被 guard 拦下的
}
```

### 公共 API

```rust
/// 执行计划（dry_run 时只记录不删除）
pub fn execute(
    plan: &Plan,
    guard_cfg: &GuardConfig,
    dry_run: bool,
    undo_log: &Path,
    quarantine_root: &Path,
) -> anyhow::Result<ExecResult>;

/// 读取 undo 日志
pub fn read_undo_log(path: &Path) -> anyhow::Result<Vec<UndoEntry>>;

/// 从 undo 恢复（隔离区可恢复）
pub fn restore(session_id: &str, undo_log: &Path) -> anyhow::Result<usize>;
```

### 内部模块

| 模块 | 职责 |
|---|---|
| `action.rs` | 三种删除动作实现 |
| `undo.rs` | undo.jsonl 读写 + 会话管理 |
| `lib.rs` | execute 编排（先过 guard） |

### 关键设计

**执行前强制过 guard**：

```rust
pub fn execute(plan, guard_cfg, dry_run, undo_log, quarantine_root) -> Result<ExecResult> {
    let mut allowed = Vec::new();
    let mut blocked = Vec::new();
    for p in &plan.paths {
        match guard::check_path(p, guard_cfg) {
            GuardResult { verdict: Verdict::Block, reason, .. } => {
                blocked.push((p.clone(), reason.unwrap_or_default()));
            }
            _ => allowed.push(p.clone()),
        }
    }
    if dry_run { return Ok(make_dry_result(allowed, blocked)); }
    // 执行 allowed...
}
```

**目录粒度 vs 文件粒度**（M3）：

```rust
match plan.granularity {
    RecycleGranularity::File => {
        for p in paths { trash::delete(p)?; }  // 每文件一条回收站记录
    }
    RecycleGranularity::Directory => {
        for p in paths { trash::delete(p)?; }  // 每目录一条
    }
}
```

---

## 2.5 wcs-report — 报告引擎

**职责**：把扫描/执行结果转为 JSON（主）或 pretty（可选），计算健康评分。

### 公共类型

```rust
/// 报告视图
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub generated_at: String,
    pub mode: String,
    pub health: HealthScore,
    pub summary: Summary,
    pub top_dirs: Vec<ChildInfo>,      // 实际实现：替代初稿的 sections
    pub scan_stats: Option<ScanStats>,
}

/// 健康评分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthScore {
    pub score: u8,              // 0-100
    pub grade: String,          // A/B/C/D/F
    pub used_percent: f32,
    pub free_gb: f64,
    pub total_gb: f64,
    pub advice: String,
}

/// 摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub safely_cleanable_gb: f64,     // GREEN（low）可清理
    pub needs_confirm_gb: f64,        // YELLOW（medium）需确认
    pub high_risk_gb: f64,            // 实际实现新增：RED（high）
    pub top_dirs: Vec<ChildInfo>,
}

/// 报告分节
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub risk: Risk,
    pub items: Vec<ReportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportItem {
    pub label: String,
    pub path: String,
    pub size: u64,
    pub rule_id: Option<String>,
}
```

### 公共 API

```rust
/// 构建报告（无磁盘数据，health 用占位）
pub fn build(tree: &Node, scan_stats: Option<ScanStats>) -> Report;

/// 构建报告（实际实现主入口：接真实磁盘空间 + 规则风险分级）
pub fn build_with_disk(
    tree: &Node,
    scan_stats: Option<ScanStats>,
    disk: Option<DiskSpace>,
    rules: &[Rule],
) -> Report;

/// 计算健康评分
pub fn health_score(used_percent: f32, free_gb: f64, total_gb: f64) -> HealthScore;

/// 输出 JSON
pub fn to_json(report: &Report) -> anyhow::Result<String>;

/// 输出人类可读
pub fn to_pretty(report: &Report) -> String;
```

### 健康评分算法（借鉴 c-drive-cleaner）

```rust
pub fn health_score(used_percent: f32, free_gb: f64, total_gb: f64) -> HealthScore {
    // 使用率 50% 时 100 分，每高 1% 扣 2 分
    let raw = 100.0 - (used_percent - 50.0) * 2.0;
    let score = raw.clamp(0.0, 100.0) as u8;
    let grade = match score {
        90..=100 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    }.to_string();
    let advice = if score >= 80 { "空间健康".into() }
        else if score >= 60 { "需关注".into() }
        else { "空间紧张，建议清理".into() };
    HealthScore { score, grade, used_percent, free_gb, total_gb, advice }
}
```

---

## 2.6 wcs-cli — 命令行入口

**职责**：解析参数、路由子命令、调用各 crate、格式化输出。这是 Agent 和 GUI 共同调用的入口。

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "scanary", version, about = "Win-C-Scanary 磁盘扫描清理工具")]
struct Cli {
    #[arg(long, env = "WCS_RULES_DIR")]
    rules_dir: Option<PathBuf>,
    #[arg(long, env = "WCS_UNDO_LOG")]
    undo_log: Option<PathBuf>,
    #[arg(long, env = "WCS_QUARANTINE_ROOT")]
    quarantine_root: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "json")]
    format: OutputFormat,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 扫描磁盘/目录
    Scan { path: String, #[arg(long, default_value = "50")] top: usize },
    /// 提取目录元数据
    Inspect { path: String, #[arg(long, default_value = "20")] samples: usize },
    /// 列出规则
    Rules,
    /// 预览规则匹配（dry-run）
    Preview { rule_id: String, path: String, #[arg(long)] scope: Option<String> },
    /// 执行清理
    Execute { rule_id: String, path: String, #[arg(long)] scope: Option<String>, #[arg(long, default_value = "true")] dry_run: bool },
    /// 生成报告
    Report { path: String },
    /// 权限状态
    Permissions,
}

#[derive(clap::ValueEnum, Clone)]
enum OutputFormat { Json, Pretty }
```

### 关键设计

- **所有子命令默认 JSON 输出**（O3），`--format pretty` 切换到人类可读
- **execute 默认 dry_run=true**，需显式 `--dry-run false` 才真删
- **环境变量**：`WCS_RULES_DIR` / `WCS_UNDO_LOG` / `WCS_QUARANTINE_ROOT`


---

# 第 3 部分：CLI 接口设计

## 3.1 命令总览

二进制名：**`scanary`**

| 子命令 | 用途 | 权限 | 默认行为 |
|---|---|:---:|---|
| `scan` | 扫描磁盘/目录 | 普通（MFT 需提权） | 输出目录树 JSON |
| `inspect` | 提取目录元数据 | 普通 | 输出元数据 JSON |
| `rules` | 列出所有规则 | 普通 | 输出规则列表 |
| `preview` | 预览规则匹配 | 普通 | 只读，不删 |
| `execute` | 执行清理 | 视目标 | **默认 dry-run** |
| `report` | 生成报告 | 普通 | JSON + 健康评分 |
| `permissions` | 查看权限状态 | 普通 | 显示能否 MFT |

## 3.2 全局参数

| 参数 | 环境变量 | 默认 | 说明 |
|---|---|---|---|
| `--rules-dir <DIR>` | `WCS_RULES_DIR` | `<exe_dir>/rules` | 规则目录 |
| `--undo-log <PATH>` | `WCS_UNDO_LOG` | `%APPDATA%\Win-C-Scanary\undo.jsonl` | undo 日志 |
| `--quarantine-root <PATH>` | `WCS_QUARANTINE_ROOT` | `%APPDATA%\Win-C-Scanary\quarantine` | 隔离区 |
| `--format <json|pretty>` | — | `json` | 输出格式 |
| `--verbose` | — | 关 | 详细日志 |

## 3.3 各子命令详解

### scan — 扫描

```
scanary scan <PATH> [--top N] [--prefer-mft] [--max-depth N]
```

| 参数 | 默认 | 说明 |
|---|---|---|
| `<PATH>` | 必需 | 扫描根路径 |
| `--top N` | 50 | 返回的 top 大目录数 |
| `--prefer-mft` | 关 | 优先用 MFT（需权限，失败降级） |
| `--max-depth N` | 无限制 | 最大深度 |

**输出（JSON）**：

```json
{
  "root": { "name": "C:", "path": "C:\\", "size": 177000000000, "file_count": 1200000, "children": [...] },
  "top_dirs": [ { "name": "Windows", "size": 32000000000, "is_dir": true }, ... ],
  "scan_stats": { "mode": "mft", "mft_succeeded": true, "total_ms": 3200, "degraded": false }
}
```

### inspect — 目录深挖

```
scanary inspect <PATH> [--samples N]
```

**输出**：

```json
{
  "path": "C:\\Users\\me\\AppData\\Local\\npm-cache",
  "size_bytes": 3200000000,
  "file_count": 15000,
  "top_extensions": [ { "ext": "tgz", "bytes": 2000000000, "count": 3000 }, ... ],
  "sample_paths": [ "..." ],
  "top_children": [ ... ],
  "rule_hint": "npm-cache"
}
```

> **注意**：inspect 只返回元数据（路径名/大小/文件数/扩展名分布/样本路径），**绝不读文件内容**。这是隐私底线。

### rules — 列出规则

```
scanary rules
```

**输出**：规则数组（id/name/risk/scopes）。

### preview — 预览（强制先行）

```
scanary preview <RULE_ID> <PATH> [--scope SCOPE_ID]
```

**输出**：

```json
{
  "rule_id": "browser-cache",
  "root_path": "C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\User Data",
  "matched": [ { "path": "...", "size": 123456, "is_dir": false } ],
  "total_bytes": 5000000000,
  "total_count": 12000,
  "guard_blocked": [ { "path": "...", "reason": "命中保护路径" } ],
  "guard_warnings": [ { "path": "...", "reason": "位于用户数据区" } ]
}
```

**关键**：preview 是只读的。Agent 必须先把 preview 结果展示给用户，确认后才 execute。

### execute — 执行

```
scanary execute <RULE_ID> <PATH> [--scope SCOPE_ID] [--dry-run true|false] [--yes]
```

| 参数 | 默认 | 说明 |
|---|---|---|
| `--dry-run` | **true** | 默认只预览，false 才真删 |
| `--yes` | 关 | 跳过交互确认（Agent 用） |

**输出**：

```json
{
  "session_id": "20260920-143022-a1b2",
  "executed": true,
  "matched_count": 12000,
  "total_bytes": 5000000000,
  "entries": [ ... ],
  "blocked": [ ... ]
}
```

### report — 报告

```
scanary report <PATH>
```

**输出**：含健康评分 + 分级建议的完整报告（见 2.5）。

### permissions — 权限状态

```
scanary permissions
```

**输出**：

```json
{
  "is_admin": false,
  "can_use_mft": false,
  "can_clean_system": false,
  "advice": "当前普通权限。全盘秒级扫描和系统清理需管理员权限。"
}
```

## 3.4 典型调用序列（Agent 视角）

```bash
# 1. 看权限
scanary permissions --format json

# 2. 扫描
scanary scan C:\ --top 50 --format json

# 3. 深挖可疑目录
scanary inspect "C:\Users\me\AppData\Local\npm-cache" --format json

# 4. 列规则
scanary rules --format json

# 5. 预览
scanary preview npm-cache "C:\Users\me\AppData\Local\npm-cache" --format json

# 6. 展示给用户确认后执行
scanary execute npm-cache "C:\Users\me\AppData\Local\npm-cache" --dry-run false --yes
```

## 3.5 退出码约定

| 码 | 含义 |
|:---:|---|
| 0 | 成功 |
| 1 | 一般错误 |
| 2 | 参数错误 |
| 3 | 权限不足 |
| 4 | 被 guard 拦截（部分/全部） |
| 5 | 路径不存在 |

## 3.6 输出稳定性承诺

- JSON schema 遵循语义化版本，破坏性变更升大版本
- pretty 格式可自由调整（消费者不应依赖其结构）
- 所有时间戳用 RFC3339


---

# 第 4 部分：规则格式规范

## 4.1 规则文件位置

```
<exe_dir>/rules/*.toml         # 外置规则（用户/社区可加）
内置（编译进二进制）             # 默认规则，最低优先级
```

加载优先级：外置规则 > 内置规则（同名 id 外置覆盖内置）。

## 4.2 规则文件结构（TOML）

```toml
# 规则唯一 id（小写连字符）
id          = "browser-cache"
# 显示名
name        = "浏览器缓存"
# 官网（可选）
homepage    = "https://..."
# 风险等级：low(GREEN) | medium(YELLOW) | high(RED)
risk        = "low"
# 免责声明（必填，展示给用户）
disclaimer  = "只清浏览器缓存，不碰书签/密码/历史/扩展。登录状态不受影响。"

# 检测 glob（多个，任一命中即认为此规则适用）
detect = [
  "%LOCALAPPDATA%/Google/Chrome/User Data",
  "%LOCALAPPDATA%/Microsoft/Edge/User Data",
]

# 匹配辅助（可选，用于检测不到但目录名相似的场景）
[match]
name_contains   = ["Chrome", "Edge"]
must_have_child = ["Default"]

# 清理范围（可多个）
[[scope]]
id       = "chrome-cache"
label    = "Chrome 缓存"
glob     = "%LOCALAPPDATA%/Google/Chrome/User Data/*/Cache/**"
mode     = "recycle"          # recycle | quarantine | delete | system_cmd
category = "cache"            # cache | media | backup
recycle_granularity = "file"  # file | directory

[[scope]]
id       = "chrome-code-cache"
label    = "Chrome Code Cache"
glob     = "%LOCALAPPDATA%/Google/Chrome/User Data/*/Code Cache/**"
mode     = "recycle"
category = "cache"
prompt   = { kind = "none" }   # none | days | bytes | choice | confirm
```

## 4.3 字段说明

### 顶层字段

| 字段 | 必填 | 类型 | 说明 |
|---|:---:|---|---|
| `id` | ✅ | string | 唯一标识，小写连字符 |
| `name` | ✅ | string | 显示名 |
| `homepage` | | string | 官网链接 |
| `risk` | ✅ | enum | `low`/`medium`/`high` |
| `disclaimer` | ✅ | string | 免责声明 |
| `detect` | ✅ | string[] | 检测 glob 列表 |
| `match` | | table | 匹配辅助 |
| `scope` | ✅ | table[] | 清理范围列表 |

### match 表

| 字段 | 类型 | 说明 |
|---|---|---|
| `name_contains` | string[] | 目录名包含这些片段之一 |
| `must_have_child` | string[] | 必须包含这些子目录 |

### scope 表

| 字段 | 必填 | 类型 | 说明 |
|---|:---:|---|---|
| `id` | ✅ | string | scope 标识 |
| `label` | ✅ | string | 显示名 |
| `glob` | ✅ | string | 匹配 glob |
| `mode` | ✅ | enum | 执行模式 |
| `prompt` | | table | 交互提示 |
| `category` | | string | cache/media/backup |
| `variant` | | string | 版本标签（如 3.x/4.x） |
| `recycle_granularity` | | enum | file（默认）/directory |

## 4.4 环境变量展开

规则里的路径支持三种写法：

| 写法 | 示例 | 展开为 |
|---|---|---|
| `%VAR%` | `%LOCALAPPDATA%` | `C:\Users\me\AppData\Local` |
| `$VAR` | `$HOME` | 用户主目录 |
| `${VAR}` | `${HOME}` | 同上 |

## 4.5 危险 glob 红线（强制校验）

**任何 scope 的 glob 不允许命中以下路径片段**（借鉴 pinkbin + windows-disk-cleanup）：

```
*.db / *.db-wal / *.db-shm         # 数据库
**/db_storage/**                    # 聊天 DB
**/Msg/** / **/MultiMsg/**          # 聊天数据
**/Accounts/** / **/login/**        # 账号状态
**/Favorite*/** / **/Fav/**        # 用户收藏
**/key/** / **/crypto/**            # 加密物料
**/.git/**                          # 版本控制
**/.vscode/** / **/JetBrains/**     # IDE 数据
```

**规则加载时自动校验**：若 glob 命中红线，规则加载失败并报错。

## 4.6 Safety Test 规范（强制）

**每条规则必须配一个 safety test，否则 CI 不通过。**

### 测试文件位置

```
tests/rules/<rule_id>_safety.rs
```

### 测试模板

```rust
//! Safety test for rule `<rule_id>`
use wcs_scaffold::{parse_toml, match_scope};

const RULE_TOML: &str = include_str!("../../rules/<rule_id>.toml");

#[test]
fn rule_parses() {
    let rule = parse_toml(RULE_TOML).expect("规则应能解析");
    assert!(!rule.scopes.is_empty(), "规则应至少有一个 scope");
}

#[test]
fn positive_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    // 每个 scope 至少有一条应命中的路径
    // 例：构造一个假路径，验证 glob 逻辑
    for scope in &rule.scopes {
        assert!(scope.glob_compiles(), "scope {} 的 glob 应可编译", scope.id);
    }
}

#[test]
fn red_line_assertions() {
    let rule = parse_toml(RULE_TOML).unwrap();
    // 一组红线路径，必须 zero match
    let red_lines = [
        "C:/Users/me/Documents/important.db",
        "C:/Users/me/.vscode/extensions",
        "C:/Users/me/Documents/project/.git/config",
    ];
    for path in red_lines {
        assert!(
            !rule_matches_any_scope(&rule, path),
            "红线路径 {:?} 不应被任何 scope 命中",
            path
        );
    }
}
```

### 断言要求

| 断言类型 | 要求 |
|---|---|
| **正向断言** | 每个 scope 至少验证一条应命中的路径 |
| **红线断言** | 一组红线路径必须 zero match |

## 4.7 规则示例（完整）

```toml
id          = "system-temp"
name        = "系统临时文件"
risk        = "low"
disclaimer  = "清理用户 TEMP 和 Windows Temp 目录。正在占用的文件会自动跳过，不影响运行中的程序。"

detect = [
  "%TEMP%",
  "%WINDIR%/Temp",
]

[[scope]]
id       = "user-temp"
label    = "用户临时文件夹"
glob     = "%TEMP%/**"
mode     = "recycle"
category = "cache"
recycle_granularity = "file"

[[scope]]
id       = "windows-temp"
label    = "Windows 临时文件夹"
glob     = "%WINDIR%/Temp/**"
mode     = "recycle"
category = "cache"
recycle_granularity = "file"
prompt   = { kind = "confirm", label = "清理 Windows Temp 需要管理员权限，确认继续？" }
```


---

# 第 5 部分：安全模型与权限策略

## 5.1 三层防护体系

```
┌─────────────────────────────────────────────────┐
│ 第 1 层：规则层（scaffold）                        │
│   - 每条 scope 的 glob 不允许命中红线片段           │
│   - 加载时自动校验，命中即拒绝加载                   │
│   - 每条规则配 safety test（正向 + 红线断言）        │
├─────────────────────────────────────────────────┤
│ 第 2 层：引擎层（guard）                           │
│   - fail-closed 路径校验                          │
│   - 拒绝：相对路径/受保护根/保护片段/越界/重解析点    │
│   - 任何不确定情况一律 Block                       │
├─────────────────────────────────────────────────┤
│ 第 3 层：交互层（CLI/GUI）                         │
│   - 默认 dry-run（预览先行）                       │
│   - 默认回收站（可恢复）                           │
│   - 破坏性操作需显式确认                           │
└─────────────────────────────────────────────────┘
```

## 5.2 安全红线（永不自动执行）

| 类别 | 红线 | 处理 |
|---|---|---|
| 系统核心 | `C:\Windows\System32`、`SysWOW64` | guard Block |
| 程序目录 | `Program Files`、`Program Files (x86)` | guard Block |
| 系统隐藏 | `$Recycle.Bin`、`System Volume Information` | guard Block |
| 用户数据 | `Documents`、`Pictures`、`Videos`、`Desktop`、`Music` | guard Warn（需确认） |
| IDE 数据 | `.vscode`、`.vscode-server`、`JetBrains`、`Code\User` | guard Block |
| 版本控制 | 任何 `.git` 目录 | guard Block |
| 数据库 | `*.db`、`*.db-wal`、`*.db-shm` | 规则红线 |
| 加密物料 | `key`、`crypto` 目录 | 规则红线 |

## 5.3 门禁判定逻辑（fail-closed）

```
check_path(path):
  1. 相对路径？               → Block
  2. 命中原生根？             → Block
  3. 命中保护片段？           → Block
  4. 超出允许根？             → Block
  5. 祖先含重解析点？         → Block
  6. 位于用户数据区？         → Warn
  7. 其它                     → Pass
```

## 5.4 删除方式与可恢复性

| 方式 | 可恢复 | 默认 | 场景 |
|---|:---:|:---:|---|
| **回收站** | ✅ | ✅ 默认 | 日常清理 |
| **隔离区** | ✅（保留 N 天） | | 需观察期 |
| **永久删除** | ❌ | | 用户明确要求 |
| **系统命令** | 视命令 | | WinSxS/DISM 等 |

所有操作写入 `undo.jsonl`，含：时间戳、会话 id、动作、源路径、目标（隔离时）、原因、释放字节。

## 5.5 权限策略（P2+）

### 5.5.1 权限分级

```
普通权限（默认）
  ├── 扫描已知缓存目录
  ├── 清理用户级缓存（%TEMP%、浏览器、npm/pip）
  ├── 重复文件检测（只读）
  ├── 目录深挖
  └── 报告生成

管理员权限（按需提权）
  ├── MFT 全盘秒级扫描
  ├── 系统级清理（Windows\Temp、SoftwareDistribution）
  ├── WinSxS（DISM）
  ├── Windows.old 清理
  ├── 休眠文件管理（powercfg）
  ├── 页面文件迁移
  └── 系统还原点删除（vssadmin）
```

### 5.5.2 权限探测

```rust
/// 检查当前是否管理员
pub fn is_admin() -> bool {
    // Windows: 检查 token 是否在 Administrators 组
}

/// 检查能否 MFT 直读某卷
pub fn can_use_mft(volume: char) -> bool {
    if !is_admin() { return false; }
    // 尝试打开卷设备测试
}
```

### 5.5.3 提权流程

```
用户/Agent 请求「全盘扫描」
  └─ can_use_mft() ?
        ├─ true  → 用 MFT，秒级
        └─ false → 提示「秒级扫描需要管理员权限」
                    ├─ 同意 → 重启自身提权（runas）
                    └─ 拒绝 → 降级 jwalk，慢但能用
```

### 5.5.4 优雅降级原则

**绝不因为缺权限而失败或误删。**

| 场景 | 行为 |
|---|---|
| MFT 无权限 | 降级 jwalk，标注 `degraded: true` |
| 非 NTFS 卷 | 降级 jwalk |
| 系统清理无权限 | 明确提示需提权，不尝试 |
| 文件被占用 | 跳过该文件，继续其他 |

## 5.6 隐私保护

| 项 | 策略 |
|---|---|
| 文件内容 | **永不读取**（只读元数据：路径名/大小/文件数/扩展名） |
| 数据外发 | **绝不上传**（引擎纯本地，无网络调用） |
| AI 交互 | 由 Agent 负责，引擎只提供元数据；GUI 不含 AI |
| 遥测 | 无 |

## 5.7 安全测试矩阵

| 测试 | 验证点 |
|---|---|
| guard 单元测试 | 各类危险路径都被 Block |
| guard 边界测试 | 相对路径/盘符根/用户根 |
| 重解析点测试 | junction/symlink 祖先被拦截 |
| 规则红线测试 | 危险 glob 加载失败 |
| 规则 safety test | 正向 + 红线断言 |
| dry-run 测试 | 预览不产生实际删除 |
| undo 测试 | 操作可追溯 |
| 降级测试 | 无权限时优雅降级 |


---

# 第 6 部分：GUI 设计、功能清单与路线图

## 6.1 GUI 设计（薄壳）

### 6.1.1 定位

GUI 是**引导者**，不是完整应用：

| GUI 做 | GUI 不做 |
|---|---|
| 扫描 + 展示 + 简单执行 | 复杂决策（交给 Agent） |
| 日常清理引导 | AI 顾问 |
| 结果可视化 | 迁移方案设计 |
| 分流提示（复杂 → Agent） | 卸载风险评估 |

### 6.1.2 实现方案（G3：先浏览器，预留升级）

```
阶段 1（MVP）：scanary gui → 起本地 HTTP 服务 → 自动开浏览器
阶段 2（可选升级）：换 WebView2 薄壳（独立窗口）
阶段 3（可选升级）：Tauri 完整壳

关键：前端只依赖后端 HTTP API，壳可替换。
```

### 6.1.3 前端页面

| 页面 | 内容 |
|---|---|
| 首页 | C 盘健康评分 + 大目录概览 |
| 可清理 | 三色分级列表（GREEN/YELLOW/RED） |
| 执行 | 勾选 + 预览 + 执行 + 结果 |
| 分流 | 复杂项 → 「建议使用 Agent 分析」入口 |

### 6.1.4 GUI 与 Agent 的分界

```
GREEN 项  → GUI 直接引导用户清理
YELLOW 项 → GUI 展示 + 说明影响，用户可清理
RED 项    → GUI 只展示，不提供清理按钮
复杂判断   → GUI 提示「请用 Agent 分析」（如增长归因、迁移）
```

### 6.1.5 后端 API（GUI 调 CLI 的桥梁）

```
GET  /api/health          → 健康评分
POST /api/scan            → 触发扫描
GET  /api/rules           → 规则列表
POST /api/preview         → 预览
POST /api/execute         → 执行（需确认）
GET  /api/permissions     → 权限状态
```

后端是薄封装，转调 `scanary` CLI。

## 6.2 功能清单（15 项，分三层）

### 第 1 层：核心闭环（必须）

| 功能 | 说明 |
|---|---|
| S1 缓存扫描 | 扫描已知缓存目录 |
| C1 安全缓存清理 | 临时文件/浏览器/包管理器缓存 |

### 第 2 层：价值增强

| 功能 | 说明 |
|---|---|
| S2 全盘扫描 | MFT 秒级（需提权） |
| S3 目录深挖 | inspect 元数据 |
| C2 应用缓存清理 | 微信/QQ/WPS 等 |
| C3 重复文件检测 | 三阶段哈希（只读） |
| A3 报告系统 | 健康评分 + 分级 |
| A4 软件清单 | 读注册表 |

### 第 3 层：高阶能力

| 功能 | 说明 |
|---|---|
| S4 空间核算 | 逻辑 vs NTFS 实际分配 |
| S5 增长追踪 | 历史快照对比 |
| C4 系统级清理 | WinSxS/Windows.old（提权） |
| C5 休眠/页面文件 | powercfg（提权） |
| C6 软件卸载 | 引导为主 —— `scanary uninstall`（2026-09-21） |
| A2 迁移指导 | 缓存/软件迁移 |

> 注：A1（AI 顾问）不实现——由 Agent 承担。

## 6.3 实施路线图

### 阶段 0：骨架搭建
- 建 Cargo workspace（6 crate）
- 定义各 crate 的公共类型
- 编译通过（空实现）
- **里程碑**：`cargo build` 成功，`scanary --help` 可用

### 阶段 1：核心闭环（第 1 层）
- wcs-scanner：jwalk 扫描（先不做 MFT）
- wcs-scaffold：TOML 加载 + 匹配
- wcs-guard：路径校验
- wcs-executor：回收站 + undo
- wcs-cli：scan/preview/execute
- **里程碑**：能扫描 → 预览 → 回收站清理 → 看释放量

### 阶段 2：性能与增强（第 2 层）— 🟡 基本完成，性能待收尾
- [x] wcs-scanner：MFT 直读 + 权限降级
- [x] wcs-report：健康评分 + pretty
- [x] 重复文件检测（`crates/dedup`）
- [x] 软件清单（`crates/inventory`）
- [x] **性能收尾：全盘 MFT < 5s**（release 纯 MFT ~4.8s / scan 命令 ~5.4s；debug ~12s）
- **里程碑**：✅ 秒级全盘扫描（已达），完整报告（已达）

> T-PERF-1 详见 `docs/HANDOFF-PARALLEL.md` 任务卡。核心命题已解决：用流总长作 EOF 权威判据 + record_size 对齐大块读。

### 阶段 3：薄 GUI — ✅ 已完成（2026-09-21）
- [x] 本地 HTTP 服务（tiny_http，随机端口）
- [x] 前端页面（HTML/JS，内嵌）
- [x] 分流引导
- **里程碑**：✅ 启动即可用，浏览器界面可用（`scanary gui`）

### 阶段 4：高阶能力（第 3 层）— ✅ 已完成（2026-09-21）
- [x] 空间核算（逻辑 vs NTFS 实际分配）—— `scanary audit`，2026-09-21
- [x] 增长追踪（历史快照对比）—— `scanary snapshot` / `scanary grow`，2026-09-21
- [x] 系统级清理（WinSxS/Windows.old）—— `scanary system`（**指导型**：只给 DISM/cleanmgr 命令不执行），2026-09-21
- [x] 休眠/页面文件（powercfg）—— `scanary hiber`（检测 + 可逆命令建议，不执行），2026-09-21
- [x] 迁移指导 —— `scanary migrate`（分析大缓存 + junction/环境变量命令，纯建议），2026-09-21
- **里程碑**：✅ 功能全覆盖（5/5）

### 阶段 5：Skill 组装 — ✅ 已完成（2026-09-21）
- [x] 写薄 SKILL.md（94 行，< 100）
- [x] 写 references（safety-rules / workflow / rule-format）
- [x] 配 examples
- [x] 同步新增命令（audit/snapshot/grow/migrate/hiber/system/uninstall）到 SKILL.md + references
- **里程碑**：✅ 可被 Agent 调用

## 6.4 已定项（原待定项，2026-09-21 拍板）

| 项 | 结论 |
|---|---|
| 配置目录 | ✅ `%APPDATA%\Win-C-Scanary\`（undo 日志 / 隔离区 / 快照统一于此；非 Windows 用配置目录兜底） |
| 报告 schema 版本 | ✅ `schema_version: "1"`（`Report` 首字段） |
| GUI 端口策略 | ✅ 随机端口（`127.0.0.1:0`，避免冲突，启动打印实际 URL） |
| 隔离区保留天数 | ✅ 默认 7 天；`scanary quarantine` 查看/清理（`prune_quarantine`） |
| 内置规则首批 | ✅ 6 条：system-temp / browser-cache / npm-cache / dev-caches / wechat-pc / conda |

## 6.5 成功标准

| 维度 | 目标 |
|---|---|
| 性能 | 全盘 MFT 扫描 < 5 秒（1M 文件）— **✅ 已达（release 纯 MFT ~4.8s；scan 命令 ~5.4s；debug ~12s）** |
| 安全 | 所有破坏性操作过 guard；零误删 |
| 测试 | 每规则配 safety test；核心模块单元测试 |
| 结构 | SKILL.md < 100 行；crate 职责清晰 |
| 可用 | Agent 能完成「扫描→分析→预览→清理」闭环 |

---

## 附录：决策记录（12 轮确认）

| # | 决策 | 结论 |
|---|---|---|
| 1 | 目标形态 | 应用+Skill 双形态，Skill 优先 |
| 2 | 引擎语言 | Rust |
| 3 | 权限策略 | P2+ 默认普通，重活提权 |
| 4 | 引擎关系 | F1 引擎即 CLI |
| 5 | GUI 形态 | G3 先浏览器，可升级 |
| 6 | GUI 与 AI | H1 不放 AI，引导去 Agent |
| 7 | 功能范围 | R1 全功能分三层 |
| 8 | 规则 | TOML + 内置外置 + 强制测试 |
| 9 | 执行模型 | M3 声明式粒度 + D4 默认回收站 |
| 10 | 输出 | O3 JSON + pretty + O5 健康评分 |
| 11 | 结构 | S2 多 crate（6 个） |
| 12 | 命名 | Win-C-Scanary / scanary / wcs- |

---

*规格书完*
