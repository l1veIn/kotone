//! 游戏目录里的 `gamestate_integration_kotone.cfg`（ADR 036 D9：默认就写，插件页能关，关了只删我们写的那个）。
//!
//! - 在哪：`KOTONE_CS2_DIR`（CS2 的安装目录，测试和真机脚本用，指向 `target/` 下的假目录）；没设时，隔离的实例
//!   （设了 `KOTONE_HOME`：自动化测试、Agent 沙箱）**不找真的游戏**；都没设时照 `games.installed` 的 Steam 那一路
//!   （注册表 `Valve\Steam` → `libraryfolders.vdf` → `appmanifest_730.acf`）找 appid 730 的安装目录。
//!   cfg 目录是 `<安装目录>/game/csgo/cfg`。国服（完美世界）的 CS2 也是 Steam 上的 730，只是启动参数不同——**没核实**。
//! - 只在 CS2 没开时写、删（开着时等它关了再做；游戏只在启动时读 cfg）。
//! - 文件第一行是 [`MARKER`]：有它才算我们写的。同名但不是我们写的文件：要写时先备份到插件目录 `backups/` 再写，
//!   要删时不碰。
//! - 写的是纯 ASCII（注释也是英文）：游戏读 cfg 的解析器不保证认得 UTF-8 的注释。

use std::path::{Path, PathBuf};

/// 文件名（游戏认 `gamestate_integration_*.cfg`）。
pub const FILE: &str = "gamestate_integration_kotone.cfg";
/// 第一行：我们写的标记。
pub const MARKER: &str = "// Written by Kotone (CS2 plugin).";
/// 测试和真机脚本用：CS2 的安装目录（里面有 `game/csgo/cfg`）。
pub const DIR_ENV: &str = "KOTONE_CS2_DIR";
/// 隔离的数据目录（`kotone_core::settings::HOME_ENV`）。
const HOME_ENV: &str = "KOTONE_HOME";
/// CS2 的 Steam appid。
const STEAM_ID: &str = "steam:730";

/// cfg 目录在哪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Where {
    /// 找到了：cfg 目录（可能还不存在，[`apply`] 会说）。
    Dir(PathBuf),
    /// 没找到装了 CS2。
    NotInstalled,
    /// 隔离的实例：不找真的游戏。
    Isolated,
}

/// `<安装目录>/game/csgo/cfg`。
pub fn cfg_dir_in(install: &Path) -> PathBuf {
    install.join("game").join("csgo").join("cfg")
}

/// 按环境变量找（纯逻辑，测试直接调）：`dir_env` = `KOTONE_CS2_DIR`，`home_env` = `KOTONE_HOME`，`steam` 查 Steam 库。
pub fn locate_with(
    dir_env: Option<String>,
    home_env: Option<String>,
    steam: impl FnOnce() -> Option<PathBuf>,
) -> Where {
    if let Some(dir) = dir_env.filter(|d| !d.trim().is_empty()) {
        return Where::Dir(cfg_dir_in(Path::new(dir.trim())));
    }
    if home_env.is_some_and(|h| !h.trim().is_empty()) {
        return Where::Isolated;
    }
    match steam() {
        Some(install) => Where::Dir(cfg_dir_in(&install)),
        None => Where::NotInstalled,
    }
}

/// 照这台电脑找。
pub fn locate() -> Where {
    locate_with(
        std::env::var(DIR_ENV).ok(),
        std::env::var(HOME_ENV).ok(),
        || {
            kotone_platform_windows::games::steam_games()
                .into_iter()
                .find(|g| g.id == STEAM_ID)
                .map(|g| PathBuf::from(g.install_dir))
        },
    )
}

/// 文件的内容。照 Valve 文档的写法：`uri` 是本机的端口，`auth.token` 是随机令牌（游戏每次 POST 都带上），
/// 心跳 10 秒（插件 30 秒收不到算断了）。只要插件用得到的几段（自己打时 allplayers_* 这些也不给）。
pub fn text(port: u16, token: &str) -> String {
    let lines = [
        MARKER.to_string(),
        "// To remove it, turn off the game config switch on the CS2 page in Kotone.".into(),
        "\"Kotone\"".into(),
        "{".into(),
        format!("\t\"uri\"\t\t\"http://127.0.0.1:{port}/\""),
        "\t\"timeout\"\t\"5.0\"".into(),
        "\t\"buffer\"\t\"0.1\"".into(),
        "\t\"throttle\"\t\"0.1\"".into(),
        "\t\"heartbeat\"\t\"10.0\"".into(),
        "\t\"auth\"".into(),
        "\t{".into(),
        format!("\t\t\"token\"\t\"{token}\""),
        "\t}".into(),
        "\t\"data\"".into(),
        "\t{".into(),
        "\t\t\"provider\"\t\t\"1\"".into(),
        "\t\t\"map\"\t\t\t\"1\"".into(),
        "\t\t\"map_round_wins\"\t\"1\"".into(),
        "\t\t\"round\"\t\t\t\"1\"".into(),
        "\t\t\"player_id\"\t\t\"1\"".into(),
        "\t\t\"player_state\"\t\t\"1\"".into(),
        "\t\t\"player_match_stats\"\t\"1\"".into(),
        "\t\t\"bomb\"\t\t\t\"1\"".into(),
        "\t\t\"phase_countdowns\"\t\"1\"".into(),
        "\t}".into(),
        "}".into(),
        String::new(),
    ];
    lines.join("\n")
}

