//! 检测本机装了哪些游戏、装在哪、怎么启动（工具 `games.installed`）。
//!
//! 照成熟开源项目的做法（MIT：Playnite 库插件、steamlocate、LeagueAkari、Starward；GPL 的
//! SCSKiller 只参考思路），**只读注册表、平台自己的清单文件和少量固定目录**，不扫全盘：
//!
//! | 来源 | 读哪里 | 怎么启动 |
//! |---|---|---|
//! | Steam | 注册表 `Valve\Steam` → `steamapps\libraryfolders.vdf` → `appmanifest_*.acf` | `steam://rungameid/<id>` |
//! | 英雄联盟国服 | `HKCU\Software\Tencent\LOL` 的 `InstallPath` | WeGame 启动器 / TCLS `Client.exe`（会弹管理员授权） |
//! | Riot（外服 LoL、无畏契约） | `%ProgramData%\Riot Games\Metadata\*\*.product_settings.yaml`、`RiotClientInstalls.json` | `RiotClientServices.exe --launch-product=… --launch-patchline=…` |
//! | 米哈游（HoYoPlay） | `HKCU\Software\miHoYo\HYP\*\<biz>`、`Cognosphere\HYP\*\<biz>` 的 `GameInstallPath` | 游戏程序本身 |
//! | Epic | `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` | `com.epicgames.launcher://apps/…?action=launch` |
//! | WeGame | 各盘根目录 `WeGameApps\rail_apps\<名称>(<id>)`（有 `rail_game_identify.json` 和 `WeGameLauncher\launcher.exe`） | WeGame 启动器 |
//! | 兜底 | 系统“卸载程序”列表（只在按名字搜时用） | 程序图标指向的 exe |
//!
//! 有反作弊的游戏（Riot、腾讯）一律走平台的启动器，不直接运行游戏程序。注册表里的路径可能是卸载残留或
//! 拔掉的硬盘：目录不存在的一律丢弃。

use serde::Serialize;

/// 一个检测到的游戏。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGame {
    /// 稳定的 ID：`steam:730`、`tencent:lol`、`riot:valorant.live`、`hoyo:hk4e_cn`、`epic:<AppName>`……
    pub id: String,
    pub name: String,
    /// `steam` / `tencent` / `riot` / `hoyoplay` / `epic` / `wegame` / `uninstall`
    pub source: String,
    pub install_dir: String,
    /// 可以直接填进游戏配置“启动路径”的命令（带引号的程序路径 + 参数，或启动器链接）；None = 不知道怎么启动。
    pub launch: Option<String>,
    /// 游戏进程名（用来和游戏配置对上），不知道时为空。
    pub process_names: Vec<String>,
    /// 启动时 Windows 会要求管理员授权。
    pub needs_admin: bool,
}

// ---------- 纯解析（各平台都能测） ----------

/// Valve KeyValues（vdf / acf）里所有 `"key" "value"` 的值（不区分大小写；反斜杠转义还原）。
pub fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let tokens = quoted_tokens(line);
        if tokens.len() == 2 && tokens[0].eq_ignore_ascii_case(key) {
            out.push(tokens[1].clone());
        }
    }
    out
}

fn quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut token = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(next) = chars.next() {
                        token.push(next);
                    }
                }
                '"' => break,
                _ => token.push(c),
            }
        }
        tokens.push(token);
    }
    tokens
}

/// 一个 Steam 游戏清单（appmanifest_*.acf）：(appid, 名字, installdir)。没装完（StateFlags 没有 4）返回 None。
pub fn parse_acf(text: &str) -> Option<(String, String, String)> {
    let first = |key| vdf_values(text, key).into_iter().next();
    let flags: u32 = first("StateFlags")?.parse().ok()?;
    if flags & 4 == 0 {
        return None;
    }
    Some((first("appid")?, first("name")?, first("installdir")?))
}

/// Steam 自带的运行库、工具，不算游戏。
pub fn steam_is_tool(appid: &str, name: &str) -> bool {
    const TOOLS: &[&str] = &["228980", "1070560", "1391110", "1628350", "1493710"];
    let lower = name.to_ascii_lowercase();
    TOOLS.contains(&appid)
        || lower.contains("redistributable")
        || lower.starts_with("proton")
        || lower.contains("steamworks")
        || lower.contains("steam linux runtime")
}

