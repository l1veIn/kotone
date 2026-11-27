//! 找 BetterGI（ADR 047 第 3 期）：插件设置里的路径；没设时只看几个固定的浅层位置，不全盘递归。
//!
//! 顺序：
//! 1. 插件设置 `plugins.genshin.settings.path`（BetterGI 的目录或 `BetterGI.exe`）；
//! 2. 环境变量 `KOTONE_BETTERGI_DIR`（测试和真机脚本指向 `target/` 下的假安装；设了就只看它）；
//! 3. 各盘（C–Z）根目录下的 `BetterGI`、`Program Files\BetterGI`、`Program Files (x86)\BetterGI`、`Games\BetterGI`，
//!    `%LOCALAPPDATA%\Programs\BetterGI`；
//! 4. 开始菜单（当前用户和所有用户）里名字带 BetterGI 的快捷方式（最多往下两层），读快捷方式指向的程序（[`crate::lnk`]）。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::bgi::Install;

pub const ENV_DIR: &str = "KOTONE_BETTERGI_DIR";
pub const EXE: &str = "BetterGI.exe";

/// 从哪找到的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Found {
    /// 插件设置里写的
    Settings,
    /// 环境变量（测试）
    Env,
    /// 常见位置
    Common,
    /// 开始菜单的快捷方式
    Shortcut,
}

/// 一个路径（目录或 exe）→ 安装；`BetterGI.exe` 不在就不算。
pub fn from_path(path: &Path) -> Option<Install> {
    let is_exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let (dir, exe) = if is_exe {
        (path.parent()?.to_path_buf(), path.to_path_buf())
    } else {
        (path.to_path_buf(), path.join(EXE))
    };
    exe.is_file().then_some(Install { dir, exe })
}

/// 常见位置（不含快捷方式）。
fn common_places() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if !root.is_dir() {
            continue;
        }
        for sub in [
            "BetterGI",
            "Program Files\\BetterGI",
            "Program Files (x86)\\BetterGI",
            "Games\\BetterGI",
        ] {
            out.push(root.join(sub));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("Programs").join("BetterGI"));
    }
    out
}

/// 开始菜单里名字带 BetterGI 的快捷方式（最多往下两层）。
fn shortcuts() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Some(data) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(data).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    fn walk(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for path in rd.flatten().map(|e| e.path()).take(500) {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if path.is_dir() {
                if depth < 2 {
                    walk(&path, depth + 1, out);
                }
            } else if name.ends_with(".lnk") && name.contains("bettergi") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for root in &roots {
        walk(root, 0, &mut out);
    }
    out
}

/// 找到的安装和从哪找到的；找不到时给出看过哪些地方（给页面说明）。
pub fn locate(setting: &str) -> Result<(Install, Found), Vec<String>> {
    let setting = setting.trim().trim_matches('"');
    if !setting.is_empty() {
        let path = PathBuf::from(setting);
        return from_path(&path)
            .map(|i| (i, Found::Settings))
            .ok_or_else(|| vec![path.display().to_string()]);
    }
    if let Some(dir) = std::env::var_os(ENV_DIR).filter(|d| !d.is_empty()) {
        let path = PathBuf::from(dir);
        return from_path(&path)
            .map(|i| (i, Found::Env))
            .ok_or_else(|| vec![path.display().to_string()]);
    }
    let places = common_places();
    for place in &places {
        if let Some(install) = from_path(place) {
            return Ok((install, Found::Common));
        }
    }
    let links = shortcuts();
    for link in &links {
        let target = std::fs::read(link)
            .ok()
            .and_then(|bytes| crate::lnk::target(&bytes));
        if let Some(install) = target.as_deref().map(Path::new).and_then(from_path) {
            return Ok((install, Found::Shortcut));
        }
    }
    let mut searched: Vec<String> = places.iter().map(|p| p.display().to_string()).collect();
    searched.push("开始菜单里的 BetterGI 快捷方式".to_string());
    Err(searched)
}

/// `BetterGI.exe` 的文件版本（`0.67.0`；末尾的 `.0` 去掉）。读不到时 None。
#[cfg(windows)]
pub fn version(exe: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };
    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: 缓冲区按 GetFileVersionInfoSizeW 给的大小分配；VerQueryValueW 返回的指针指向这块缓冲区里面。
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, buf.as_mut_ptr().cast()).ok()?;
        let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(buf.as_ptr().cast(), w!("\\"), &mut info, &mut len).as_bool()
            || info.is_null()
            || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        let fixed = &*(info as *const VS_FIXEDFILEINFO);
        let parts = [
            fixed.dwFileVersionMS >> 16,
            fixed.dwFileVersionMS & 0xffff,
            fixed.dwFileVersionLS >> 16,
            fixed.dwFileVersionLS & 0xffff,
        ];
        Some(format_version(parts))
    }
}

#[cfg(not(windows))]
pub fn version(_exe: &Path) -> Option<String> {
    None
}

fn format_version(parts: [u32; 4]) -> String {
    let keep = if parts[3] == 0 { 3 } else { 4 };
    parts[..keep]
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// 版本够不够新（`0.67.0` ≥ `0.67`）。读不出版本时按不够算（None）。
pub fn at_least(version: Option<&str>, major: u32, minor: u32) -> Option<bool> {
    let mut parts = version?.split('.').map(|p| p.parse::<u32>().ok());
    let a = parts.next()??;
    let b = parts.next().flatten().unwrap_or(0);
    Some((a, b) >= (major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_or_the_exe_is_an_install_only_with_bettergi_exe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(from_path(dir.path()).is_none());
        std::fs::write(dir.path().join(EXE), b"MZ").unwrap();
        let by_dir = from_path(dir.path()).unwrap();
        assert_eq!(by_dir.exe, dir.path().join(EXE));
        let by_exe = from_path(&dir.path().join(EXE)).unwrap();
        assert_eq!(by_exe, by_dir);
        assert!(from_path(&dir.path().join("Other.exe")).is_none());
    }

    #[test]
    fn the_setting_wins_and_a_wrong_setting_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(EXE), b"MZ").unwrap();
        let setting = format!("\"{}\"", dir.path().display());
        let (install, found) = locate(&setting).unwrap();
        assert_eq!(
            (install.dir.as_path(), found),
            (dir.path(), Found::Settings)
        );
        let missing = dir.path().join("none");
        let searched = locate(&missing.display().to_string()).unwrap_err();
        assert_eq!(searched, [missing.display().to_string()]);
    }

    #[test]
    fn versions_compare_by_major_and_minor() {
        assert_eq!(format_version([0, 67, 0, 0]), "0.67.0");
        assert_eq!(format_version([0, 67, 1, 3]), "0.67.1.3");
        assert_eq!(at_least(Some("0.67.0"), 0, 67), Some(true));
        assert_eq!(at_least(Some("0.66.9"), 0, 67), Some(false));
        assert_eq!(at_least(Some("1.0"), 0, 67), Some(true));
        assert_eq!(at_least(None, 0, 67), None);
        assert_eq!(at_least(Some("x"), 0, 67), None);
    }

    /// 读的是真的文件版本：拿系统自带的程序试（不碰玩家的 BetterGI）。
    #[cfg(windows)]
    #[test]
    fn version_reads_the_file_version_resource() {
        let cmd = PathBuf::from(std::env::var("SystemRoot").unwrap_or("C:\\Windows".into()))
            .join("System32")
            .join("cmd.exe");
        let v = version(&cmd).unwrap();
        assert!(v.split('.').count() >= 3, "{v}");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(EXE), b"MZ").unwrap();
        assert!(version(&dir.path().join(EXE)).is_none());
    }
}
