//! 通用文件下载器：流式下载 + 进度回调 + SHA256 校验 + 原子落盘（tmp + rename）。
//!
//! 供 model.rs 的模型下载使用。blocking 实现（reqwest blocking），
//! 调用方负责放到阻塞线程（Tauri 命令用 spawn_blocking，CLI 直接调用亦可）。
//!
//! 下载源（settings.download）：模型优先使用清单内固定 revision 的 ModelScope
//! 镜像；其他 HuggingFace / GitHub 资源按配置改写。auto（默认）镜像失败后
//! 回退官方；official 只走官方；mirror 只走镜像。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kotone_core::settings::{DownloadConfig, DownloadSource};
use reqwest::header::{ACCEPT_ENCODING, CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};

/// 进度回调：（已下载字节数，总字节数（未知为 None））
pub type Progress<'a> = &'a dyn Fn(u64, Option<u64>);

/// 单块读取大小（256KB：进度足够平滑，syscall 又不至于太密）
const CHUNK: usize = 256 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// 单次网络读取最长等待时间；取消请求最迟会在该时间后重新获得执行机会。
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// 防止服务端持续滴流却永不完成。模型较大，给正常慢速下载保留四小时。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);

// ---------- 下载取消与并发互斥（P2-⑦） ----------

/// 全局取消标记：IPC `cancel_download` 置位；下载循环每块检查。
/// 取消不删除 `.part` 临时文件（下次启动自动断点续传）。
static CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
/// 下载互斥：GUI 与 CLI 共用同一实现，防并发重复下载同一模型
static DOWNLOAD_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 取消时返回的稳定前缀（前端靠它区分「取消」和真正的下载失败）。
pub const CANCELLED_MSG: &str = "下载已取消（临时文件已保留，可随时续传）";

