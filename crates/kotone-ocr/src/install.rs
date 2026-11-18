//! 本地模型安装与就绪校验。
//!
//! - 下载/复制先写入同目录独占临时文件，校验字节数与 SHA-256 后再原子改名发布；
//!   校验失败不会留下或替换任何已发布文件。
//! - 就绪检查逐文件核对存在、大小与 SHA-256；哈希结果按 (大小, 修改时间) 缓存。
//! - 不自动下载：只有显式调用 `install`（CLI `kotone-cli ocr install`）才会访问网络。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::manifest::{ModelFile, ModelPackage};

/// 单个文件的校验结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub name: &'static str,
    pub role: &'static str,
    pub present: bool,
    pub size_ok: bool,
    pub sha256_ok: bool,
    pub actual_bytes: Option<u64>,
}

/// 整个模型包的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageState {
    Ready,
    /// 任一文件缺失（且没有文件损坏）。
    NotInstalled,
    /// 存在但大小或哈希不符。
    Invalid(String),
}

impl PackageState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotInstalled => "not-installed",
            Self::Invalid(_) => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageStatus {
    pub state: PackageState,
    pub files: Vec<FileStatus>,
}

/// 哈希缓存键：路径 + 大小 + 修改时间。
type Stamp = (PathBuf, u64, Option<SystemTime>);

/// 校验器：缓存已通过的 (路径, 大小, mtime)，避免每次就绪检查都重算 18 MB 哈希。
#[derive(Default)]
pub struct Verifier {
    verified: Mutex<HashMap<PathBuf, Stamp>>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

impl Verifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// 校验模型包。`full=false` 时只信任缓存过的哈希（大小/mtime 变化即重算）。
    pub fn check(&self, dir: &Path, package: &ModelPackage) -> PackageStatus {
        let mut files = Vec::new();
        let mut missing = false;
        let mut invalid: Vec<String> = Vec::new();
        for file in package.files {
            let status = self.check_file(dir, file);
            if !status.present {
                missing = true;
            } else if !status.size_ok {
                invalid.push(format!(
                    "{} 大小为 {} 字节，期望 {}",
                    file.name,
                    status.actual_bytes.unwrap_or(0),
                    file.bytes
                ));
            } else if !status.sha256_ok {
                invalid.push(format!("{} SHA-256 校验失败", file.name));
            }
            files.push(status);
        }
        let state = if !invalid.is_empty() {
            PackageState::Invalid(invalid.join("；"))
        } else if missing {
            PackageState::NotInstalled
        } else {
            PackageState::Ready
        };
        PackageStatus { state, files }
    }

    fn check_file(&self, dir: &Path, file: &ModelFile) -> FileStatus {
        let path = dir.join(file.name);
        let mut status = FileStatus {
            name: file.name,
            role: file.role,
            present: false,
            size_ok: false,
            sha256_ok: false,
            actual_bytes: None,
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            return status;
        };
        if !meta.is_file() {
            return status;
        }
        status.present = true;
        status.actual_bytes = Some(meta.len());
        status.size_ok = meta.len() == file.bytes;
        if !status.size_ok {
            self.verified.lock().unwrap().remove(&path);
            return status;
        }
        let stamp: Stamp = (path.clone(), meta.len(), meta.modified().ok());
        if self.verified.lock().unwrap().get(&path) == Some(&stamp) {
            status.sha256_ok = true;
            return status;
        }
        status.sha256_ok = sha256_file(&path).is_ok_and(|h| h == file.sha256);
        let mut cache = self.verified.lock().unwrap();
        if status.sha256_ok {
            cache.insert(path, stamp);
        } else {
            cache.remove(&path);
        }
        status
    }

    /// 读取文件字节并在内存中校验大小与哈希（加载模型用：校验与加载使用同一份字节，无 TOCTOU）。
    pub fn read_verified(&self, dir: &Path, file: &ModelFile) -> Result<Vec<u8>, String> {
        let path = dir.join(file.name);
        let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("缺少模型文件 {}", file.name),
            _ => format!("读取模型文件 {} 失败: {e}", file.name),
        })?;
        if bytes.len() as u64 != file.bytes {
            return Err(format!(
                "{} 大小为 {} 字节，期望 {}",
                file.name,
                bytes.len(),
                file.bytes
            ));
        }
        if sha256_hex(&bytes) != file.sha256 {
            return Err(format!("{} SHA-256 校验失败", file.name));
        }
        Ok(bytes)
    }
}

/// 安装来源。
pub enum InstallSource<'a> {
    /// 从清单 URL 下载（显式网络访问）。
    Download,
    /// 从本地目录复制（离线安装；仍逐文件校验）。
    FromDir(&'a Path),
}

