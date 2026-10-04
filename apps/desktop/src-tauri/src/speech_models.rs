//! Desktop ownership of the optional Kokoro package. Network transport and cancellation
//! reuse the STT downloader; only a verified, complete package is published to the engine.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kotone_stt::{download, model};
use kotone_tts::{KOKORO_MODEL_DIR as MODEL_ID, REQUIRED_DIRS, REQUIRED_FILES};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{runtime::RuntimeManager, SharedState};

const ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2";
const ARCHIVE_SIZE: u64 = 147_031_220;
const ARCHIVE_SHA256: &str = "a1e94694776049035c4f2c6529f003aaece993c76aae9a78995831c3c4dcafc6";
const MAX_UNPACKED_BYTES: u64 = 1_073_741_824;
const MAX_ARCHIVE_ENTRIES: usize = 32_768;
const CANCELLED: &str = "下载已取消（本次临时文件已清理）";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechModelStatus {
    pub model_id: String,
    pub installed: bool,
    pub downloading: bool,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub phase: String,
    pub error: Option<String>,
    pub model_dir: String,
}

impl SpeechModelStatus {
    fn at(directory: &Path) -> Self {
        let installed = package_ready(directory);
        Self {
            model_id: MODEL_ID.into(),
            installed,
            downloading: false,
            downloaded_bytes: if installed { ARCHIVE_SIZE } else { 0 },
            total_bytes: ARCHIVE_SIZE,
            phase: if installed { "installed" } else { "idle" }.into(),
            error: None,
            model_dir: directory.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Default)]
pub struct SpeechModelManager {
    current: Mutex<Option<SpeechModelStatus>>,
}

impl SpeechModelManager {
    pub fn is_downloading(&self) -> bool {
        self.current
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|status| status.downloading)
    }
}

fn current_models_dir(app: &AppHandle) -> PathBuf {
    model::models_dir_from(&app.state::<SharedState>().settings.read().unwrap())
}

fn model_directory(models: &Path) -> PathBuf {
    models.join("speech").join(MODEL_ID)
}

#[tauri::command]
pub fn get_speech_model_status(app: AppHandle) -> SpeechModelStatus {
    let directory = model_directory(&current_models_dir(&app));
    let actual = SpeechModelStatus::at(&directory);
    let manager = app.state::<SpeechModelManager>();
    let previous = manager.current.lock().unwrap();
    match previous.as_ref() {
        Some(status) if status.downloading => status.clone(),
        Some(status) if status.model_dir == actual.model_dir && !actual.installed => {
            let mut result = status.clone();
            result.installed = false;
            if result.phase == "installed" {
                result.phase = "idle".into();
            }
            result
        }
        _ => actual,
    }
}

/// Cancellation is scoped to this manager's lease. A TTS cancel cannot cancel a later
/// STT task: publishing the inactive state and releasing the shared guard use one lock.
#[tauri::command]
pub fn cancel_speech_model_download(manager: tauri::State<'_, SpeechModelManager>) {
    let status = manager.current.lock().unwrap();
    if status.as_ref().is_some_and(|status| status.downloading) {
        download::request_cancel();
    }
}

#[tauri::command]
pub async fn download_speech_model(app: AppHandle) -> Result<SpeechModelStatus, String> {
    let lease = {
        let runtime = app.state::<RuntimeManager>();
        let _configuration = runtime.configuration_guard();
        let models = current_models_dir(&app);
        model::validate_models_dir_path(&models)?;
        let directory = model_directory(&models);
        let status = SpeechModelStatus::at(&directory);
        if status.installed {
            return Ok(status);
        }
        let global_guard = download::begin_download()?;
        let manager = app.state::<SpeechModelManager>();
        let mut current = manager.current.lock().unwrap();
        let mut status = status;
        status.downloading = true;
        status.phase = "downloading".into();
        *current = Some(status.clone());
        drop(current);
        let _ = app.emit("kotone://speech-model", &status);
        DownloadLease {
            app: app.clone(),
            models,
            global_guard: Some(global_guard),
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut lease = lease;
        let result = install(&lease.app, &lease.models);
        let status = lease.complete(result.as_ref().err());
        result.map(|()| status)
    })
    .await
    .map_err(|error| format!("语音模型下载任务异常：{error}"))?
}

struct DownloadLease {
    app: AppHandle,
    models: PathBuf,
    global_guard: Option<download::DownloadGuard>,
}

