//! 通用文件下载器：流式下载 + 进度回调 + SHA256 校验 + 原子落盘（tmp + rename）。
//!
//! 供 model.rs 的模型下载使用。blocking 实现（reqwest blocking），
//! 调用方负责放到阻塞线程（Tauri 命令用 spawn_blocking，CLI 直接调用亦可）。
//!
//! 下载源（settings.download）：模型托管在 HuggingFace / GitHub，国内直连
//! 常超时。auto（默认）先走镜像（hf-mirror.com / ghProxy 前缀），失败回退
//! 官方源一次；official 只走官方；mirror 只走镜像。

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use kotone_core::settings::{DownloadConfig, DownloadSource};
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

// ---------- 下载源镜像（settings.download；规则见模块头注释） ----------

const HF_MIRROR_HOST: &str = "hf-mirror.com";
const GITHUB_PREFIX: &str = "https://github.com/";

/// 把官方 URL 重写为镜像 URL；无法重写（非 HF/GitHub 直链）时原样返回。
/// - `https://huggingface.co/...` → host 换 `hf-mirror.com`（路径、查询串不动）
/// - `https://github.com/...`     → 前面拼 ghProxy 前缀（代理服务整链透传）
pub fn rewrite_url(url: &str, cfg: &DownloadConfig) -> String {
    if let Some(rest) = url.strip_prefix("https://huggingface.co") {
        // rest 须为空或以 / ? 开头（防止 huggingface.co.evil.com 被误换 host）
        if rest.is_empty() || rest.starts_with('/') || rest.starts_with('?') {
            return format!("https://{HF_MIRROR_HOST}{rest}");
        }
    }
    if url.starts_with(GITHUB_PREFIX) {
        return format!("{}{}", cfg.gh_proxy, url);
    }
    url.to_string()
}

/// 按下载源策略生成候选 URL 列表（按尝试顺序）：
/// - official：[官方]
/// - mirror：  [镜像]（无法重写时 = [官方]，等于直通）
/// - auto：    [镜像, 官方]；URL 不可重写时只有 [官方]（不重复尝试同一地址）
pub fn candidate_urls(url: &str, cfg: &DownloadConfig) -> Vec<String> {
    match cfg.source {
        DownloadSource::Official => vec![url.to_string()],
        DownloadSource::Mirror => vec![rewrite_url(url, cfg)],
        DownloadSource::Auto => {
            let mirror = rewrite_url(url, cfg);
            if mirror == url {
                vec![url.to_string()]
            } else {
                vec![mirror, url.to_string()]
            }
        }
    }
}

/// 带下载源策略的下载：按 candidate_urls 顺序尝试，任一成功即返回；
/// 全部失败报最后一个错误。SHA256 校验失败同样触发回退（download_file
/// 失败时已删除 tmp，重试安全）——镜像偶尔会同步出不完整文件。
pub fn download_resolved(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    progress: Progress<'_>,
    cfg: &DownloadConfig,
) -> Result<(), String> {
    let candidates = candidate_urls(url, cfg);
    let mut last_err = String::new();
    for (i, cand) in candidates.iter().enumerate() {
        if i > 0 {
            log(&format!("镜像失败，回退重试：{cand}"));
        }
        match download_file(cand, dest, expected_sha256, progress) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

fn log(msg: &str) {
    eprintln!("[download] {msg}");
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

    // ---------- 下载源镜像（纯函数，不打网络） ----------

    use kotone_core::settings::DownloadSource;

    fn cfg(source: DownloadSource) -> DownloadConfig {
        DownloadConfig {
            source,
            gh_proxy: "https://ghfast.top/".into(),
        }
    }

    #[test]
    fn rewrite_hf_swaps_host_keeps_path_and_query() {
        let url = "https://huggingface.co/k2-fsa/x-asr/resolve/main/model.int8.onnx?download=true";
        assert_eq!(
            rewrite_url(url, &cfg(DownloadSource::Auto)),
            "https://hf-mirror.com/k2-fsa/x-asr/resolve/main/model.int8.onnx?download=true"
        );
    }

    #[test]
    fn rewrite_github_prefixes_proxy() {
        let url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/1.0/m.tar.bz2";
        assert_eq!(
            rewrite_url(url, &cfg(DownloadSource::Auto)),
            "https://ghfast.top/https://github.com/k2-fsa/sherpa-onnx/releases/download/1.0/m.tar.bz2"
        );
    }

    #[test]
    fn rewrite_others_passthrough() {
        let c = cfg(DownloadSource::Auto);
        let url = "https://example.com/model.bin";
        assert_eq!(rewrite_url(url, &c), url);
        // 同前缀钓鱼域名不应被换 host
        let evil = "https://huggingface.co.evil.com/x";
        assert_eq!(rewrite_url(evil, &c), evil);
    }

    #[test]
    fn candidates_official_only_original() {
        let url = "https://huggingface.co/a/b.onnx";
        assert_eq!(candidate_urls(url, &cfg(DownloadSource::Official)), vec![url]);
    }

    #[test]
    fn candidates_mirror_only_rewritten() {
        let url = "https://github.com/a/b.zip";
        assert_eq!(
            candidate_urls(url, &cfg(DownloadSource::Mirror)),
            vec!["https://ghfast.top/https://github.com/a/b.zip"]
        );
    }

    #[test]
    fn candidates_auto_mirror_then_official() {
        let url = "https://huggingface.co/a/b.onnx";
        assert_eq!(
            candidate_urls(url, &cfg(DownloadSource::Auto)),
            vec!["https://hf-mirror.com/a/b.onnx", url]
        );
    }

    #[test]
    fn candidates_auto_unrewriteable_single() {
        // 不可重写的 URL 在 auto 下不重复尝试同一地址
        let url = "https://example.com/m.bin";
        assert_eq!(candidate_urls(url, &cfg(DownloadSource::Auto)), vec![url]);
    }

    #[test]
    fn download_resolved_single_candidate_success_and_error() {
        // auto 对不可重写 URL 只产单候选：验证 download_resolved 直通语义。
        // 多候选回退顺序由 candidates_* 纯函数测试覆盖（真实回退需网络）。
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let err = download_resolved(
            "http://127.0.0.1:1/x.bin",
            &dest,
            None,
            &|_, _| {},
            &cfg(DownloadSource::Official),
        )
        .unwrap_err();
        assert!(err.contains("下载失败"), "err: {err}");

        static BODY: &[u8] = b"resolved ok";
        let good = serve_once(BODY);
        download_resolved(&good, &dest, None, &|_, _| {}, &cfg(DownloadSource::Official)).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }
}
