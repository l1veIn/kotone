//! 游戏列表的图标：找到游戏的主程序（.exe），取它的图标（和资源管理器显示的一样），编码成 PNG。
//!
//! 主程序按这个顺序找：启动命令里带引号的 .exe → 安装目录（或下一层）里与进程名同名的 .exe →
//! 安装目录里第一个不像安装 / 卸载 / 崩溃上报程序的 .exe。

use std::path::{Path, PathBuf};

use crate::games::InstalledGame;

/// 不是游戏本体的程序（卸载、安装、崩溃上报、运行库）。
fn is_helper(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "unins", "setup", "install", "crash", "redist", "vcredist", "dxsetup", "report", "update",
        "helper",
    ]
    .iter()
    .any(|w| n.contains(w))
}

/// 启动命令里带引号的程序路径（`"C:\...\x.exe" --arg`）。
fn quoted_exe(launch: &str) -> Option<PathBuf> {
    let rest = launch.trim().strip_prefix('"')?;
    let end = rest.find('"')?;
    let path = PathBuf::from(&rest[..end]);
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        .then_some(path)
}

fn exes_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// 这个游戏的主程序：先找与进程名同名的（游戏本体），再看启动命令里的程序（可能是启动器），
/// 最后是安装目录里第一个不像辅助程序的 .exe。
pub fn main_exe(game: &InstalledGame) -> Option<PathBuf> {
    let dir = Path::new(&game.install_dir);
    let subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    for name in &game.process_names {
        let direct = dir.join(name);
        if direct.is_file() {
            return Some(direct);
        }
        if let Some(found) = subdirs.iter().map(|d| d.join(name)).find(|p| p.is_file()) {
            return Some(found);
        }
    }
    if let Some(path) = game.launch.as_deref().and_then(quoted_exe) {
        if path.is_file() {
            return Some(path);
        }
    }
    exes_in(dir).into_iter().find(|p| {
        !is_helper(
            &p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
        )
    })
}

/// Steam 自己缓存的游戏小图标：`<Steam>/appcache/librarycache/<appid>/<hash>.jpg`（新版，目录里最小的那张），
/// 或旧版的 `<appid>_icon.jpg`。
pub fn steam_cached_icon(steam_root: &Path, game: &InstalledGame) -> Option<Vec<u8>> {
    let appid = game.id.strip_prefix("steam:")?;
    let cache = steam_root.join("appcache").join("librarycache");
    let legacy = cache.join(format!("{appid}_icon.jpg"));
    if let Ok(bytes) = std::fs::read(&legacy) {
        return Some(bytes);
    }
    let smallest = std::fs::read_dir(cache.join(appid))
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("jpg"))
        })
        .filter_map(|e| e.metadata().ok().map(|m| (m.len(), e.path())))
        .min_by_key(|(len, _)| *len)?;
    std::fs::read(smallest.1).ok()
}

/// 游戏图标（PNG 或 JPEG 字节）：主程序的图标；没有就用 Steam 缓存的图标。
pub fn game_icon(game: &InstalledGame, size: u32) -> Option<Vec<u8>> {
    main_exe(game)
        .and_then(|exe| icon_png(&exe, size))
        .or_else(|| crate::games::steam_root().and_then(|root| steam_cached_icon(&root, game)))
}

/// 程序图标（`size`×`size`，RGBA）编码成 PNG。取不到返回 None。
pub fn icon_png(exe: &Path, size: u32) -> Option<Vec<u8>> {
    let (w, h, rgba) = icon_rgba(exe, size)?;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(out)
}

use image::ImageEncoder;