impl DownloadLease {
    fn complete(&mut self, error: Option<&String>) -> SpeechModelStatus {
        let manager = self.app.state::<SpeechModelManager>();
        let mut current = manager.current.lock().unwrap();
        let status = current.as_mut().expect("download lease owns its status");
        status.downloading = false;
        status.installed = error.is_none();
        status.phase = match error {
            None => "installed",
            Some(error) if download::is_cancelled(error) => "cancelled",
            Some(_) => "error",
        }
        .into();
        status.error = error.cloned();
        if error.is_none() {
            status.downloaded_bytes = ARCHIVE_SIZE;
        }
        // Hold the manager lock while releasing the global downloader, see cancel above.
        self.global_guard.take();
        let result = status.clone();
        drop(current);
        let _ = self.app.emit("kotone://speech-model", &result);
        result
    }
}

impl Drop for DownloadLease {
    fn drop(&mut self) {
        if self.global_guard.is_some() {
            self.complete(Some(&"语音模型下载任务中断，请重试".to_string()));
        }
    }
}

fn update_progress(app: &AppHandle, phase: &str, downloaded: u64) {
    let manager = app.state::<SpeechModelManager>();
    let mut current = manager.current.lock().unwrap();
    let Some(status) = current.as_mut() else {
        return;
    };
    status.phase = phase.into();
    status.downloaded_bytes = downloaded;
    let result = status.clone();
    drop(current);
    let _ = app.emit("kotone://speech-model", &result);
}

fn ensure_same_directory(app: &AppHandle, expected: &Path) -> Result<(), String> {
    if current_models_dir(app) != expected {
        Err("下载期间模型存储目录已改变，本次安装已取消；请在新目录重新下载".into())
    } else {
        Ok(())
    }
}

