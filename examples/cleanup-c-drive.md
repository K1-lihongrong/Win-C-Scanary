# 示例：C 盘空间不足，安全清理一轮

> 目标：找到可安全清理的缓存，预览确认后回收站清理，最后看释放量。
> 全程默认预览先行、回收站兜底。

```bash
# 0. 权限状态（决定能否用 MFT 秒级全盘扫描）
scanary permissions

# 1. 全盘扫描，看空间大头（release 构建 < 5s；debug 约 12s）
scanary --format pretty scan "C:\" --top 30

# 2. 看健康评分 + 可安全清理量（GREEN/YELLOW/RED 分级）
scanary --format pretty report "C:\"

# 3. 列出可用规则
scanary --rules-dir rules rules

# 4. 预览某规则会命中什么（不删）
scanary --rules-dir rules preview system-temp "%TEMP%"

# 5. 把命中项和可释放空间展示给用户，获确认

# 6. 执行清理（默认 dry-run=true；确认后才 false）
scanary --rules-dir rules execute system-temp "%TEMP%" --dry-run=false

# 7. 复核释放量
scanary --format pretty report "C:\"
```

**铁律**：先预览后执行；回收站可恢复；释放量用执行结果的 `bytes_freed`，不要估算。