#[cfg(windows)]
fn icon_rgba(exe: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, PrivateExtractIconsW, HICON, ICONINFO,
    };

    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
    let mut path = [0u16; 260];
    if wide.len() > path.len() {
        return None;
    }
    path[..wide.len()].copy_from_slice(&wide);
    let mut icons = [HICON::default()];
    let got = unsafe {
        PrivateExtractIconsW(
            &path,
            0,
            size as i32,
            size as i32,
            Some(&mut icons),
            None,
            0,
        )
    };
    if got == 0 || icons[0].is_invalid() {
        return None;
    }
    let icon = icons[0];
    let result = unsafe {
        let mut info = ICONINFO::default();
        if GetIconInfo(icon, &mut info).is_err() {
            let _ = DestroyIcon(icon);
            return None;
        }
        let mut bitmap = BITMAP::default();
        let ok = GetObjectW(
            HGDIOBJ(info.hbmColor.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut _ as *mut _),
        ) != 0;
        let (w, h) = (bitmap.bmWidth.max(0) as u32, bitmap.bmHeight.max(0) as u32);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = if ok && w > 0 && h > 0 {
            GetDIBits(
                dc,
                info.hbmColor,
                0,
                h,
                Some(pixels.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        let _ = DeleteDC(dc);
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        let _ = DestroyIcon(icon);
        if lines == 0 {
            return None;
        }
        // BGRA → RGBA；老图标没有透明通道（全 0）时当作不透明。
        let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
        for p in pixels.chunks_exact_mut(4) {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            }
        }
        Some((w, h, pixels))
    };
    result
}

#[cfg(not(windows))]
fn icon_rgba(_exe: &Path, _size: u32) -> Option<(u32, u32, Vec<u8>)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(dir: &Path, launch: Option<&str>, process: &[&str]) -> InstalledGame {
        InstalledGame {
            id: "t".into(),
            name: "t".into(),
            source: "steam".into(),
            install_dir: dir.to_string_lossy().into_owned(),
            launch: launch.map(str::to_string),
            process_names: process.iter().map(|s| s.to_string()).collect(),
            needs_admin: false,
        }
    }

    #[test]
    fn finds_the_main_exe_and_skips_helpers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("unins000.exe"), b"x").unwrap();
        std::fs::write(dir.path().join("Game.exe"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin").join("Real.exe"), b"x").unwrap();
        assert_eq!(
            main_exe(&game(dir.path(), None, &[])).unwrap(),
            dir.path().join("Game.exe")
        );
        assert_eq!(
            main_exe(&game(dir.path(), None, &["Real.exe"])).unwrap(),
            dir.path().join("bin").join("Real.exe")
        );
        let quoted = format!(
            "\"{}\" -arg",
            dir.path().join("bin").join("Real.exe").display()
        );
        assert_eq!(
            main_exe(&game(dir.path(), Some(&quoted), &[])).unwrap(),
            dir.path().join("bin").join("Real.exe")
        );
        assert!(main_exe(&game(
            &dir.path().join("missing"),
            Some("steam://run/1"),
            &[]
        ))
        .is_none());
    }

    #[test]
    fn steam_icons_come_from_its_library_cache() {
        let root = tempfile::tempdir().unwrap();
        let cache = root
            .path()
            .join("appcache")
            .join("librarycache")
            .join("570");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("big.jpg"), vec![1u8; 500]).unwrap();
        std::fs::write(cache.join("icon.jpg"), vec![2u8; 20]).unwrap();
        let mut dota = game(root.path(), Some("steam://rungameid/570"), &[]);
        dota.id = "steam:570".into();
        assert_eq!(
            steam_cached_icon(root.path(), &dota).unwrap(),
            vec![2u8; 20]
        );
        dota.id = "epic:x".into();
        assert!(steam_cached_icon(root.path(), &dota).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn extracts_a_png_icon_from_a_windows_program() {
        let exe = std::path::PathBuf::from(std::env::var("WINDIR").unwrap()).join("notepad.exe");
        let png = icon_png(&exe, 48).expect("notepad has an icon");
        assert_eq!(&png[1..4], b"PNG");
        if let Ok(out) = std::env::var("KOTONE_ICON_OUT") {
            std::fs::write(out, &png).unwrap();
        }
    }
}