/// YAML 里一行 `key: "value"` 的值（Riot 的 product_settings 只需要这一种）。
pub fn yaml_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line
            .trim()
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix(':')?;
        let value = rest.trim().trim_matches('"').trim_matches('\'').to_string();
        (!value.is_empty()).then_some(value)
    })
}

/// Riot 产品 ID → (显示名, 进程名)。
pub fn riot_product(product: &str) -> (String, Vec<String>) {
    match product {
        "league_of_legends" => (
            "英雄联盟（外服）".into(),
            vec!["League of Legends.exe".into()],
        ),
        "valorant" => (
            "无畏契约（VALORANT）".into(),
            vec!["VALORANT-Win64-Shipping.exe".into()],
        ),
        "bacon" => ("符文之地传说".into(), vec!["LoR.exe".into()]),
        other => (other.replace('_', " "), Vec::new()),
    }
}

/// 米哈游 biz（如 `hk4e_cn`）→ (显示名, 程序名)。
pub fn hoyo_game(biz: &str) -> Option<(&'static str, &'static str)> {
    let (game, region) = biz.split_once('_')?;
    Some(match (game, region) {
        ("hk4e", "cn") => ("原神", "YuanShen.exe"),
        ("hk4e", _) => ("原神（国际服）", "GenshinImpact.exe"),
        ("hkrpg", "cn") => ("崩坏：星穹铁道", "StarRail.exe"),
        ("hkrpg", _) => ("崩坏：星穹铁道（国际服）", "StarRail.exe"),
        ("nap", "cn") => ("绝区零", "ZenlessZoneZero.exe"),
        ("nap", _) => ("绝区零（国际服）", "ZenlessZoneZero.exe"),
        ("bh3", "cn") => ("崩坏3", "BH3.exe"),
        ("bh3", _) => ("崩坏3（国际服）", "BH3.exe"),
        _ => return None,
    })
}

/// Epic 清单（.item，JSON）→ 游戏。没装完或不是游戏的返回 None；目录是否存在由调用方检查。
pub fn parse_epic_item(json: &serde_json::Value) -> Option<InstalledGame> {
    let s = |k: &str| json.get(k).and_then(|v| v.as_str()).unwrap_or("").trim();
    if json.get("bIsIncompleteInstall").and_then(|v| v.as_bool()) == Some(true) {
        return None;
    }
    let is_game = json
        .get("AppCategories")
        .and_then(|v| v.as_array())
        .map_or(true, |c| c.iter().any(|x| x.as_str() == Some("games")));
    let (name, dir, app) = (s("DisplayName"), s("InstallLocation"), s("AppName"));
    if !is_game || name.is_empty() || dir.is_empty() || app.is_empty() {
        return None;
    }
    let (ns, item) = (s("CatalogNamespace"), s("CatalogItemId"));
    let launch = (!ns.is_empty() && !item.is_empty()).then(|| {
        format!("com.epicgames.launcher://apps/{ns}%3A{item}%3A{app}?action=launch&silent=true")
    });
    let exe = s("LaunchExecutable");
    let process = exe
        .rsplit(['/', '\\'])
        .next()
        .filter(|e| e.to_ascii_lowercase().ends_with(".exe"))
        .map(str::to_string);
    Some(InstalledGame {
        id: format!("epic:{app}"),
        name: name.into(),
        source: "epic".into(),
        install_dir: dir.replace('/', "\\"),
        launch,
        process_names: process.into_iter().collect(),
        needs_admin: false,
    })
}

/// WeGame 游戏目录名 `英雄联盟(26)` → `英雄联盟`。
pub fn wegame_name(folder: &str) -> String {
    match folder.rfind('(') {
        Some(i) if folder.ends_with(')') && i > 0 => folder[..i].trim().to_string(),
        _ => folder.trim().to_string(),
    }
}

/// 带引号的程序路径 + 参数。
pub fn command(exe: &str, args: &[&str]) -> String {
    let mut out = format!("\"{exe}\"");
    for a in args {
        out.push(' ');
        out.push_str(a);
    }
    out
}

