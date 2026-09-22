# 示例（Examples）

面向 Agent 与用户的典型调用示例。每个 `.md` 是一段可直接照做的命令序列。

| 文件 | 场景 |
|---|---|
| [cleanup-c-drive.md](cleanup-c-drive.md) | C 盘空间不足，安全清理一轮 |
| [investigate-big-dir.md](investigate-big-dir.md) | 发现某个大目录，判断它是什么、能否清 |

> CLI 名 `scanary`。全局参数 `--format pretty` 须放在子命令**之前**。
> 破坏性操作默认 dry-run，确认后才 `--dry-run=false`。
