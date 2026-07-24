//! 通用文件下载器：流式下载 + 进度回调 + SHA256 校验 + 原子落盘（tmp + rename）。
//!
//! 供 model.rs 的模型/二进制下载使用。blocking 实现（reqwest blocking），
//! 调用方负责放到阻塞线程（Tauri 命令用 spawn_blocking，CLI 直接调用亦可）。

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

/// 进度回调：（已下载字节数，总字节数（未知为 None））
pub type Progress<'a> = &'a dyn Fn(u64, Option<u64>);

/// 单块读取大小（256KB：进度足够平滑，syscall 又不至于太密）
const CHUNK: usize = 256 * 1024;

/// 计算文件的 SHA256（小写 hex）
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| format!("无法打开 {}：{e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

/// 流式下载 url 到 dest：
/// 1. 写入 `dest.with_extension("part")` 临时文件，边下边算 SHA256、报进度；
/// 2. 若提供 expected_sha256 则校验，不符删除临时文件并报错；
/// 3. rename 原子落盘（同目录同卷；目标已存在先删除，Windows rename 不覆盖）。
pub fn download_file(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    progress: Progress<'_>,
) -> Result<(), String> {
    let parent = dest
        .parent()
        .ok_or_else(|| format!("目标路径无父目录：{}", dest.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("无法创建目录 {}：{e}", parent.display()))?;

    let tmp = dest.with_extension("part");
    // 失败时尽量清掉临时文件
    let result = download_to_tmp(url, &tmp, progress);
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    if let Some(expected) = expected_sha256 {
        let actual = sha256_file(&tmp)?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "SHA256 校验失败：期望 {expected}，实际 {actual}（文件已删除，请重试）"
            ));
        }
    }

    if dest.exists() {
        fs::remove_file(dest).map_err(|e| format!("无法替换旧文件 {}：{e}", dest.display()))?;
    }
    fs::rename(&tmp, dest).map_err(|e| {
        format!(
            "落盘失败（{} → {}）：{e}",
            tmp.display(),
            dest.display()
        )
    })?;
    Ok(())
}

fn download_to_tmp(url: &str, tmp: &Path, progress: Progress<'_>) -> Result<(), String> {
    let resp = reqwest::blocking::get(url)
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("下载失败（{url}）：{e}"))?;
    let total = resp.content_length();
    let mut resp = resp;
    let mut out = File::create(tmp)
        .map_err(|e| format!("无法创建临时文件 {}：{e}", tmp.display()))?;

    let mut downloaded: u64 = 0;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("下载中断：{e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
        downloaded += n as u64;
        progress(downloaded, total);
    }
    out.flush().map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
    if let Some(t) = total {
        if downloaded != t {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("下载不完整：{downloaded}/{t} 字节"),
            )
            .to_string());
        }
    }
    Ok(())
}

/// bytes → 小写 hex（不引 hex crate，几行的事）
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn sha256_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.bin");
        fs::write(&p, b"hello kotone").unwrap();
        // echo -n "hello kotone" | sha256sum
        assert_eq!(
            sha256_file(&p).unwrap(),
            "5ea673601ae0ff62c361e4ef7c54faeefd3462fec6d90f2b295ba0758762e772"
        );
    }

    #[test]
    fn hex_lower_works() {
        assert_eq!(hex_lower(&[0x00, 0xff, 0x1a]), "00ff1a");
    }

    /// 极简一次性 HTTP 服务器：返回固定 body
    fn serve_once(body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // 读完请求头（到空行）
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            conn.write_all(head.as_bytes()).unwrap();
            conn.write_all(body).unwrap();
        });
        format!("http://127.0.0.1:{port}/file.bin")
    }

    #[test]
    fn download_ok_with_progress_and_rename() {
        static BODY: &[u8] = b"kotone download payload";
        let url = serve_once(BODY);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");

        let calls = std::sync::Mutex::new(Vec::new());
        download_file(&url, &dest, None, &|done, total| {
            calls.lock().unwrap().push((done, total));
        })
        .unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
        let calls = calls.lock().unwrap();
        assert!(!calls.is_empty(), "应有进度回调");
        let (last_done, last_total) = *calls.last().unwrap();
        assert_eq!(last_done, BODY.len() as u64);
        assert_eq!(last_total, Some(BODY.len() as u64));
        assert!(!dir.path().join("out.part").exists(), "临时文件应已改名");
    }

    #[test]
    fn download_sha256_mismatch_cleans_tmp() {
        static BODY: &[u8] = b"tampered";
        let url = serve_once(BODY);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");

        let err = download_file(&url, &dest, Some("00".repeat(32).as_str()), &|_, _| {})
            .unwrap_err();
        assert!(err.contains("SHA256 校验失败"), "err: {err}");
        assert!(!dest.exists());
        assert!(!dir.path().join("out.part").exists(), "校验失败应删除临时文件");
    }

    #[test]
    fn download_sha256_match_ok() {
        static BODY: &[u8] = b"hello kotone";
        let url = serve_once(BODY);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        download_file(
            &url,
            &dest,
            Some("5ea673601ae0ff62c361e4ef7c54faeefd3462fec6d90f2b295ba0758762e772"),
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }
}