/// 是不是我们写的。
pub fn ours(content: &str) -> bool {
    content.trim_start_matches('\u{feff}').starts_with(MARKER)
}

/// 从文件里读回端口和令牌（真机脚本、页面用；不是我们写的返回 None）。
pub fn read_back(content: &str) -> Option<(u16, String)> {
    if !ours(content) {
        return None;
    }
    let value = |key: &str| {
        content.lines().find_map(|line| {
            let mut parts = line.split('"').filter(|p| !p.trim().is_empty());
            (parts.next()? == key).then(|| parts.next().map(str::to_string))?
        })
    };
    let uri = value("uri")?;
    let port = uri.trim_end_matches('/').rsplit(':').next()?.parse().ok()?;
    Some((port, value("token")?))
}

/// 这个文件现在怎样（插件页显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfgState {
    /// 写好了，内容是现在的端口和令牌。
    Written,
    /// 没有这个文件。
    Absent,
    /// 要写（或要改），CS2 开着，等它关了再写。
    WaitWrite,
    /// 要删，CS2 开着，等它关了再删。
    WaitRemove,
    /// 有一个同名的文件不是我们写的（要删时不碰它）。
    Foreign,
}

impl CfgState {
    pub fn as_str(self) -> &'static str {
        match self {
            CfgState::Written => "written",
            CfgState::Absent => "absent",
            CfgState::WaitWrite => "wait_write",
            CfgState::WaitRemove => "wait_remove",
            CfgState::Foreign => "foreign",
        }
    }
}

