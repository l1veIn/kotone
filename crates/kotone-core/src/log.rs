//! 轻量文件日志：GUI 子系统进程与提权进程都没有控制台，
//! eprintln 无处可去，排障依赖 ~/.kotone/kotone.log。
//!
//! 仅用于开发期诊断；追加写入，启动时截断到最近一次会话（保留 512KB 尾部）。

use std::io::Write;
use std::sync::Mutex;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

const MAX_KEEP_BYTES: u64 = 512 * 1024;

/// 初始化日志文件（每次启动新建会话头；文件过大时截断保留尾部）
pub fn init() {
    let dir = crate::settings::kotone_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("kotone.log");

    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_KEEP_BYTES {
            if let Ok(content) = std::fs::read(&path) {
                let tail = &content[(content.len() - MAX_KEEP_BYTES as usize)..];
                let _ = std::fs::write(&path, tail);
            }
        }
    }

    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        *FILE.lock().unwrap() = Some(f);
        log(&format!(
            "===== session start (pid {}) =====",
            std::process::id()
        ));
    }
}

/// 追加一行日志（带本地时间戳）。任何失败都静默忽略，绝不 panic。
pub fn log(msg: &str) {
    let ts = timestamp();
    if let Some(f) = FILE.lock().unwrap().as_mut() {
        let _ = writeln!(f, "[{ts}] {msg}");
        let _ = f.flush();
    }
    eprintln!("[kotone] {msg}");
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // 简化时间戳：epoch 秒 + 毫秒。精确日历时间在开发排障中非必需。
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}
