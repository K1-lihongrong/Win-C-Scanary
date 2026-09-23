//! wcs-executor 单元测试。
//!
//! 重点：dry-run 不删文件、guard 拦截生效、undo 日志写入、隔离区移动。

use std::path::PathBuf;
use wcs_executor::{execute, read_undo_log, Action, Granularity, Plan};
use wcs_guard::GuardConfig;

/// 构造一个宽松的 guard 配置，允许临时目录操作
fn test_guard(tmp: &std::path::Path) -> GuardConfig {
    GuardConfig {
        allowed_roots: vec![tmp.to_path_buf()],
        protect_c_drive_boundary: false,
        reject_reparse_points: false,
        extra_protected: Vec::new(),
    }
}

fn make_file(dir: &std::path::Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"test data").unwrap();
    p
}

#[test]
fn dry_run_does_not_delete_files() {
    let tmp = tempfile::tempdir().unwrap();
    let file = make_file(tmp.path(), "cache.tmp");
    assert!(file.exists());

    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");
    let plan = Plan {
        action: Action::Recycle,
        paths: vec![file.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let result = execute(&plan, &test_guard(tmp.path()), true, &undo_log, &quarantine).unwrap();

    assert!(!result.executed, "dry-run 不应标记为已执行");
    assert_eq!(result.matched_count, 1);
    assert!(file.exists(), "dry-run 不应删除文件");
    assert!(undo_log.exists(), "应写入 undo 日志");

    let entries = read_undo_log(&undo_log).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].reason.contains("dry-run"));
}

#[test]
fn guard_blocks_protected_path() {
    let tmp = tempfile::tempdir().unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    // 用一个 guard 配置，其中 allowed_roots 不含该路径，且路径命中保护模式
    let guard = GuardConfig {
        allowed_roots: vec![PathBuf::from("C:\\")],
        protect_c_drive_boundary: true,
        reject_reparse_points: false,
        extra_protected: Vec::new(),
    };
    let plan = Plan {
        action: Action::Recycle,
        paths: vec![PathBuf::from("C:\\Windows\\System32\\kernel32.dll")],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let result = execute(&plan, &guard, false, &undo_log, &quarantine).unwrap();

    assert_eq!(result.matched_count, 0, "受保护路径不应进入允许列表");
    assert_eq!(result.blocked.len(), 1, "应记录 1 个被拦截项");
    assert!(result.blocked[0].1.contains("受保护根") || result.blocked[0].1.contains("保护"));
}

#[test]
fn quarantine_moves_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = make_file(tmp.path(), "to-quarantine.tmp");
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Quarantine,
        paths: vec![file.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let result = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();

    assert!(result.executed);
    assert!(!file.exists(), "原文件应被移走");
    assert!(quarantine.exists(), "隔离区应被创建");

    let entries = read_undo_log(&undo_log).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].destination.is_some(), "隔离应记录目标路径");
    let dest = entries[0].destination.as_ref().unwrap();
    assert!(dest.exists(), "隔离目标文件应存在");
}

