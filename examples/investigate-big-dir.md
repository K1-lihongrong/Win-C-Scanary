# 示例：发现一个大目录，判断它是什么、能否清

> 目标：对扫描发现的大目录做深挖（只读元数据），再决定用哪条规则或迁移。

```bash
# 1. 扫描，找到可疑大目录
scanary --format pretty scan "C:\" --top 30

# 2. 深挖该目录（只读元数据，不读文件内容）
scanary inspect "C:\Users\me\AppData\Local\SomeApp"

# 3. 看它是缓存还是用户数据：
#    - 缓存 → 用对应规则 preview → execute
#    - 大缓存但不想删 → 迁移到其他盘
scanary --format pretty migrate "C:\" --top 20 --target D

# 4. 若是重复文件占用（只检测不删）
scanary --format pretty dedup "C:\Users\me\Downloads" --min-size 1024

# 5. 看空间是否被"簇对齐浪费"吃掉
scanary --format pretty audit "C:\" --top 20
```

**说明**：`inspect` 只读元数据（路径/大小/文件数/扩展名/样本），**不读文件内容**，隐私安全。