/// 让 `cfg_dir` 里的文件变成想要的样子：`want` = 要写的内容（None = 不要这个文件）。CS2 开着（`game_running`）时
/// 只看不动。同名但不是我们写的文件：要写时先备份到 `backups`（`<文件名>.<毫秒>.bak`），要删时不碰。
pub fn apply(
    cfg_dir: &Path,
    want: Option<&str>,
    game_running: bool,
    backups: &Path,
    now_ms: u64,
) -> Result<CfgState, String> {
    if !cfg_dir.is_dir() {
        return Err(format!("找不到 CS2 的 cfg 目录：{}", cfg_dir.display()));
    }
    let path = cfg_dir.join(FILE);
    let existing = std::fs::read_to_string(&path).ok();
    match want {
        Some(text) => {
            if existing.as_deref() == Some(text) {
                return Ok(CfgState::Written);
            }
            if game_running {
                return Ok(CfgState::WaitWrite);
            }
            if let Some(old) = existing.as_deref().filter(|old| !ours(old)) {
                std::fs::create_dir_all(backups).map_err(|e| format!("建备份目录失败：{e}"))?;
                let backup = backups.join(format!("{FILE}.{now_ms}.bak"));
                std::fs::write(&backup, old).map_err(|e| format!("备份原来的文件失败：{e}"))?;
            }
            std::fs::write(&path, text).map_err(|e| format!("写 {} 失败：{e}", path.display()))?;
            Ok(CfgState::Written)
        }
        None => match existing {
            None => Ok(CfgState::Absent),
            Some(old) if !ours(&old) => Ok(CfgState::Foreign),
            Some(_) if game_running => Ok(CfgState::WaitRemove),
            Some(_) => {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("删 {} 失败：{e}", path.display()))?;
                Ok(CfgState::Absent)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试的假目录一律在仓库的 `target/` 下，不碰真的 Steam / CS2。
    fn fake_install() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/cs2-tests");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(&root).unwrap();
        std::fs::create_dir_all(cfg_dir_in(dir.path())).unwrap();
        dir
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn locate_prefers_the_override_and_never_looks_in_an_isolated_instance() {
        let never = || -> Option<PathBuf> { panic!("不该去找 Steam") };
        assert_eq!(
            locate_with(Some("D:\\fake".into()), Some("x".into()), never),
            Where::Dir(cfg_dir_in(Path::new("D:\\fake")))
        );
        assert_eq!(
            locate_with(None, Some("C:\\home".into()), never),
            Where::Isolated
        );
        assert_eq!(
            locate_with(Some("  ".into()), None, || Some(PathBuf::from("E:\\cs2"))),
            Where::Dir(cfg_dir_in(Path::new("E:\\cs2")))
        );
        assert_eq!(
            locate_with(None, Some(String::new()), || None),
            Where::NotInstalled
        );
        assert_eq!(
            cfg_dir_in(Path::new("C:\\S\\Counter-Strike Global Offensive")),
            PathBuf::from("C:\\S\\Counter-Strike Global Offensive\\game\\csgo\\cfg")
        );
    }

    #[test]
    fn the_file_has_the_port_the_token_and_the_sections() {
        let text = text(23730, TOKEN);
        assert!(text.is_ascii());
        assert!(ours(&text));
        assert!(text.contains("\"uri\"\t\t\"http://127.0.0.1:23730/\""));
        for section in [
            "provider",
            "map",
            "round",
            "player_id",
            "player_state",
            "player_match_stats",
            "bomb",
            "phase_countdowns",
        ] {
            assert!(text.contains(&format!("\"{section}\"")), "{section}");
        }
        assert!(!text.contains("allplayers"));
        assert_eq!(read_back(&text), Some((23730, TOKEN.to_string())));
        assert_eq!(read_back("\"Other\"\n{\n\"uri\" \"http://x:1\"\n}"), None);
        // 括号配对（KeyValues 的块）
        assert_eq!(text.matches('{').count(), text.matches('}').count());
    }

    #[test]
    fn write_then_remove_only_our_file() {
        let install = fake_install();
        let dir = cfg_dir_in(install.path());
        let backups = install.path().join("backups");
        let want = text(23730, TOKEN);
        assert_eq!(
            apply(&dir, Some(&want), false, &backups, 1),
            Ok(CfgState::Written)
        );
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), want);
        // 再来一次：不动
        assert_eq!(
            apply(&dir, Some(&want), false, &backups, 2),
            Ok(CfgState::Written)
        );
        // 端口变了，游戏开着：等
        let moved = text(23731, TOKEN);
        assert_eq!(
            apply(&dir, Some(&moved), true, &backups, 3),
            Ok(CfgState::WaitWrite)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(FILE)).unwrap(),
            want,
            "游戏开着不改"
        );
        assert_eq!(
            apply(&dir, Some(&moved), false, &backups, 4),
            Ok(CfgState::Written)
        );
        assert!(!backups.exists(), "我们自己的旧文件不用备份");
        // 关掉：游戏开着时等，关了删
        assert_eq!(
            apply(&dir, None, true, &backups, 5),
            Ok(CfgState::WaitRemove)
        );
        assert!(dir.join(FILE).exists());
        assert_eq!(apply(&dir, None, false, &backups, 6), Ok(CfgState::Absent));
        assert!(!dir.join(FILE).exists());
        assert_eq!(apply(&dir, None, false, &backups, 7), Ok(CfgState::Absent));
        // 别的 GSI 文件一个都不碰
        std::fs::write(dir.join("gamestate_integration_other.cfg"), "x").unwrap();
        apply(&dir, Some(&want), false, &backups, 8).unwrap();
        apply(&dir, None, false, &backups, 9).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("gamestate_integration_other.cfg")).unwrap(),
            "x"
        );
    }

    #[test]
    fn a_file_we_did_not_write_is_backed_up_before_writing_and_never_removed() {
        let install = fake_install();
        let dir = cfg_dir_in(install.path());
        let backups = install.path().join("backups");
        std::fs::write(dir.join(FILE), "\"Someone\" {}").unwrap();
        assert_eq!(apply(&dir, None, false, &backups, 1), Ok(CfgState::Foreign));
        assert!(dir.join(FILE).exists(), "不是我们写的，不删");
        let want = text(23730, TOKEN);
        assert_eq!(
            apply(&dir, Some(&want), false, &backups, 42),
            Ok(CfgState::Written)
        );
        let backup = backups.join(format!("{FILE}.42.bak"));
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "\"Someone\" {}");
    }

    #[test]
    fn a_missing_cfg_dir_is_an_error() {
        let install = fake_install();
        let missing = install.path().join("nope");
        assert!(apply(&missing, Some("x"), false, &missing, 1).is_err());
    }
}