#[test]
fn blocked_paths_are_not_touched() {
    let tmp = tempfile::tempdir().unwrap();
    let file = make_file(tmp.path(), "should-survive.tmp");
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    // guard 只允许别的根，当前文件会被拦截
    let guard = GuardConfig {
        allowed_roots: vec![PathBuf::from("Z:\\nonexistent")],
        protect_c_drive_boundary: false,
        reject_reparse_points: false,
        extra_protected: Vec::new(),
    };
    let plan = Plan {
        action: Action::Delete,
        paths: vec![file.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let result = execute(&plan, &guard, false, &undo_log, &quarantine).unwrap();

    assert_eq!(result.matched_count, 0);
    assert!(file.exists(), "被 guard 拦截的文件不应被删除");
}

#[test]
fn undo_log_appends_multiple_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    for name in ["a.tmp", "b.tmp"] {
        let file = make_file(tmp.path(), name);
        let plan = Plan {
            action: Action::Recycle,
            paths: vec![file],
            reason: "test".into(),
            granularity: Granularity::File,
            system_cmd: None,
        };
        execute(&plan, &test_guard(tmp.path()), true, &undo_log, &quarantine).unwrap();
    }

    let entries = read_undo_log(&undo_log).unwrap();
    assert_eq!(entries.len(), 2, "两次执行应追加两条记录");
}

#[test]
fn directory_granularity_quarantine() {
    let tmp = tempfile::tempdir().unwrap();
    let subdir = tmp.path().join("cache-dir");
    std::fs::create_dir(&subdir).unwrap();
    make_file(&subdir, "inner.tmp");

    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");
    let plan = Plan {
        action: Action::Quarantine,
        paths: vec![subdir.clone()],
        reason: "test".into(),
        granularity: Granularity::Directory,
        system_cmd: None,
    };
    let result = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();
    assert!(result.executed);
    assert!(!subdir.exists(), "目录应被移走");
}

#[test]
fn recycle_reports_freed_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let f1 = make_file(tmp.path(), "a.tmp");
    std::fs::write(&f1, vec![0u8; 1000]).unwrap();
    let f2 = make_file(tmp.path(), "b.tmp");
    std::fs::write(&f2, vec![0u8; 2000]).unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Recycle,
        paths: vec![f1.clone(), f2.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let result = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();

    assert!(result.executed);
    assert_eq!(result.matched_count, 2);
    assert_eq!(result.total_bytes, 3000, "应统计到 3000 字节");
    assert!(!f1.exists() && !f2.exists(), "文件应被删除");

    let entries = read_undo_log(&undo_log).unwrap();
    let sum: u64 = entries.iter().map(|e| e.bytes_freed).sum();
    assert_eq!(sum, 3000, "undo 记录应含每项释放量");
}

#[test]
fn delete_reports_freed_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let f = make_file(tmp.path(), "del.tmp");
    std::fs::write(&f, vec![0u8; 5000]).unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Delete,
        paths: vec![f.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    // Delete 不可逆：需显式 allow_delete=true 才执行（M1 扩展）。
    let result = wcs_executor::execute_with(
        &plan,
        &test_guard(tmp.path()),
        false,
        &undo_log,
        &quarantine,
        false,
        true,
    )
    .unwrap();
    assert!(result.executed);
    assert_eq!(result.total_bytes, 5000);
    assert!(!f.exists());
}

#[test]
fn delete_is_rejected_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let f = make_file(tmp.path(), "del-guard.tmp");
    std::fs::write(&f, vec![0u8; 5000]).unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Delete,
        paths: vec![f.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    // 兼容入口 execute()：allow_delete 默认 false
    let result = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();
    assert!(f.exists(), "Delete 默认应被拒绝，文件必须还在");
    assert_eq!(result.total_bytes, 0, "未执行则释放量为 0");
    assert!(result.blocked.iter().any(|(_, r)| r.contains("不可逆")));
}

#[test]
fn recycle_directory_reports_total_size() {
    let tmp = tempfile::tempdir().unwrap();
    let sub = tmp.path().join("cache");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("x.bin"), vec![0u8; 4000]).unwrap();
    std::fs::write(sub.join("y.bin"), vec![0u8; 6000]).unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Recycle,
        paths: vec![sub.clone()],
        reason: "test".into(),
        granularity: Granularity::Directory,
        system_cmd: None,
    };
    let result = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();
    assert_eq!(result.total_bytes, 10000, "目录应递归统计 10000 字节");
    assert!(!sub.exists());
}

#[test]
fn prune_quarantine_removes_old_keeps_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let q = tmp.path().join("q");
    std::fs::create_dir_all(&q).unwrap();
    // 旧项：时间戳设为 30 天前
    let old_ms = (chrono::Utc::now().timestamp_millis() as u64) - 30 * 24 * 3600 * 1000;
    let old_file = q.join(format!("{}-old.tmp", old_ms));
    std::fs::write(&old_file, b"x").unwrap();
    // 新项：当前时间
    let now_ms = chrono::Utc::now().timestamp_millis() as u64;
    let new_file = q.join(format!("{}-new.tmp", now_ms));
    std::fs::write(&new_file, b"y").unwrap();
    // 无法解析时间戳的文件：保守不删
    let weird = q.join("not-a-timestamp.tmp");
    std::fs::write(&weird, b"z").unwrap();

    let removed = wcs_executor::prune_quarantine(&q, 7).unwrap();
    assert_eq!(removed, 1, "只应删除超龄项");
    assert!(!old_file.exists(), "旧项应被删");
    assert!(new_file.exists(), "新项应保留");
    assert!(weird.exists(), "无法解析时间戳的文件应保守保留");
}

