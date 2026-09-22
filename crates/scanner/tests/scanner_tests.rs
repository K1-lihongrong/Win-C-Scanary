//! Scanner 单元测试（walk / inspect / aligned_reader / perm 不变量）。
//!
//! 设计原则：**环境无关 + 确定性**。测试用自建临时目录，不依赖真实盘内容；
//! perm 相关只断言不变量（不假设当前用户是/不是管理员）。

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use wcs_scanner::aligned_reader::AlignedReader;
use wcs_scanner::{inspect, scan, scan_with_stats, DiskSpace, ScanMode, ScanOptions};

/// 在系统临时目录下建一个唯一测试目录。
fn make_temp_dir(tag: &str) -> PathBuf {
    let mut base = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    base.push(format!("wcs_scanner_test_{}_{}", tag, nanos));
    fs::create_dir_all(&base).unwrap();
    base
}

fn write_file(path: &std::path::Path, bytes: usize) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut f = fs::File::create(path).unwrap();
    f.write_all(&vec![b'x'; bytes]).unwrap();
}

// ---------- walk / scan ----------

#[test]
fn scan_empty_dir_has_zero_size() {
    let dir = make_temp_dir("empty");
    let node = scan(&dir).unwrap();
    assert!(node.is_dir);
    assert_eq!(node.size, 0);
    assert_eq!(node.file_count, 0);
    assert!(node.children.is_empty());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn scan_sums_nested_file_sizes() {
    let dir = make_temp_dir("nested");
    write_file(&dir.join("a.txt"), 100);
    write_file(&dir.join("sub/b.bin"), 250);
    write_file(&dir.join("sub/deep/c.dat"), 50);

    let node = scan(&dir).unwrap();
    assert_eq!(node.file_count, 3, "应统计到 3 个文件");
    assert_eq!(node.size, 400, "size 应为 100+250+50");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn scan_children_sorted_by_size_desc() {
    let dir = make_temp_dir("sort");
    write_file(&dir.join("small/a"), 10);
    write_file(&dir.join("big/a"), 1000);
    let node = scan(&dir).unwrap();
    assert!(node.children.len() >= 2);
    let sizes: Vec<u64> = node.children.iter().map(|c| c.size).collect();
    let mut sorted = sizes.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(sizes, sorted, "子目录应按 size 降序");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn scan_prunes_recycle_bin() {
    let dir = make_temp_dir("prune");
    write_file(&dir.join("keep/a.txt"), 10);
    // 被剪枝的目录不应计入
    write_file(&dir.join("$Recycle.Bin/junk.dat"), 9999);
    let node = scan(&dir).unwrap();
    assert_eq!(node.size, 10, "$Recycle.Bin 应被剪枝");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn scan_max_depth_limits_recursion() {
    let dir = make_temp_dir("depth");
    write_file(&dir.join("l1/l2/l3/deep.txt"), 100);
    let opts = ScanOptions {
        max_depth: Some(1),
        prefer_mft: false,
        ..Default::default()
    };
    let node = wcs_scanner::scan_with(&dir, opts, |_| {}).unwrap();
    assert_eq!(node.size, 0, "depth=1 时不应下探到 l1/l2/l3");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn scan_with_stats_reports_walk_mode() {
    let dir = make_temp_dir("stats");
    write_file(&dir.join("f.txt"), 42);
    let opts = ScanOptions {
        prefer_mft: false, // 强制走 walk，结果确定
        ..Default::default()
    };
    let (node, stats) = scan_with_stats(&dir, opts, |_| {}).unwrap();
    assert_eq!(stats.mode, ScanMode::Walk);
    assert_eq!(stats.files_seen, 1);
    assert_eq!(stats.bytes_seen, 42);
    assert_eq!(node.size, 42);
    fs::remove_dir_all(&dir).ok();
}

// ---------- inspect ----------

#[test]
fn inspect_returns_metadata_without_file_content() {
    let dir = make_temp_dir("inspect");
    write_file(&dir.join("one.txt"), 10);
    write_file(&dir.join("two.log"), 20);
    let meta = inspect(&dir, 5).unwrap();
    assert_eq!(meta.file_count, 2);
    assert_eq!(meta.size_bytes, 30);
    // 只返回元数据：扩展名占比存在
    let total_ext_bytes: u64 = meta.top_extensions.iter().map(|e| e.bytes).sum();
    assert_eq!(total_ext_bytes, 30);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn inspect_samples_capped() {
    let dir = make_temp_dir("inspect_samples");
    for i in 0..10 {
        fs::create_dir_all(dir.join(format!("d{}", i))).unwrap();
    }
    let meta = inspect(&dir, 3).unwrap();
    assert!(meta.sample_paths.len() <= 3, "samples 上限应生效");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn inspect_small_dir_not_truncated() {
    let dir = make_temp_dir("inspect_not_truncated");
    write_file(&dir.join("a.txt"), 5);
    let meta = inspect(&dir, 5).unwrap();
    assert!(!meta.truncated, "小目录不应标记截断");
    fs::remove_dir_all(&dir).ok();
}

// ---------- aligned_reader ----------

#[test]
fn aligned_reader_reads_full_stream() {
    // 用 Vec 作后端；内容 0..1000，读全部应一致
    let data: Vec<u8> = (0..1000u32).map(|i| (i % 256) as u8).collect();
    let cursor = std::io::Cursor::new(data.clone());
    let mut r = AlignedReader::new(cursor);
    let mut out = Vec::new();
    r.read_to_end(&mut out).unwrap();
    assert_eq!(out, data, "顺序读取应还原全部字节");
}

#[test]
fn aligned_reader_unaligned_read_window() {
    // 关键：非 512 对齐长度的读取，适配器应能返回正确字节
    let data: Vec<u8> = (0..2048u32).map(|i| (i % 256) as u8).collect();
    let cursor = std::io::Cursor::new(data.clone());
    let mut r = AlignedReader::new(cursor);
    let mut buf = [0u8; 517]; // 非扇区倍数
    r.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[..517]);
}

#[test]
fn aligned_reader_seek_then_read() {
    let data: Vec<u8> = (0..2048u32).map(|i| (i % 256) as u8).collect();
    let cursor = std::io::Cursor::new(data.clone());
    let mut r = AlignedReader::new(cursor);
    let pos = r.seek(SeekFrom::Start(600)).unwrap();
    assert_eq!(pos, 600);
    let mut buf = [0u8; 100];
    r.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[600..700], "seek 后读取应命中正确区间");
}

#[test]
fn aligned_reader_seek_current_and_end() {
    let data: Vec<u8> = (0..1000u32).map(|i| (i % 256) as u8).collect();
    let cursor = std::io::Cursor::new(data.clone());
    let mut r = AlignedReader::new(cursor);
    r.seek(SeekFrom::Start(100)).unwrap();
    let p = r.seek(SeekFrom::Current(50)).unwrap();
    assert_eq!(p, 150);
    let end = r.seek(SeekFrom::End(0)).unwrap();
    assert_eq!(end, 1000, "SeekFrom::End(0) 应返回流长度");
}

#[test]
fn aligned_reader_empty_read_is_zero() {
    let data = vec![1u8, 2, 3];
    let cursor = std::io::Cursor::new(data);
    let mut r = AlignedReader::new(cursor);
    let mut buf = [0u8; 0];
    assert_eq!(r.read(&mut buf).unwrap(), 0);
}

#[test]
fn aligned_reader_read_past_end_returns_zero() {
    let data = vec![1u8, 2, 3, 4, 5];
    let cursor = std::io::Cursor::new(data);
    let mut r = AlignedReader::new(cursor);
    r.seek(SeekFrom::Start(100)).unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(r.read(&mut buf).unwrap(), 0, "越界读取应返回 0");
}

// ---------- perm 不变量（环境无关） ----------

#[test]
fn can_use_mft_implies_admin_and_openable() {
    // 不变量：can_use_mft 为真 ⇒ is_admin 为真（非管理员一定不能 MFT）
    if wcs_scanner::can_use_mft('C') {
        assert!(wcs_scanner::is_admin(), "can_use_mft 为真时必须是管理员");
    }
}

#[test]
fn can_use_mft_nonexistent_drive_is_false() {
    // 不存在的盘符一定不能打开卷设备
    assert!(!wcs_scanner::can_use_mft('Q'), "不存在的卷不应可 MFT");
}

#[test]
fn disk_space_nonexistent_drive_is_none() {
    assert!(wcs_scanner::disk_space('Q').is_none(), "不存在的卷应返回 None");
}

#[test]
fn disk_space_existing_drive_consistent() {
    // C 盘通常存在；若存在则验证 total >= free 且 used = total - free
    if let Some(DiskSpace { total_bytes, free_bytes, used_bytes }) = wcs_scanner::disk_space('C') {
        assert!(total_bytes >= free_bytes, "total 应 >= free");
        assert_eq!(used_bytes, total_bytes - free_bytes, "used 应 = total - free");
        assert!(total_bytes > 0);
    }
}