fn check_cancelled() -> Result<(), String> {
    if download::cancel_requested() {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn install(app: &AppHandle, models: &Path) -> Result<(), String> {
    check_cancelled()?;
    ensure_same_directory(app, models)?;
    let parent = models.join("speech");
    fs::create_dir_all(&parent).map_err(|error| format!("无法创建语音模型目录：{error}"))?;
    // Unique and on the destination volume: cleanup never targets another task's files,
    // and publishing the extracted directory is an atomic rename.
    let staging = tempfile::Builder::new()
        .prefix(".kokoro-install-")
        .tempdir_in(&parent)
        .map_err(|error| format!("无法创建语音模型临时目录：{error}"))?;
    let archive = staging.path().join("package.tar.bz2");
    let config = app
        .state::<SharedState>()
        .settings
        .read()
        .unwrap()
        .download
        .clone();
    let last_emit = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
    download::download_resolved(
        ARCHIVE_URL,
        &archive,
        Some(ARCHIVE_SHA256),
        &|bytes, _| {
            let complete = bytes == ARCHIVE_SIZE;
            if complete || last_emit.get().elapsed() >= Duration::from_millis(150) {
                update_progress(
                    app,
                    if complete { "verifying" } else { "downloading" },
                    bytes,
                );
                last_emit.set(Instant::now());
            }
        },
        &config,
    )
    .map_err(|error| {
        if download::is_cancelled(&error) {
            CANCELLED.into()
        } else {
            error
        }
    })?;
    check_cancelled()?;
    let actual_size = fs::metadata(&archive)
        .map_err(|error| error.to_string())?
        .len();
    if actual_size != ARCHIVE_SIZE {
        return Err(format!(
            "语音模型包大小不符：期望 {ARCHIVE_SIZE}，实际 {actual_size}"
        ));
    }
    update_progress(app, "extracting", ARCHIVE_SIZE);
    let payload = staging.path().join("payload");
    fs::create_dir(&payload).map_err(|error| error.to_string())?;
    let file = File::open(&archive).map_err(|error| error.to_string())?;
    extract_package(
        bzip2::read::BzDecoder::new(file),
        &payload,
        &check_cancelled,
    )?;
    if !package_ready(&payload) {
        return Err("语音模型包缺少必要文件或词典资源，请重新下载".into());
    }

    let runtime = app.state::<RuntimeManager>();
    let _configuration = runtime.configuration_guard();
    check_cancelled()?;
    ensure_same_directory(app, models)?;
    let destination = model_directory(models);
    let backup = staging.path().join("previous");
    if let Err(error) = publish_package(&payload, &destination, &backup) {
        // If Windows refuses rollback (e.g. another process opened the destination),
        // never let TempDir's cleanup delete the user's recoverable prior model.
        if backup.exists() {
            let retained = staging.keep();
            return Err(format!("{error}；恢复文件保留在 {}", retained.display()));
        }
        return Err(error);
    }
    // The guard also excludes set_models_dir while installing; verify the final location.
    ensure_same_directory(app, models)?;
    if !package_ready(&destination) {
        let retained = staging.keep();
        return Err(format!(
            "语音模型安装后校验失败；恢复文件保留在 {}",
            retained.display()
        ));
    }
    Ok(())
}

fn package_ready(directory: &Path) -> bool {
    if !kotone_tts::model_ready(directory) {
        return false;
    }
    let Ok(metadata) = fs::symlink_metadata(directory) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    REQUIRED_FILES.iter().all(|name| {
        fs::symlink_metadata(directory.join(name)).is_ok_and(|metadata| {
            metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() > 0
        })
    }) && REQUIRED_DIRS.iter().all(|name| {
        let path = directory.join(name);
        fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            && fs::read_dir(path)
                .is_ok_and(|mut entries| entries.next().is_some_and(|entry| entry.is_ok()))
    })
}

/// Retain the complete archive, including dictionaries and licenses. Do not use tar's
/// link/unpack behavior: accept only directories and ordinary files at validated paths.
fn extract_package(
    reader: impl Read,
    destination: &Path,
    cancelled: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let mut names = HashSet::new();
    let mut total = 0u64;
    for (index, entry) in archive
        .entries()
        .map_err(|error| format!("无法读取模型包：{error}"))?
        .enumerate()
    {
        cancelled()?;
        if index >= MAX_ARCHIVE_ENTRIES {
            return Err("模型包包含过多条目".into());
        }
        let mut entry = entry.map_err(|error| format!("模型包条目损坏：{error}"))?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err("模型包包含不支持的链接或特殊文件".into());
        }
        let relative = safe_archive_path(entry.path_bytes().as_ref())?;
        if relative.as_os_str().is_empty() {
            if entry_type.is_dir() {
                continue;
            }
            return Err("模型包根条目不是目录".into());
        }
        // Windows paths are case-insensitive. Refuse alias/duplicate entries on every OS.
        if !names.insert(relative.to_string_lossy().to_ascii_lowercase()) {
            return Err("模型包包含重复路径".into());
        }
        let output = destination.join(&relative);
        if entry_type.is_dir() {
            fs::create_dir_all(&output).map_err(|error| format!("创建模型子目录失败：{error}"))?;
            continue;
        }
        let size = entry.size();
        total = total.checked_add(size).ok_or("模型包解压大小溢出")?;
        if total > MAX_UNPACKED_BYTES {
            return Err("模型包解压大小超出预期".into());
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| format!("创建模型文件失败：{error}"))?;
        let mut buffer = [0u8; 256 * 1024];
        let mut written = 0u64;
        loop {
            cancelled()?;
            let count = entry
                .read(&mut buffer)
                .map_err(|error| format!("解压模型失败：{error}"))?;
            if count == 0 {
                break;
            }
            out.write_all(&buffer[..count])
                .map_err(|error| format!("写入模型文件失败：{error}"))?;
            written += count as u64;
        }
        if written != size {
            return Err("模型包中的文件不完整".into());
        }
        out.sync_all()
            .map_err(|error| format!("保存模型文件失败：{error}"))?;
    }
    Ok(())
}

fn safe_archive_path(bytes: &[u8]) -> Result<PathBuf, String> {
    let raw = std::str::from_utf8(bytes).map_err(|_| "模型包包含无效路径编码")?;
    if raw.contains('\\') || raw.starts_with('/') {
        return Err("模型包包含越界路径".into());
    }
    let raw = raw.trim_end_matches('/');
    let raw = raw.strip_prefix("./").unwrap_or(raw);
    let mut parts = raw.split('/');
    if parts.next() != Some(MODEL_ID) {
        return Err("模型包顶层目录不符".into());
    }
    let mut result = PathBuf::new();
    for part in parts {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|ch| ch.is_control() || ":<>\"|?*".contains(ch))
            || reserved
        {
            return Err("模型包包含不安全路径".into());
        }
        result.push(part);
    }
    Ok(result)
}

