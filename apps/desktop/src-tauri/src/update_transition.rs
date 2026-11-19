//! 0.1.x 的两条更新通道。旧通道继续提供维护版，0.2 通道由本机分桶决定。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kotone_core::settings::kotone_dir;
use serde::Serialize;
use tauri::{ipc::Channel, AppHandle, State, Url};
use tauri_plugin_updater::{Update, UpdaterBuilder, UpdaterExt};

const TRANSITION_ENDPOINT: &str =
    "https://kotone-1258931230.cos.ap-guangzhou.myqcloud.com/releases/latest-0.2.json";
const BUCKET_FILE: &str = "update-rollout-bucket";
const BUCKET_COUNT: u8 = 100;
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct InstallLock(tokio::sync::Mutex<()>);

#[derive(Serialize)]
pub struct UpdateOffer {
    version: String,
    notes: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DownloadEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        content_length: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        chunk_length: usize,
    },
    Finished,
}

fn belongs_to_stable_channel(version: &str, channel: &str) -> bool {
    // 传给这里的是插件已解析并规范化的 SemVer；预发行版由玩家自己安装。
    let stable_version = version.split('+').next().unwrap_or(version);
    stable_version.starts_with(channel) && !stable_version.contains('-')
}

fn rollout_percent(raw: &serde_json::Value) -> Option<u8> {
    let percent = raw.get("rolloutPercent")?.as_u64()?;
    (percent <= u64::from(BUCKET_COUNT)).then_some(percent as u8)
}

fn eligible(bucket: u8, percent: u8) -> bool {
    bucket < percent
}

fn read_bucket(path: &Path) -> io::Result<u8> {
    fs::read_to_string(path)?
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|bucket| *bucket < BUCKET_COUNT)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "更新分桶文件格式错误"))
}

fn local_bucket(home: &Path) -> io::Result<u8> {
    let path = home.join(BUCKET_FILE);
    match read_bucket(&path) {
        Ok(bucket) => return Ok(bucket),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    fs::create_dir_all(home)?;
    let bucket = fastrand::u8(..BUCKET_COUNT);
    let mut pending = tempfile::NamedTempFile::new_in(home)?;
    writeln!(pending, "{bucket}")?;
    pending.as_file().sync_all()?;
    match pending.persist_noclobber(&path) {
        Ok(_) => Ok(bucket),
        Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => read_bucket(&path),
        Err(e) => Err(e.error),
    }
}

async fn query(builder: UpdaterBuilder) -> Result<Option<Update>, String> {
    let updater = builder.build().map_err(|error| error.to_string())?;
    tokio::time::timeout(CHECK_TIMEOUT, updater.check())
        .await
        .map_err(|_| "检查更新超时".to_string())?
        .map_err(|error| error.to_string())
}

async fn offered_update(app: &AppHandle) -> Result<Option<Update>, String> {
    // 先提供旧通道的修补版。旧地址意外指向 0.2 时，仍不能绕过灰度。
    let maintenance = query(app.updater_builder()).await;
    if let Ok(Some(update)) = &maintenance {
        if belongs_to_stable_channel(&update.version, "0.1.") {
            return Ok(Some(update.clone()));
        }
    }

    let endpoint = Url::parse(TRANSITION_ENDPOINT).map_err(|error| error.to_string())?;
    let builder = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?
        .header("Cache-Control", "no-cache")
        .map_err(|error| error.to_string())?;
    let transition = query(builder).await;
    if let Ok(Some(update)) = transition {
        if belongs_to_stable_channel(&update.version, "0.2.") {
            if let Some(percent) = rollout_percent(&update.raw_json).filter(|percent| *percent > 0)
            {
                let bucket = local_bucket(&kotone_dir())
                    .map_err(|error| format!("无法读取本机更新分桶：{error}"))?;
                if eligible(bucket, percent) {
                    return Ok(Some(update));
                }
            }
        }
    }
    // 灰度地址可在放量前不存在，旧通道检查仍应正常。
    maintenance.map(|_| None)
}

#[tauri::command]
pub async fn check_kotone_update(app: AppHandle) -> Result<Option<UpdateOffer>, String> {
    Ok(offered_update(&app).await?.map(|update| UpdateOffer {
        version: update.version,
        notes: update.body,
    }))
}

#[tauri::command]
pub async fn install_kotone_update(
    app: AppHandle,
    lock: State<'_, InstallLock>,
    version: String,
    on_download: Channel<DownloadEvent>,
) -> Result<(), String> {
    let _guard = lock
        .0
        .try_lock()
        .map_err(|_| "已有更新进行中".to_string())?;
    let update = offered_update(&app)
        .await?
        .filter(|offer| offer.version == version)
        .ok_or_else(|| "此版本暂时不可用，请重新检查更新".to_string())?;

    let current = update.current_version.clone();
    tauri::async_runtime::spawn_blocking(move || backup_home(&kotone_dir(), &current))
        .await
        .map_err(|error| format!("备份未完成，更新已停止：{error}"))?
        .map_err(|error| format!("备份未完成，更新已停止：{error}"))?;

    let mut started = false;
    let bytes = update
        .download(
            |chunk_length, content_length| {
                if !started {
                    let _ = on_download.send(DownloadEvent::Started { content_length });
                    started = true;
                }
                let _ = on_download.send(DownloadEvent::Progress { chunk_length });
            },
            || {
                let _ = on_download.send(DownloadEvent::Finished);
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    // 用户等待备份和下载期间可能已经停止放量。包内容已由插件验签。
    let still_offered = offered_update(&app).await?.is_some_and(|offer| {
        offer.version == update.version
            && offer.signature == update.signature
            && offer.download_url == update.download_url
    });
    if !still_offered {
        return Err("此版本暂时不可用，已保留备份并停止安装".into());
    }
    update.install(bytes).map_err(|error| error.to_string())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Windows 的目录联接也可能指向数据目录之外。
        const REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn require_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("无法完整备份目录 {}", path.display()),
        ));
    }
    Ok(())
}