/// 把启动路径拆成（程序 / 链接，参数）。以引号开头时引号里是程序，后面是参数；否则整串是程序或链接
/// （兼容用户手填的、没加引号的带空格路径）。
pub fn split_command(launch: &str) -> (String, Option<String>) {
    let launch = launch.trim();
    if let Some(rest) = launch.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let args = rest[end + 1..].trim();
            return (
                rest[..end].to_string(),
                (!args.is_empty()).then(|| args.to_string()),
            );
        }
    }
    (launch.to_string(), None)
}

/// 用名字 / 进程名找检测到的游戏（给“打开游戏”用）：先比进程名，再比名字互相包含。
pub fn find_match<'a>(
    games: &'a [InstalledGame],
    name: &str,
    process_names: &[String],
) -> Option<&'a InstalledGame> {
    let launchable = || games.iter().filter(|g| g.launch.is_some());
    if let Some(g) = launchable().find(|g| {
        g.process_names
            .iter()
            .any(|p| process_names.iter().any(|q| q.eq_ignore_ascii_case(p)))
    }) {
        return Some(g);
    }
    let key = normalize(name);
    if key.is_empty() {
        return None;
    }
    launchable().find(|g| {
        let n = normalize(&g.name);
        !n.is_empty() && (n.contains(&key) || key.contains(&n))
    })
}