/// 安装进度回调：(文件名, 已写入字节, 总字节)。
pub type Progress<'a> = &'a (dyn Fn(&str, u64, u64) + Send + Sync);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// (文件名, 动作)：`kept`（已存在且校验通过）/ `installed`。
    pub files: Vec<(String, &'static str)>,
}

fn temp_path(dir: &Path, name: &str) -> PathBuf {
    let mut nonce = [0u8; 8];
    let _ = getrandom::getrandom(&mut nonce);
    dir.join(format!(".{name}.{}.part", hex(&nonce)))
}

/// 把已校验的临时文件发布为最终文件（同目录原子改名，替换损坏的旧文件）。
fn publish(temp: &Path, target: &Path) -> Result<(), String> {
    std::fs::rename(temp, target).map_err(|e| {
        let _ = std::fs::remove_file(temp);
        format!("发布 {} 失败: {e}", target.display())
    })
}

/// 安装模型包到 `dir`。已存在且校验通过的文件保留；其余文件经临时文件校验后发布。
pub async fn install(
    package: &ModelPackage,
    dir: &Path,
    source: InstallSource<'_>,
    progress: Progress<'_>,
) -> Result<InstallReport, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建模型目录失败: {e}"))?;
    let verifier = Verifier::new();
    let mut report = InstallReport { files: Vec::new() };
    let client = match source {
        InstallSource::Download => Some(
            reqwest::Client::builder()
                // ModelScope CDN 拒绝没有 User-Agent 的请求（403）；只发送通用客户端名
                .user_agent(concat!("kotone-ocr/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(std::time::Duration::from_secs(20))
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .map_err(|e| format!("创建下载客户端失败: {e}"))?,
        ),
        InstallSource::FromDir(_) => None,
    };
    for file in package.files {
        let status = verifier.check_file(dir, file);
        if status.present && status.size_ok && status.sha256_ok {
            report.files.push((file.name.to_string(), "kept"));
            continue;
        }
        let temp = temp_path(dir, file.name);
        let result = match (&source, &client) {
            (InstallSource::FromDir(src), _) => copy_verified(&src.join(file.name), &temp, file),
            (InstallSource::Download, Some(client)) => {
                download_verified(client, file, &temp, progress).await
            }
            _ => Err("内部错误：缺少下载客户端".into()),
        };
        if let Err(error) = result {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
        publish(&temp, &dir.join(file.name))?;
        report.files.push((file.name.to_string(), "installed"));
    }
    // 最终整体复核（防止并发安装或外部修改）
    match verifier.check(dir, package).state {
        crate::install::PackageState::Ready => Ok(report),
        other => Err(format!("安装后复核未通过: {other:?}")),
    }
}

fn copy_verified(src: &Path, temp: &Path, file: &ModelFile) -> Result<(), String> {
    let bytes = std::fs::read(src).map_err(|e| format!("读取 {} 失败: {e}", src.display()))?;
    check_bytes(file, &bytes)?;
    write_new(temp, &bytes)
}

fn check_bytes(file: &ModelFile, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 != file.bytes {
        return Err(format!(
            "{} 大小为 {} 字节，期望 {}（未安装）",
            file.name,
            bytes.len(),
            file.bytes
        ));
    }
    let actual = sha256_hex(bytes);
    if actual != file.sha256 {
        return Err(format!(
            "{} SHA-256 不符（实际 {actual}，期望 {}）；未安装",
            file.name, file.sha256
        ));
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("创建临时文件失败: {e}"))?;
    out.write_all(bytes)
        .and_then(|_| out.sync_all())
        .map_err(|e| format!("写入临时文件失败: {e}"))
}

async fn download_verified(
    client: &reqwest::Client,
    file: &ModelFile,
    temp: &Path,
    progress: Progress<'_>,
) -> Result<(), String> {
    let mut response = client
        .get(file.url)
        .send()
        .await
        .map_err(|e| format!("下载 {} 失败（网络）: {e}", file.name))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载 {} 失败：HTTP {}",
            file.name,
            response.status().as_u16()
        ));
    }
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(|e| format!("创建临时文件失败: {e}"))?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("下载 {} 中断: {e}", file.name))?
    {
        written += chunk.len() as u64;
        if written > file.bytes {
            return Err(format!(
                "{} 超过期望大小 {} 字节（未安装）",
                file.name, file.bytes
            ));
        }
        hasher.update(&chunk);
        out.write_all(&chunk)
            .map_err(|e| format!("写入临时文件失败: {e}"))?;
        progress(file.name, written, file.bytes);
    }
    out.sync_all()
        .map_err(|e| format!("写入临时文件失败: {e}"))?;
    drop(out);
    if written != file.bytes {
        return Err(format!(
            "{} 只下载了 {written} 字节，期望 {}（未安装）",
            file.name, file.bytes
        ));
    }
    let actual = hex(&hasher.finalize());
    if actual != file.sha256 {
        return Err(format!(
            "{} SHA-256 不符（实际 {actual}，期望 {}）；未安装",
            file.name, file.sha256
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 测试用小包：内容与哈希由测试生成，不触网、不使用真实权重。
    pub fn fake_package(det: &[u8], rec: &[u8]) -> ModelPackage {
        let files: &'static [ModelFile] = Box::leak(Box::new([
            ModelFile {
                name: "det.onnx",
                role: "det",
                url: "https://example.invalid/det.onnx",
                sha256: Box::leak(sha256_hex(det).into_boxed_str()),
                bytes: det.len() as u64,
            },
            ModelFile {
                name: "rec.onnx",
                role: "rec",
                url: "https://example.invalid/rec.onnx",
                sha256: Box::leak(sha256_hex(rec).into_boxed_str()),
                bytes: rec.len() as u64,
            },
        ]));
        ModelPackage {
            files,
            ..crate::manifest::KOREAN_MOBILE
        }
    }

    fn noop(_: &str, _: u64, _: u64) {}

    #[tokio::test]
    async fn missing_corrupt_and_copied_models_are_distinguished() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let package = fake_package(b"detector-bytes", b"recognizer-bytes");
        let verifier = Verifier::new();
        assert_eq!(
            verifier.check(dst.path(), &package).state,
            PackageState::NotInstalled
        );
        // 源目录里 rec 被篡改：安装失败，目标目录不留下文件或临时文件
        std::fs::write(src.path().join("det.onnx"), b"detector-bytes").unwrap();
        std::fs::write(src.path().join("rec.onnx"), b"recognizer-BYTES").unwrap();
        let err = install(
            &package,
            dst.path(),
            InstallSource::FromDir(src.path()),
            &noop,
        )
        .await
        .unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
        let leftovers: Vec<_> = std::fs::read_dir(dst.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(
            leftovers,
            vec!["det.onnx".to_string()],
            "只发布校验通过的文件"
        );
        // 修正后安装成功，已存在的 det 保留
        std::fs::write(src.path().join("rec.onnx"), b"recognizer-bytes").unwrap();
        let report = install(
            &package,
            dst.path(),
            InstallSource::FromDir(src.path()),
            &noop,
        )
        .await
        .unwrap();
        assert_eq!(
            report.files,
            vec![
                ("det.onnx".to_string(), "kept"),
                ("rec.onnx".to_string(), "installed")
            ]
        );
        assert_eq!(
            verifier.check(dst.path(), &package).state,
            PackageState::Ready
        );
        // 安装后被篡改（同大小）：哈希不符 → invalid（缓存因 mtime/内容重算）
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dst.path().join("det.onnx"), b"detector-BYTES").unwrap();
        let fresh = Verifier::new();
        let status = fresh.check(dst.path(), &package);
        assert!(matches!(status.state, PackageState::Invalid(ref m) if m.contains("det.onnx")));
        assert!(fresh
            .read_verified(dst.path(), package.file("det").unwrap())
            .is_err());
        // 截断：大小不符
        std::fs::write(dst.path().join("rec.onnx"), b"short").unwrap();
        let status = fresh.check(dst.path(), &package);
        assert!(matches!(status.state, PackageState::Invalid(ref m) if m.contains("大小")));
    }

    #[tokio::test]
    async fn download_verifies_size_and_hash_before_publishing() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/det.onnx"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"det-ok".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/rec.onnx"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"rec-tampered".to_vec()))
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let dir = tempfile::tempdir().unwrap();
        let det = ModelFile {
            name: "det.onnx",
            role: "det",
            url: Box::leak(format!("{}/det.onnx", server.uri()).into_boxed_str()),
            sha256: Box::leak(sha256_hex(b"det-ok").into_boxed_str()),
            bytes: 6,
        };
        let temp = dir.path().join("det.part");
        download_verified(&client, &det, &temp, &noop)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&temp).unwrap(), b"det-ok");
        let rec = ModelFile {
            name: "rec.onnx",
            role: "rec",
            url: Box::leak(format!("{}/rec.onnx", server.uri()).into_boxed_str()),
            sha256: Box::leak(sha256_hex(b"rec-original").into_boxed_str()),
            bytes: 12,
        };
        let err = download_verified(&client, &rec, &dir.path().join("rec.part"), &noop)
            .await
            .unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
        let oversize = ModelFile { bytes: 3, ..rec };
        let err = download_verified(&client, &oversize, &dir.path().join("x.part"), &noop)
            .await
            .unwrap_err();
        assert!(err.contains("超过期望大小"), "{err}");
    }
}