#[test]
fn warn_is_blocked_by_default_and_allowed_with_flag() {
    let tmp = tempfile::tempdir().unwrap();
    // 构造一个落在 USER_DATA_PATTERNS 的路径：.../Documents/note.txt
    let docs = tmp.path().join("Documents");
    std::fs::create_dir_all(&docs).unwrap();
    let file = make_file(&docs, "note.txt");
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Recycle,
        paths: vec![file.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };

    // 默认：Warn 被拦截，文件必须还在
    let r1 = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();
    assert!(file.exists(), "默认路径下 Warn 项不应被删除");
    assert_eq!(r1.matched_count, 0);
    assert!(r1.blocked.iter().any(|(_, s)| s.contains("allow-warn")));

    // 显式放行：allow_warn=true，文件被回收
    let r2 = wcs_executor::execute_with(
        &plan,
        &test_guard(tmp.path()),
        false,
        &undo_log,
        &quarantine,
        true,
        false,
    )
    .unwrap();
    assert!(!file.exists(), "allow_warn=true 时 Warn 项应被处理");
    assert_eq!(r2.matched_count, 1);
}

#[test]
fn quarantine_restore_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let file = make_file(tmp.path(), "to-q.txt");
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Quarantine,
        paths: vec![file.clone()],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let r = execute(&plan, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();
    assert!(!file.exists());

    // 恢复该会话
    let rr = wcs_executor::restore_session(&undo_log, &r.session_id).unwrap();
    assert_eq!(rr.restored.len(), 1, "应恢复 1 项");
    assert!(file.exists(), "恢复后原文件应回来");
}

#[test]
fn restore_skips_delete_as_irreversible() {
    let tmp = tempfile::tempdir().unwrap();
    let f = make_file(tmp.path(), "gone.tmp");
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    let plan = Plan {
        action: Action::Delete,
        paths: vec![f],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    let r = wcs_executor::execute_with(
        &plan,
        &test_guard(tmp.path()),
        false,
        &undo_log,
        &quarantine,
        false,
        true,
    )
    .unwrap();
    let rr = wcs_executor::restore_session(&undo_log, &r.session_id).unwrap();
    assert!(rr.restored.is_empty(), "Delete 项不可恢复");
    assert!(rr.skipped.iter().any(|(_, s)| s.contains("不可逆")));
}

#[test]
fn list_sessions_merges_and_skips_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let undo_log = tmp.path().join("undo.jsonl");
    let quarantine = tmp.path().join("q");

    // 一次 dry-run（应被跳过）
    let f0 = make_file(tmp.path(), "dry.tmp");
    let p0 = Plan {
        action: Action::Recycle,
        paths: vec![f0],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    execute(&p0, &test_guard(tmp.path()), true, &undo_log, &quarantine).unwrap();

    // 一次真隔离（应出现）
    let f1 = make_file(tmp.path(), "real.tmp");
    let p1 = Plan {
        action: Action::Quarantine,
        paths: vec![f1],
        reason: "test".into(),
        granularity: Granularity::File,
        system_cmd: None,
    };
    execute(&p1, &test_guard(tmp.path()), false, &undo_log, &quarantine).unwrap();

    let sessions = wcs_executor::list_sessions(&undo_log).unwrap();
    assert_eq!(sessions.len(), 1, "dry-run 会话应被跳过");
    assert_eq!(sessions[0].count, 1);
}

#[test]
fn prune_quarantine_nonexistent_dir_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let q = tmp.path().join("nope");
    let removed = wcs_executor::prune_quarantine(&q, 7).unwrap();
    assert_eq!(removed, 0);
}
