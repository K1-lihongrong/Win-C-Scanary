//! dedup 单元测试。

use std::fs;
use std::io::Write;

use wcs_dedup::find_duplicates;

fn write(path: &std::path::Path, content: &[u8]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    let mut f = fs::File::create(path).unwrap();
    f.write_all(content).unwrap();
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let mut d = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    d.push(format!("wcs_dedup_test_{}_{}", tag, nanos));
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn finds_simple_duplicates() {
    let dir = temp_dir("simple");
    write(&dir.join("a.txt"), b"hello world");
    write(&dir.join("b.txt"), b"hello world");
    write(&dir.join("c.txt"), b"different!!");
    let res = find_duplicates(&dir, 0).unwrap();
    assert_eq!(res.groups.len(), 1, "应有一组重复");
    let g = &res.groups[0];
    assert_eq!(g.paths.len(), 2);
    assert_eq!(g.size, 11);
    assert_eq!(g.waste_bytes, 11);
    assert_eq!(res.total_waste_bytes, 11);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_duplicates_when_all_unique() {
    let dir = temp_dir("unique");
    write(&dir.join("a"), b"aaa");
    write(&dir.join("b"), b"bbbb");
    write(&dir.join("c"), b"ccccc");
    let res = find_duplicates(&dir, 0).unwrap();
    assert!(res.groups.is_empty());
    assert_eq!(res.total_waste_bytes, 0);
    assert_eq!(res.files_scanned, 3);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn same_size_different_content_not_duplicates() {
    // 三阶段的价值：size 相同但内容不同，head-hash 即应区分
    let dir = temp_dir("samesize");
    write(&dir.join("a"), b"AAAAA");
    write(&dir.join("b"), b"BBBBB");
    let res = find_duplicates(&dir, 0).unwrap();
    assert!(res.groups.is_empty(), "同 size 不同内容不应判为重复");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn min_size_filters_small_files() {
    let dir = temp_dir("minsize");
    write(&dir.join("small_a"), b"hi");
    write(&dir.join("small_b"), b"hi");
    write(&dir.join("big_a"), b"0123456789");
    write(&dir.join("big_b"), b"0123456789");
    let res = find_duplicates(&dir, 5).unwrap();
    assert_eq!(res.groups.len(), 1, "只应有大文件组");
    assert_eq!(res.groups[0].size, 10);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn large_files_full_hash_confirms() {
    // 超过 HEAD_BYTES(8KiB)，且前缀相同但尾部不同 —— 必须 full-hash 才能区分
    let dir = temp_dir("large");
    let head = vec![b'x'; 16 * 1024];
    let mut a = head.clone();
    a.extend_from_slice(b"TAIL-A");
    let mut b = head.clone();
    b.extend_from_slice(b"TAIL-B");
    write(&dir.join("a.bin"), &a);
    write(&dir.join("b.bin"), &b);
    // 前缀相同、大小相同，但 full-hash 不同 → 不是重复
    let res = find_duplicates(&dir, 0).unwrap();
    assert!(res.groups.is_empty(), "尾部不同不应判为重复");

    // 现在写两个与 a 真正相同的（连同 a 共 3 份）
    write(&dir.join("c.bin"), &a);
    write(&dir.join("d.bin"), &a);
    let res2 = find_duplicates(&dir, 0).unwrap();
    assert_eq!(res2.groups.len(), 1, "应只有一组（a/c/d 相同）");
    assert_eq!(res2.groups[0].paths.len(), 3, "a/c/d 三份相同");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn nested_dirs_are_scanned() {
    let dir = temp_dir("nested");
    write(&dir.join("x/deep/a.dat"), b"same-content");
    write(&dir.join("y/other/b.dat"), b"same-content");
    let res = find_duplicates(&dir, 0).unwrap();
    assert_eq!(res.groups.len(), 1);
    assert_eq!(res.groups[0].paths.len(), 2);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn three_way_duplicate_waste_accounting() {
    let dir = temp_dir("three");
    write(&dir.join("a"), b"duplicate-content");
    write(&dir.join("b"), b"duplicate-content");
    write(&dir.join("c"), b"duplicate-content");
    let res = find_duplicates(&dir, 0).unwrap();
    assert_eq!(res.groups.len(), 1);
    let g = &res.groups[0];
    assert_eq!(g.paths.len(), 3);
    // 保留 1 份，浪费 2 份
    assert_eq!(g.waste_bytes, g.size * 2);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_and_zero_byte_files_ignored() {
    let dir = temp_dir("empty");
    write(&dir.join("a"), b"");
    write(&dir.join("b"), b"");
    let res = find_duplicates(&dir, 0).unwrap();
    assert!(res.groups.is_empty(), "0 字节文件不应参与重复检测");
    fs::remove_dir_all(&dir).ok();
}