fn publish_package(payload: &Path, destination: &Path, backup: &Path) -> Result<(), String> {
    let exists = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("现有模型路径不是普通目录，无法替换".into());
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("读取现有模型失败：{error}")),
    };
    if exists {
        fs::rename(destination, backup)
            .map_err(|error| format!("模型正在使用或无法备份，请停止播报后重试：{error}"))?;
    }
    if let Err(error) = fs::rename(payload, destination) {
        if exists {
            fs::rename(backup, destination).map_err(|restore| {
                format!(
                    "安装失败：{error}；旧模型恢复失败：{restore}，保留在 {}",
                    backup.display()
                )
            })?;
        }
        return Err(format!("安装语音模型失败：{error}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_with(name: &str, kind: tar::EntryType, body: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        let name_bytes = name.as_bytes();
        header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
        header.set_entry_type(kind);
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, body).unwrap();
        builder.into_inner().unwrap()
    }

    #[test]
    fn archive_paths_reject_traversal_windows_aliases_and_other_roots() {
        for name in [
            "../outside",
            "/tmp/file",
            "C:/file",
            "wrong/tokens.txt",
            "kokoro-int8-multi-lang-v1_1/../outside",
            "kokoro-int8-multi-lang-v1_1/dict/x:ads",
            "kokoro-int8-multi-lang-v1_1/CON.txt",
            "kokoro-int8-multi-lang-v1_1/dict/file.",
            "kokoro-int8-multi-lang-v1_1/dict\\outside",
        ] {
            assert!(safe_archive_path(name.as_bytes()).is_err(), "{name}");
        }
        assert_eq!(
            safe_archive_path(format!("{MODEL_ID}/espeak-ng-data/voices/zh").as_bytes()).unwrap(),
            PathBuf::from("espeak-ng-data/voices/zh")
        );
    }

    #[test]
    fn unpack_keeps_nested_resources_and_rejects_links() {
        let temporary = tempfile::tempdir().unwrap();
        let bytes = tar_with(
            &format!("{MODEL_ID}/espeak-ng-data/voices/zh"),
            tar::EntryType::Regular,
            b"voice",
        );
        extract_package(bytes.as_slice(), temporary.path(), &|| Ok(())).unwrap();
        assert_eq!(
            fs::read(temporary.path().join("espeak-ng-data/voices/zh")).unwrap(),
            b"voice"
        );
        for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
            let bytes = tar_with(&format!("{MODEL_ID}/evil"), kind, b"");
            assert!(
                extract_package(bytes.as_slice(), temporary.path(), &|| Ok(()))
                    .unwrap_err()
                    .contains("链接")
            );
        }
    }

    #[test]
    fn cancelled_extraction_writes_nothing() {
        let temporary = tempfile::tempdir().unwrap();
        let bytes = tar_with(
            &format!("{MODEL_ID}/tokens.txt"),
            tar::EntryType::Regular,
            b"tokens",
        );
        let error = extract_package(
            bytes.as_slice(),
            temporary.path(),
            &|| Err(CANCELLED.into()),
        )
        .unwrap_err();
        assert!(download::is_cancelled(&error));
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 0);
    }

    #[test]
    fn failed_publish_restores_previous_installation() {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("installed");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("existing"), b"preserve").unwrap();
        assert!(publish_package(
            &temporary.path().join("missing"),
            &destination,
            &temporary.path().join("backup")
        )
        .is_err());
        assert_eq!(fs::read(destination.join("existing")).unwrap(), b"preserve");
    }

    #[test]
    fn incomplete_package_is_never_reported_installed() {
        let temporary = tempfile::tempdir().unwrap();
        for name in REQUIRED_FILES {
            let path = temporary.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"resource").unwrap();
        }
        for name in REQUIRED_DIRS {
            fs::create_dir_all(temporary.path().join(name)).unwrap();
            fs::write(temporary.path().join(name).join("data"), b"resource").unwrap();
        }
        assert!(package_ready(temporary.path()));
        fs::write(temporary.path().join("voices.bin"), b"").unwrap();
        assert!(!package_ready(temporary.path()));
    }

    /// Offline package smoke test. The caller supplies an already-downloaded official
    /// archive; extraction uses a disposable directory, never the configured model path.
    #[test]
    #[ignore = "requires KOTONE_KOKORO_TEST_ARCHIVE pointing to the official local archive"]
    fn official_archive_unpacks_complete_package() {
        let path = PathBuf::from(
            std::env::var_os("KOTONE_KOKORO_TEST_ARCHIVE")
                .expect("set KOTONE_KOKORO_TEST_ARCHIVE to a local tar.bz2"),
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), ARCHIVE_SIZE);
        assert_eq!(download::sha256_file(&path).unwrap(), ARCHIVE_SHA256);
        let temporary = tempfile::tempdir().unwrap();
        extract_package(
            bzip2::read::BzDecoder::new(File::open(path).unwrap()),
            temporary.path(),
            &|| Ok(()),
        )
        .unwrap();
        assert!(
            package_ready(temporary.path()),
            "official package must retain all engine resources"
        );
        assert!(temporary.path().join("LICENSE").is_file());
    }
}