fn normalize(name: &str) -> String {
    let cut = name.split(['（', '(']).next().unwrap_or(name);
    cut.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

// ---------- 读本机（Windows） ----------

#[cfg(windows)]
mod reg {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, KEY_READ,
        KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    };

    pub use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open(root: HKEY, path: &str, view: REG_SAM_FLAGS) -> Option<Key> {
        let mut key = HKEY::default();
        let path = wide(path);
        let ok =
            unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), None, KEY_READ | view, &mut key) };
        (ok.0 == 0).then_some(Key(key))
    }

    fn views() -> [REG_SAM_FLAGS; 2] {
        [KEY_WOW64_64KEY, KEY_WOW64_32KEY]
    }

    fn query(key: &Key, value: &str) -> Option<String> {
        let name = wide(value);
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        unsafe {
            if RegQueryValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
            .0 != 0
            {
                return None;
            }
            if kind != REG_SZ && kind != REG_EXPAND_SZ {
                return None;
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
            let mut bytes = (buf.len() * 2) as u32;
            if RegQueryValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
            .0 != 0
            {
                return None;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..len]).trim().to_string();
            (!s.is_empty()).then_some(s)
        }
    }

    /// 读字符串值（64 位、32 位两个视图都试）。
    pub fn string(root: HKEY, path: &str, value: &str) -> Option<String> {
        views()
            .into_iter()
            .find_map(|view| open(root, path, view).and_then(|key| query(&key, value)))
    }

    /// 子键名（两个视图合并去重）。
    pub fn subkeys(root: HKEY, path: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for view in views() {
            let Some(key) = open(root, path, view) else {
                continue;
            };
            for index in 0.. {
                let mut buf = [0u16; 256];
                let mut len = buf.len() as u32;
                let ok = unsafe {
                    RegEnumKeyExW(
                        key.0,
                        index,
                        Some(PWSTR(buf.as_mut_ptr())),
                        &mut len,
                        None,
                        None,
                        None,
                        None,
                    )
                };
                if ok.0 != 0 {
                    break;
                }
                let name = String::from_utf16_lossy(&buf[..len as usize]);
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }
}

#[cfg(windows)]
mod scan {
    use super::*;
    use std::path::{Path, PathBuf};

    fn dir_ok(path: &Path) -> bool {
        path.is_dir()
    }

    fn display(path: &Path) -> String {
        path.to_string_lossy().replace('/', "\\")
    }

    fn program_data() -> PathBuf {
        PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into()))
    }

    pub fn steam(out: &mut Vec<InstalledGame>) {
        let root = reg::string(
            reg::HKEY_CURRENT_USER,
            "Software\\Valve\\Steam",
            "SteamPath",
        )
        .or_else(|| {
            reg::string(
                reg::HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Valve\\Steam",
                "InstallPath",
            )
        });
        let Some(root) = root.map(|r| PathBuf::from(r.replace('/', "\\"))) else {
            return;
        };
        let mut libraries = vec![root.clone()];
        if let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf"))
        {
            for path in vdf_values(&text, "path") {
                let path = PathBuf::from(path.replace('/', "\\"));
                if !libraries.iter().any(|l| {
                    l.to_string_lossy()
                        .eq_ignore_ascii_case(&path.to_string_lossy())
                }) {
                    libraries.push(path);
                }
            }
        }
        for library in libraries {
            let apps = library.join("steamapps");
            let Ok(entries) = std::fs::read_dir(&apps) else {
                continue;
            };
            for entry in entries.flatten() {
                let file = entry.file_name().to_string_lossy().to_string();
                if !file.starts_with("appmanifest_") || !file.ends_with(".acf") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let Some((appid, name, installdir)) = parse_acf(&text) else {
                    continue;
                };
                let dir = apps.join("common").join(&installdir);
                if steam_is_tool(&appid, &name) || !dir_ok(&dir) {
                    continue;
                }
                out.push(InstalledGame {
                    id: format!("steam:{appid}"),
                    name,
                    source: "steam".into(),
                    install_dir: display(&dir),
                    launch: Some(format!("steam://rungameid/{appid}")),
                    process_names: Vec::new(),
                    needs_admin: false,
                });
            }
        }
    }

    pub fn tencent_lol(out: &mut Vec<InstalledGame>) {
        let path = reg::string(
            reg::HKEY_CURRENT_USER,
            "Software\\Tencent\\LOL",
            "InstallPath",
        )
        .or_else(|| {
            reg::string(
                reg::HKEY_LOCAL_MACHINE,
                "SOFTWARE\\Tencent\\LOL",
                "InstallPath",
            )
        });
        let Some(dir) = path.map(PathBuf::from).filter(|d| dir_ok(d)) else {
            return;
        };
        let launcher = [
            dir.join("WeGameLauncher").join("launcher.exe"),
            dir.join("Launcher").join("Client.exe"),
            dir.join("TCLS").join("Client.exe"),
        ]
        .into_iter()
        .find(|p| p.is_file());
        out.push(InstalledGame {
            id: "tencent:lol".into(),
            name: "英雄联盟（国服）".into(),
            source: "tencent".into(),
            install_dir: display(&dir),
            launch: launcher.map(|p| command(&display(&p), &[])),
            process_names: vec!["League of Legends.exe".into()],
            needs_admin: true,
        });
    }

    pub fn riot(out: &mut Vec<InstalledGame>) {
        let base = program_data().join("Riot Games");
        let installs: serde_json::Value =
            std::fs::read_to_string(base.join("RiotClientInstalls.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
        let norm = |p: &str| {
            p.replace('\\', "/")
                .trim_end_matches('/')
                .to_ascii_lowercase()
        };
        let Ok(entries) = std::fs::read_dir(base.join("Metadata")) else {
            return;
        };
        for entry in entries.flatten() {
            let folder = entry.file_name().to_string_lossy().to_string();
            let Some((product, patchline)) = folder.split_once('.') else {
                continue;
            };
            if product.eq_ignore_ascii_case("riot_client") || folder.starts_with("Riot Client") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(
                entry.path().join(format!("{folder}.product_settings.yaml")),
            ) else {
                continue;
            };
            let Some(path) = yaml_value(&text, "product_install_full_path") else {
                continue;
            };
            // 腾讯代理的国服也可能登记在这里，由 tencent_lol 负责
            if path.contains("英雄联盟") || path.contains("WeGameApps") {
                continue;
            }
            let dir = PathBuf::from(path.replace('/', "\\"));
            if !dir_ok(&dir) {
                continue;
            }
            let client = installs
                .get("associated_client")
                .and_then(|m| m.as_object())
                .and_then(|m| {
                    m.iter()
                        .find(|(k, _)| norm(k) == norm(&path))
                        .map(|(_, v)| v)
                })
                .or_else(|| installs.get("rc_default"))
                .or_else(|| installs.get("rc_live"))
                .and_then(|v| v.as_str())
                .map(|s| s.replace('/', "\\"))
                .filter(|s| Path::new(s).is_file());
            let (name, process_names) = riot_product(product);
            let product_arg = format!("--launch-product={product}");
            let patch_arg = format!("--launch-patchline={patchline}");
            out.push(InstalledGame {
                id: format!("riot:{folder}"),
                name,
                source: "riot".into(),
                install_dir: display(&dir),
                launch: client.map(|c| command(&c, &[product_arg.as_str(), patch_arg.as_str()])),
                process_names,
                needs_admin: false,
            });
        }
    }

    pub fn hoyoplay(out: &mut Vec<InstalledGame>) {
        for root in ["Software\\miHoYo\\HYP", "Software\\Cognosphere\\HYP"] {
            for version in reg::subkeys(reg::HKEY_CURRENT_USER, root) {
                let parent = format!("{root}\\{version}");
                for biz in reg::subkeys(reg::HKEY_CURRENT_USER, &parent) {
                    let Some((name, exe)) = hoyo_game(&biz) else {
                        continue;
                    };
                    let Some(dir) = reg::string(
                        reg::HKEY_CURRENT_USER,
                        &format!("{parent}\\{biz}"),
                        "GameInstallPath",
                    )
                    .map(|p| PathBuf::from(p.replace('/', "\\")))
                    .filter(|d| dir_ok(d)) else {
                        continue;
                    };
                    let program = dir.join(exe);
                    if out.iter().any(|g| g.id == format!("hoyo:{biz}")) {
                        continue;
                    }
                    out.push(InstalledGame {
                        id: format!("hoyo:{biz}"),
                        name: name.into(),
                        source: "hoyoplay".into(),
                        install_dir: display(&dir),
                        launch: program.is_file().then(|| command(&display(&program), &[])),
                        process_names: vec![exe.into()],
                        needs_admin: true,
                    });
                }
            }
        }
    }

    pub fn epic(out: &mut Vec<InstalledGame>) {
        let dir = program_data()
            .join("Epic")
            .join("EpicGamesLauncher")
            .join("Data")
            .join("Manifests");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("item") {
                continue;
            }
            let Some(json) = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            else {
                continue;
            };
            if let Some(game) =
                parse_epic_item(&json).filter(|g| Path::new(&g.install_dir).is_dir())
            {
                out.push(game);
            }
        }
    }

    pub fn wegame(out: &mut Vec<InstalledGame>) {
        for letter in 'C'..='Z' {
            let apps = PathBuf::from(format!("{letter}:\\WeGameApps\\rail_apps"));
            let Ok(entries) = std::fs::read_dir(&apps) else {
                continue;
            };
            for entry in entries.flatten() {
                let dir = entry.path();
                let launcher = dir.join("WeGameLauncher").join("launcher.exe");
                if !dir
                    .join("rail_files")
                    .join("rail_game_identify.json")
                    .is_file()
                    || !launcher.is_file()
                {
                    continue;
                }
                let folder = entry.file_name().to_string_lossy().to_string();
                out.push(InstalledGame {
                    id: format!("wegame:{folder}"),
                    name: wegame_name(&folder),
                    source: "wegame".into(),
                    install_dir: display(&dir),
                    launch: Some(command(&display(&launcher), &[])),
                    process_names: Vec::new(),
                    needs_admin: true,
                });
            }
        }
    }

    /// 系统“卸载程序”列表里名字包含 query 的（不区分大小写），最多 20 个。
    pub fn uninstall(query: &str, out: &mut Vec<InstalledGame>) {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return;
        }
        const PATH: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
        for root in [reg::HKEY_LOCAL_MACHINE, reg::HKEY_CURRENT_USER] {
            for sub in reg::subkeys(root, PATH) {
                if out.len() >= 20 {
                    return;
                }
                let key = format!("{PATH}\\{sub}");
                let Some(name) = reg::string(root, &key, "DisplayName") else {
                    continue;
                };
                if !name.to_lowercase().contains(&query) {
                    continue;
                }
                let dir = reg::string(root, &key, "InstallLocation").unwrap_or_default();
                let exe = reg::string(root, &key, "DisplayIcon")
                    .map(|icon| {
                        icon.split(',')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .trim_matches('"')
                            .to_string()
                    })
                    .filter(|p| {
                        let lower = p.to_lowercase();
                        lower.ends_with(".exe")
                            && !lower.contains("unins")
                            && Path::new(p).is_file()
                    });
                if dir.is_empty() && exe.is_none() {
                    continue;
                }
                let install_dir = if dir.is_empty() {
                    exe.as_deref()
                        .and_then(|e| Path::new(e).parent())
                        .map(display)
                        .unwrap_or_default()
                } else {
                    dir.trim_matches('"').to_string()
                };
                let id = format!("uninstall:{sub}");
                if out.iter().any(|g| g.id == id) {
                    continue;
                }
                out.push(InstalledGame {
                    id,
                    name,
                    source: "uninstall".into(),
                    install_dir,
                    process_names: exe
                        .as_deref()
                        .and_then(|e| Path::new(e).file_name())
                        .map(|f| f.to_string_lossy().to_string())
                        .into_iter()
                        .collect(),
                    launch: exe.map(|e| command(&e, &[])),
                    needs_admin: false,
                });
            }
        }
    }
}

