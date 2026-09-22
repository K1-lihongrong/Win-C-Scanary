//! MFT 直读验证程序。
//!
//! 默认全量扫描 C:；设环境变量 `MFT_LIMIT=<n>` 可限制记录数以快速验证。
//! 需管理员权限。

use std::panic;
fn main() {
    panic::set_hook(Box::new(|i| eprintln!("PANIC: {}", i)));
    let t = std::time::Instant::now();
    let limit: Option<u64> = std::env::var("MFT_LIMIT").ok().and_then(|s| s.parse().ok());
    let r = panic::catch_unwind(|| wcs_scanner::mft::scan_volume('C', None, limit, |_, _| {}));
    match r {
        Ok(Ok((n, idx))) => println!("MFT OK: files={} size={} dirs={} ms={}", n.file_count, n.size, idx.len(), t.elapsed().as_millis()),
        Ok(Err(e)) => println!("MFT Err: {:#}", e),
        Err(_) => println!("MFT PANICKED"),
    }
}