fn copy_contents(from: &Path, to: &Path, root: bool) -> io::Result<()> {
    let before = fs::symlink_metadata(from)?.modified()?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if root && name.eq_ignore_ascii_case("backup") {
            continue;
        }
        let source = entry.path();
        let target = to.join(name);
        let metadata = fs::symlink_metadata(&source)?;
        if is_link(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("数据目录含有链接，无法完整备份 {}", source.display()),
            ));
        }
        if metadata.is_dir() {
            fs::create_dir(&target)?;
            copy_contents(&source, &target, false)?;
        } else if metadata.is_file() {
            let bytes = fs::copy(&source, &target)?;
            let updated = fs::symlink_metadata(&source)?;
            if is_link(&updated)
                || bytes != metadata.len()
                || bytes != updated.len()
                || metadata.modified()? != updated.modified()?
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("文件在备份时发生变化 {}", source.display()),
                ));
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("无法备份文件 {}", source.display()),
            ));
        }
    }
    let after = fs::symlink_metadata(from)?;
    if is_link(&after) || before != after.modified()? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("目录在备份时发生变化 {}", from.display()),
        ));
    }
    Ok(())
}

fn backup_home(home: &Path, version: &str) -> io::Result<PathBuf> {
    require_directory(home)?;
    let backup = home.join("backup");
    match fs::create_dir(&backup) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => require_directory(&backup)?,
        Err(error) => return Err(error),
    }
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    let final_path = backup.join(format!("{version}-{time}"));
    let staging = tempfile::Builder::new()
        .prefix(".partial-")
        .tempdir_in(&backup)?;
    copy_contents(home, staging.path(), true)?;
    // 完整复制后才发布备份。正常失败时 TempDir 会清理未完成的目录。
    fs::rename(staging.path(), &final_path)?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rollout_accepts_only_valid_integer_percent_and_stable_channel() {
        for invalid in [
            json!({}),
            json!({"rolloutPercent": "50"}),
            json!({"rolloutPercent": 1.5}),
            json!({"rolloutPercent": -1}),
            json!({"rolloutPercent": 101}),
        ] {
            assert_eq!(rollout_percent(&invalid), None);
        }
        assert_eq!(rollout_percent(&json!({"rolloutPercent": 0})), Some(0));
        assert_eq!(rollout_percent(&json!({"rolloutPercent": 100})), Some(100));
        assert!(!eligible(0, 0));
        assert!(eligible(9, 10));
        assert!(!eligible(10, 10));
        assert!(eligible(99, 100));
        assert!(belongs_to_stable_channel("0.1.14", "0.1."));
        assert!(!belongs_to_stable_channel("0.2.0", "0.1."));
        assert!(belongs_to_stable_channel("0.2.0", "0.2."));
        assert!(!belongs_to_stable_channel("0.2.0-rc.1", "0.2."));
    }

    #[test]
    fn rollout_bucket_is_local_persistent_and_does_not_resample_corruption() {
        let home = tempfile::tempdir().unwrap();
        let first = local_bucket(home.path()).unwrap();
        assert!(first < BUCKET_COUNT);
        assert_eq!(local_bucket(home.path()).unwrap(), first);
        let path = home.path().join(BUCKET_FILE);
        assert_eq!(read_bucket(&path).unwrap(), first);
        fs::write(&path, "100").unwrap();
        assert_eq!(
            local_bucket(home.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_to_string(path).unwrap(), "100");
    }

    #[test]
    fn concurrent_checks_share_the_same_bucket() {
        let home = tempfile::tempdir().unwrap();
        let ready = std::sync::Barrier::new(6);
        let buckets = std::thread::scope(|scope| {
            let checks: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        ready.wait();
                        local_bucket(home.path()).unwrap()
                    })
                })
                .collect();
            checks
                .into_iter()
                .map(|check| check.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(buckets.iter().all(|bucket| *bucket == buckets[0]));
    }

    #[test]
    fn backup_copies_all_home_files_and_excludes_backups() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        for dir in [
            "profiles",
            "history/audio",
            "history/empty",
            "backup/an-older-copy",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (path, data) in [
            ("config.json", b"{\"hotkey\":\"F9\"}".as_slice()),
            ("profiles/dota2.json", b"profile"),
            ("history/audio/a.wav", b"\x00\x01\xFF"),
            ("reminders.json", b"[]"),
            ("profiles/.dota2-deleted", b""),
            (BUCKET_FILE, b"42"),
        ] {
            fs::write(root.join(path), data).unwrap();
        }
        let snapshot = backup_home(root, "0.1.14").unwrap();
        assert!(snapshot
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("0.1.14-"));
        for path in [
            "config.json",
            "profiles/dota2.json",
            "history/audio/a.wav",
            "reminders.json",
            "profiles/.dota2-deleted",
            BUCKET_FILE,
        ] {
            assert_eq!(
                fs::read(snapshot.join(path)).unwrap(),
                fs::read(root.join(path)).unwrap()
            );
        }
        assert!(snapshot.join("history/empty").is_dir());
        assert!(!snapshot.join("backup").exists());
        assert_eq!(fs::read_dir(root.join("backup")).unwrap().count(), 2);
    }

    #[test]
    fn failed_snapshot_does_not_publish_a_backup() {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("backup"), b"occupied").unwrap();
        assert!(backup_home(home.path(), "0.1.14").is_err());
        assert_eq!(fs::read(home.path().join("backup")).unwrap(), b"occupied");
        #[cfg(unix)]
        {
            fs::remove_file(home.path().join("backup")).unwrap();
            std::os::unix::fs::symlink("outside", home.path().join("unreadable")).unwrap();
            assert!(backup_home(home.path(), "0.1.14").is_err());
            assert_eq!(fs::read_dir(home.path().join("backup")).unwrap().count(), 0);
        }
    }
}