/// 检测已安装的游戏（Steam、英雄联盟国服、Riot、米哈游、Epic、WeGame）。每个来源互不影响；
/// 只读注册表和清单文件，通常 0.1 秒内完成。`query` 非空时再从系统“卸载程序”列表按名字搜。
#[cfg(windows)]
pub fn detect(query: Option<&str>) -> Vec<InstalledGame> {
    let mut out = Vec::new();
    scan::tencent_lol(&mut out);
    scan::riot(&mut out);
    scan::hoyoplay(&mut out);
    scan::steam(&mut out);
    scan::epic(&mut out);
    scan::wegame(&mut out);
    // 同一个目录只留第一个（国服 LoL 也会出现在 WeGameApps 里）
    let mut seen: Vec<String> = Vec::new();
    out.retain(|g| {
        let key = g.install_dir.trim_end_matches('\\').to_lowercase();
        let fresh = !seen.contains(&key);
        seen.push(key);
        fresh
    });
    if let Some(query) = query.map(str::trim).filter(|q| !q.is_empty()) {
        let lower = query.to_lowercase();
        out.retain(|g| g.name.to_lowercase().contains(&lower));
        let mut extra = Vec::new();
        scan::uninstall(query, &mut extra);
        for game in extra {
            let key = game.install_dir.trim_end_matches('\\').to_lowercase();
            if !out
                .iter()
                .any(|g| g.install_dir.trim_end_matches('\\').to_lowercase() == key)
            {
                out.push(game);
            }
        }
    }
    out
}