/// 请求取消当前下载（幂等）
pub fn request_cancel() {
    CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

/// 下载循环是否应中止（每块读/每次候选回退前检查）
pub fn cancel_requested() -> bool {
    CANCEL_REQUESTED.load(Ordering::SeqCst)
}

pub fn cancelled_err() -> String {
    CANCELLED_MSG.to_string()
}

/// 该错误是否表示用户取消（回退源时必须立刻停，不能当成镜像失败）。
pub fn is_cancelled(err: &str) -> bool {
    err.starts_with("下载已取消")
}

async fn wait_until_cancelled() {
    loop {
        if cancel_requested() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 下载临界区守卫。离开作用域（包括错误返回和 panic 展开）时自动释放全局状态。
pub struct DownloadGuard;

impl Drop for DownloadGuard {
    fn drop(&mut self) {
        DOWNLOAD_ACTIVE.store(false, Ordering::SeqCst);
        CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    }
}

/// 进入下载临界区：已有下载进行中时拒绝（IPC 并发防重入）
pub fn begin_download() -> Result<DownloadGuard, String> {
    if DOWNLOAD_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("已有模型下载进行中，请等待完成或先取消".to_string());
    }
    CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    Ok(DownloadGuard)
}
/// ModelScope 的 LFS CDN 会拒绝没有 User-Agent 的匿名请求（403）。
/// 使用固定、可识别的应用标识；公开模型下载不需要用户 Token。
const USER_AGENT: &str = concat!("Kotone/", env!("CARGO_PKG_VERSION"), " model-downloader");

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
    // 网络中断时保留临时文件，下次从已有字节继续；内容校验失败仍会删除。
    download_to_tmp(url, &tmp, progress)?;

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
    fs::rename(&tmp, dest)
        .map_err(|e| format!("落盘失败（{} → {}）：{e}", tmp.display(), dest.display()))?;
    Ok(())
}

fn download_to_tmp(url: &str, tmp: &Path, progress: Progress<'_>) -> Result<(), String> {
    download_to_tmp_with_timeouts(
        url,
        tmp,
        progress,
        CONNECT_TIMEOUT,
        READ_TIMEOUT,
        REQUEST_TIMEOUT,
    )
}

fn download_to_tmp_with_timeouts(
    url: &str,
    tmp: &Path,
    progress: Progress<'_>,
    connect_timeout: Duration,
    read_timeout: Duration,
    request_timeout: Duration,
) -> Result<(), String> {
    // blocking ClientBuilder 没有暴露逐次读取超时。这里在当前阻塞线程内运行
    // 单线程异步 client，外部同步 API 不变，同时获得 read_timeout 与总超时。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("无法初始化下载运行时：{e}"))?;
    runtime.block_on(async {
        let existing = fs::metadata(tmp).map(|md| md.len()).unwrap_or(0);
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .read_timeout(read_timeout)
            .timeout(request_timeout)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| format!("无法初始化下载器：{e}"))?;
        let mut request = client.get(url).header(ACCEPT_ENCODING, "identity");
        if existing > 0 {
            request = request.header(RANGE, format!("bytes={existing}-"));
        }
        let mut resp = tokio::select! {
            biased;
            _ = wait_until_cancelled() => return Err(cancelled_err()),
            resp = request.send() => {
                // 重定向后的 ModelScope CDN URL 带短期 auth_key；错误日志不应把它输出。
                resp.map_err(|e| format!("下载失败（{url}）：{}", e.without_url()))?
            }
        };

        if resp.status() == StatusCode::RANGE_NOT_SATISFIABLE && existing > 0 {
            let total = resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(content_range_total);
            if total == Some(existing) {
                progress(existing, total);
                return Ok(());
            }
            return Err(format!(
                "下载源拒绝续传：本地已有 {existing} 字节，远端总大小为 {}",
                total.map_or_else(|| "未知".into(), |n| n.to_string())
            ));
        }
        if !resp.status().is_success() {
            return Err(format!("下载失败（{url}）：HTTP {}", resp.status()));
        }

        let resumed = existing > 0 && resp.status() == StatusCode::PARTIAL_CONTENT;
        let (start, total) = if resumed {
            let range = resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("下载源返回 206 但缺少 Content-Range（{url}）"))?;
            let (range_start, total) = parse_content_range(range)
                .ok_or_else(|| format!("下载源返回了无效 Content-Range：{range}"))?;
            if range_start != existing {
                return Err(format!(
                    "下载源续传位置不符：请求从 {existing} 开始，实际从 {range_start} 开始"
                ));
            }
            (existing, Some(total))
        } else {
            // 服务器忽略 Range 并返回 200 时安全地从头覆盖，不能把完整文件追加到 part。
            (0, resp.content_length())
        };
        let mut out = if resumed {
            OpenOptions::new()
                .append(true)
                .open(tmp)
                .map_err(|e| format!("无法继续写入临时文件 {}：{e}", tmp.display()))?
        } else {
            File::create(tmp).map_err(|e| format!("无法创建临时文件 {}：{e}", tmp.display()))?
        };

        let mut downloaded = start;
        progress(downloaded, total);
        loop {
            if cancel_requested() {
                // 取消：保留 .part 供续传（与网络中断同语义，不删临时文件）
                return Err(cancelled_err());
            }
            let chunk = tokio::select! {
                biased;
                _ = wait_until_cancelled() => return Err(cancelled_err()),
                chunk = resp.chunk() => {
                    chunk.map_err(|e| format!("下载中断：{}", e.without_url()))?
                }
            };
            let Some(chunk) = chunk else {
                break;
            };
            out.write_all(&chunk)
                .map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
            downloaded += chunk.len() as u64;
            progress(downloaded, total);
        }
        out.flush()
            .map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
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
    })
}

fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.parse().ok()?, total.parse().ok()?))
}

fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.parse().ok()
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
        if cancel_requested() {
            return Err(cancelled_err());
        }
        if i > 0 {
            log(&format!("镜像失败，回退重试：{cand}"));
        }
        match download_file(cand, dest, expected_sha256, progress) {
            Ok(()) => return Ok(()),
            Err(e) if is_cancelled(&e) => return Err(e),
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

    #[test]
    fn cancelled_error_is_recognized() {
        assert!(is_cancelled(CANCELLED_MSG));
        assert!(is_cancelled("下载已取消（临时文件已保留，可随时续传）"));
        assert!(!is_cancelled("下载失败（https://example）：error sending request"));
        assert!(!is_cancelled("大小不符：期望 1，实际 2"));
    }

    #[test]
    fn download_guard_releases_active_state_on_unwind() {
        let unwind = std::panic::catch_unwind(|| {
            let _guard = begin_download().unwrap();
            assert!(begin_download().is_err(), "守卫存活期间必须拒绝第二个下载");
            panic!("exercise Drop during unwind");
        });
        assert!(unwind.is_err());
        let guard = begin_download().expect("panic 展开后应自动释放下载状态");
        drop(guard);
    }

    #[test]
    fn stalled_response_hits_read_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = conn.read(&mut request);
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n")
                .unwrap();
            conn.flush().unwrap();
            std::thread::sleep(Duration::from_secs(1));
        });

        let dir = tempfile::tempdir().unwrap();
        let error = download_to_tmp_with_timeouts(
            &format!("http://127.0.0.1:{port}/stalled.bin"),
            &dir.path().join("stalled.part"),
            &|_, _| {},
            Duration::from_millis(200),
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.contains("下载中断"), "error: {error}");
    }

    /// 极简一次性 HTTP 服务器：返回固定 body
    fn serve_once(body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // 读完请求头（到空行）
            let mut buf = [0u8; 1024];
            let n = conn.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
            assert!(
                request.contains("user-agent: kotone/"),
                "下载请求必须携带 Kotone User-Agent：{request}"
            );
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            conn.write_all(head.as_bytes()).unwrap();
            conn.write_all(body).unwrap();
        });
        format!("http://127.0.0.1:{port}/file.bin")
    }

    /// 返回一次标准 206，顺便断言客户端确实从 expected_start 续传。
    fn serve_range_once(body: &'static [u8], expected_start: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = conn.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
            assert!(
                request.contains("user-agent: kotone/"),
                "续传请求必须携带 Kotone User-Agent：{request}"
            );
            assert!(
                request.contains(&format!("range: bytes={expected_start}-")),
                "缺少正确的 Range 请求头：{request}"
            );
            let suffix = &body[expected_start..];
            let head = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                suffix.len(),
                expected_start,
                body.len() - 1,
                body.len()
            );
            conn.write_all(head.as_bytes()).unwrap();
            conn.write_all(suffix).unwrap();
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

        let err =
            download_file(&url, &dest, Some("00".repeat(32).as_str()), &|_, _| {}).unwrap_err();
        assert!(err.contains("SHA256 校验失败"), "err: {err}");
        assert!(!dest.exists());
        assert!(
            !dir.path().join("out.part").exists(),
            "校验失败应删除临时文件"
        );
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

    #[test]
    fn download_resumes_existing_part_with_range() {
        static BODY: &[u8] = b"kotone resumable model";
        let split = 7;
        let url = serve_range_once(BODY, split);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        fs::write(dir.path().join("out.part"), &BODY[..split]).unwrap();

        download_file(&url, &dest, None, &|_, _| {}).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
        assert!(!dir.path().join("out.part").exists());
    }

    #[test]
    fn download_overwrites_part_when_server_ignores_range() {
        static BODY: &[u8] = b"complete response";
        let url = serve_once(BODY);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        fs::write(dir.path().join("out.part"), b"stale prefix").unwrap();

        download_file(&url, &dest, None, &|_, _| {}).unwrap();

        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }

    #[test]
    fn parses_content_range_headers() {
        assert_eq!(parse_content_range("bytes 7-20/21"), Some((7, 21)));
        assert_eq!(content_range_total("bytes */21"), Some(21));
        assert_eq!(parse_content_range("invalid"), None);
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
        assert_eq!(
            candidate_urls(url, &cfg(DownloadSource::Official)),
            vec![url]
        );
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
        download_resolved(
            &good,
            &dest,
            None,
            &|_, _| {},
            &cfg(DownloadSource::Official),
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), BODY);
    }

    /// 真实网络兼容性探针：公开 ModelScope LFS 文件应可匿名下载。
    ///
    /// 手动运行：
    /// cargo test -p kotone-stt modelscope_public_file_downloads_without_token -- --ignored
    #[test]
    #[ignore = "依赖 ModelScope 公网；发布前手动运行"]
    fn modelscope_public_file_downloads_without_token() {
        let cases = [
            (
                "https://www.modelscope.cn/models/yangchen1258/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05/resolve/aa50faf0a9e45f6ea8913762151d47679ba468d7/tokens.txt",
                "b818a60878b9aae978cbb8ad594acbd403d76d1af2e31ef4197c84e2dbdba27c",
                58_806u64,
            ),
            (
                "https://www.modelscope.cn/models/fengge2024/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/9bd9398d89294cbf1964af126ff13e6890394cbc/tokens.txt",
                "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
                315_894u64,
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        for (i, (url, sha, size)) in cases.iter().enumerate() {
            let dest = dir.path().join(format!("file-{i}.bin"));
            download_file(url, &dest, Some(sha), &|_, _| {}).unwrap();
            assert_eq!(fs::metadata(&dest).unwrap().len(), *size, "url={url}");
        }
    }
}
