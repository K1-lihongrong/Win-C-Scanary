# 分发包说明（win-c-scanary-skill-v0.1.0）

## 解压后的结构

```
win-c-scanary-skill-v0.1.0/
├── SKILL.md              ← Agent 入口
├── README.md
├── CHANGELOG.md
├── scanary.exe           ← 预编译 CLI（静态链接，免 VC++ 运行库，Windows x64）
├── rules/                ← 6 条清理规则（与 exe 同级 → 开箱即用）
├── references/           ← 安全红线 / 工作流 / 规则格式
├── examples/             ← 典型调用示例
└── docs/                 ← 关键设计/验证文档
```

## 开箱即用

**exe 与 rules/ 同级**，无需任何参数即可加载规则：

```cmd
cd win-c-scanary-skill-v0.1.0

:: 直接可用
scanary.exe --help
scanary.exe permissions
scanary.exe --format pretty scan "C:\" --top 30
scanary.exe --format pretty report "C:\"

:: 预览 / 执行（默认 dry-run）
scanary.exe preview system-temp "%TEMP%"
scanary.exe execute system-temp "%TEMP%" --dry-run=false
```

> `--format` 须放在子命令**之前**。

## 免依赖

`scanary.exe` 采用 **静态链接 CRT**（`+crt-static`），不依赖
`vcruntime140.dll` / `msvcp140.dll` / `ucrtbase.dll`。
**复制到任意 Windows 10/11 x64 机器即可运行**，无需装运行库。

## 用法 A：作为 Agent Skill

把整个目录（或 `SKILL.md` + `references/`）放到 Agent 的 skill 目录。
触发词见 `SKILL.md` description。Agent 调用同目录下的 `scanary.exe`。

## 用法 B：作为 CLI 工具

把本目录加进 PATH，或在其下运行 `scanary.exe`。规则目录默认查 exe 同级 `rules/`。

## 性能

- release 构建：全盘 MFT 扫描 ~4.8s（管理员）。
- 普通权限自动降级 walk（较慢，功能可用）。

## 权限

- 普通权限：基本功能可用。
- 管理员：MFT 秒级扫描 + 系统级操作。用 `scanary.exe permissions` 查看并获取提权命令。