#[cfg(not(windows))]
pub fn detect(_query: Option<&str>) -> Vec<InstalledGame> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 诊断：打印这台电脑检测到的游戏（`cargo test -p kotone-platform-windows games::tests::this_machine -- --ignored --nocapture`）。
    #[test]
    #[ignore]
    fn this_machine() {
        let start = std::time::Instant::now();
        let games = detect(None);
        for g in &games {
            println!(
                "{} | {} | {} | {:?} | admin={}",
                g.id, g.name, g.install_dir, g.launch, g.needs_admin
            );
        }
        println!("{} games in {:?}", games.len(), start.elapsed());
        for q in ["网易", "永劫"] {
            for g in detect(Some(q)) {
                println!("query {q}: {} | {} | {:?}", g.id, g.name, g.launch);
            }
        }
    }

    /// 诊断：带引号的程序 + 参数真的按参数传下去、在程序所在目录运行（用 cmd.exe 写一个临时文件）。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn launch_passes_arguments() {
        let dir = std::env::temp_dir().join(format!("kotone-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("ok.txt");
        let cmd = command(
            r"C:\Windows\System32\cmd.exe",
            &["/c", &format!("cd > \"{}\"", out.display())],
        );
        crate::game_window::launch(&cmd).unwrap();
        for _ in 0..50 {
            if out.is_file() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let cwd = std::fs::read_to_string(&out).unwrap();
        assert!(
            cwd.trim().eq_ignore_ascii_case(r"C:\Windows\System32"),
            "{cwd}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_steam_library_folders_and_manifests() {
        let folders = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            vdf_values(folders, "path"),
            ["C:\\Program Files (x86)\\Steam", "D:\\SteamLibrary"]
        );
        let acf = r#""AppState"
{
	"appid"		"570"
	"name"		"Dota 2"
	"StateFlags"		"4"
	"installdir"		"dota 2 beta"
}"#;
        assert_eq!(
            parse_acf(acf),
            Some(("570".into(), "Dota 2".into(), "dota 2 beta".into()))
        );
        assert_eq!(
            parse_acf(&acf.replace("\"4\"", "\"1026\"")),
            None,
            "下载中没装完"
        );
        assert!(steam_is_tool(
            "228980",
            "Steamworks Common Redistributables"
        ));
        assert!(!steam_is_tool("570", "Dota 2"));
    }

    #[test]
    fn reads_riot_settings_and_names() {
        let yaml = "product_install_full_path: \"C:/Riot Games/VALORANT/live\"\nproduct_install_root: \"C:/Riot Games\"\n";
        assert_eq!(
            yaml_value(yaml, "product_install_full_path").as_deref(),
            Some("C:/Riot Games/VALORANT/live")
        );
        assert_eq!(riot_product("valorant").1, ["VALORANT-Win64-Shipping.exe"]);
        assert_eq!(hoyo_game("hk4e_cn"), Some(("原神", "YuanShen.exe")));
        assert_eq!(hoyo_game("hkrpg_global").map(|g| g.1), Some("StarRail.exe"));
        assert_eq!(hoyo_game("unknown"), None);
        assert_eq!(wegame_name("英雄联盟(26)"), "英雄联盟");
    }

    #[test]
    fn reads_epic_manifests() {
        let item = json!({
            "DisplayName": "Fortnite", "InstallLocation": "C:/Program Files/Epic Games/Fortnite",
            "LaunchExecutable": "FortniteGame/Binaries/Win64/FortniteLauncher.exe",
            "CatalogNamespace": "fn", "CatalogItemId": "4fe7", "AppName": "Fortnite",
            "AppCategories": ["public", "games", "applications"], "bIsIncompleteInstall": false
        });
        let game = parse_epic_item(&item).unwrap();
        assert_eq!(game.id, "epic:Fortnite");
        assert_eq!(
            game.launch.as_deref(),
            Some("com.epicgames.launcher://apps/fn%3A4fe7%3AFortnite?action=launch&silent=true")
        );
        assert_eq!(game.process_names, ["FortniteLauncher.exe"]);
        assert_eq!(game.install_dir, "C:\\Program Files\\Epic Games\\Fortnite");
        let mut plugin = item.clone();
        plugin["AppCategories"] = json!(["plugins"]);
        assert!(parse_epic_item(&plugin).is_none());
    }

    #[test]
    fn commands_round_trip() {
        let c = command(
            "C:\\Riot Games\\Riot Client\\RiotClientServices.exe",
            &["--launch-product=valorant", "--launch-patchline=live"],
        );
        assert_eq!(
            split_command(&c),
            (
                "C:\\Riot Games\\Riot Client\\RiotClientServices.exe".into(),
                Some("--launch-product=valorant --launch-patchline=live".into())
            )
        );
        assert_eq!(
            split_command("steam://rungameid/570"),
            ("steam://rungameid/570".into(), None)
        );
        // 用户手填、没加引号的带空格路径：整串当程序
        assert_eq!(
            split_command("D:\\Games\\My Game\\game.exe"),
            ("D:\\Games\\My Game\\game.exe".into(), None)
        );
    }

    #[test]
    fn matches_profiles_by_process_then_name() {
        let game = |id: &str, name: &str, process: &str| InstalledGame {
            id: id.into(),
            name: name.into(),
            source: "x".into(),
            install_dir: format!("C:\\{id}"),
            launch: Some("x".into()),
            process_names: vec![process.into()],
            needs_admin: false,
        };
        let games = [
            game("tencent:lol", "英雄联盟（国服）", "League of Legends.exe"),
            game("hoyo:hk4e_cn", "原神", "YuanShen.exe"),
        ];
        let by_process = find_match(&games, "LoL", &["league of legends.exe".into()]);
        assert_eq!(by_process.unwrap().id, "tencent:lol");
        assert_eq!(find_match(&games, "原神", &[]).unwrap().id, "hoyo:hk4e_cn");
        assert_eq!(
            find_match(&games, "英雄联盟", &[]).unwrap().id,
            "tencent:lol"
        );
        assert!(find_match(&games, "Dota 2", &[]).is_none());
    }
}
