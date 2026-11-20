//! 路径：解析（相对路径按 Kotone 文件区）、是否在某个目录里、游戏目录、备份、“问过的游戏”记录。

use std::path::{Component, Path, PathBuf};

use crate::{ExecConfig, GameDir};

/// 解析模型给的路径：展开 `%VAR%` / `$env:VAR`，相对路径按 Kotone 文件区，去掉 `.` / `..`，
/// 再把已经存在的那一段换成真实路径（跟随目录联接 / 符号链接，免得借联接绕出去）。
pub fn resolve(cfg: &ExecConfig, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("path 不能为空".into());
    }
    let expanded = expand_env(raw);
    let path = PathBuf::from(&expanded);
    let absolute = if path.is_absolute() {
        path
    } else {
        cfg.files_dir().join(path)
    };
    Ok(real(&lexical(&absolute)))
}

/// 去掉 `.` 和 `..`（不访问磁盘）。
pub fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir) | Some(Component::Prefix(_)) | None
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 最深的已存在祖先换成真实路径，后面原样接上。
pub fn real(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(&existing) {
            let mut out = strip_verbatim(&canonical);
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `\\?\C:\x` → `C:\x`，`\\?\UNC\srv\share` → `\\srv\share`。
fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn expand_env(raw: &str) -> String {
    let mut out = raw.to_string();
    // %VAR%
    while let Some(start) = out.find('%') {
        let Some(len) = out[start + 1..].find('%') else {
            break;
        };
        let name = &out[start + 1..start + 1 + len];
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => {
                out = format!("{}{}{}", &out[..start], value, &out[start + 2 + len..]);
            }
            _ => break,
        }
    }
    // $env:VAR（到下一个分隔符为止）
    while let Some(start) = out.to_ascii_lowercase().find("$env:") {
        let name_start = start + 5;
        let end = out[name_start..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '(' || c == ')'))
            .map(|i| name_start + i)
            .unwrap_or(out.len());
        let name = out[name_start..end].replace(['(', ')'], "");
        match std::env::var(&name) {
            Ok(value) if !name.is_empty() => {
                out = format!("{}{}{}", &out[..start], value, &out[end..]);
            }
            _ => break,
        }
    }
    out
}

fn key(path: &Path) -> Vec<String> {
    real(&lexical(path))
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// `path` 是 `dir` 本身或在它里面（大小写不敏感，按路径段比较）。
pub fn within(path: &Path, dir: &Path) -> bool {
    let p = key(path);
    let d = key(dir);
    !d.is_empty() && p.len() >= d.len() && p[..d.len()] == d[..]
}

/// 这个路径在哪个游戏的目录里。
pub fn game_of<'a>(cfg: &'a ExecConfig, path: &Path) -> Option<&'a GameDir> {
    cfg.game_dirs.iter().find(|g| within(path, &g.dir))
}

/// 命令里提到了哪些游戏目录（按字面，大小写不敏感，正反斜杠都算）。
pub fn games_mentioned<'a>(cfg: &'a ExecConfig, command: &str) -> Vec<&'a GameDir> {
    let text = command.to_lowercase().replace('/', "\\");
    cfg.game_dirs
        .iter()
        .filter(|g| {
            let dir = g.dir.to_string_lossy().to_lowercase().replace('/', "\\");
            let dir = dir.trim_end_matches('\\');
            !dir.is_empty() && text.contains(dir)
        })
        .collect()
}

/// 改文件前备份：`<home>/backups/<毫秒>/<盘符>/<原路径>`（UNC 放在 `UNC/` 下）。
pub fn backup(cfg: &ExecConfig, path: &Path, stamp: u128) -> Result<PathBuf, String> {
    let mut rel = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => {
                let s = p.as_os_str().to_string_lossy();
                let s = s.trim_start_matches(r"\\?\");
                if let Some(unc) = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("UNC\\")) {
                    rel.push("UNC");
                    for part in unc.split('\\').filter(|p| !p.is_empty()) {
                        rel.push(part);
                    }
                } else {
                    rel.push(s.trim_end_matches(':'));
                }
            }
            Component::RootDir | Component::CurDir | Component::ParentDir => {}
            Component::Normal(n) => rel.push(n),
        }
    }
    let dest = cfg.backups_dir().join(stamp.to_string()).join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建备份目录失败：{e}"))?;
    }
    std::fs::copy(path, &dest).map_err(|e| format!("备份失败：{e}"))?;
    Ok(dest)
}

/// 问过、玩家同意改的游戏目录（`<home>/exec-state.json`）。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecState {
    #[serde(default)]
    pub approved_games: Vec<String>,
}

impl ExecState {
    pub fn load(cfg: &ExecConfig) -> Self {
        std::fs::read_to_string(cfg.state_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn approve_game(cfg: &ExecConfig, game_id: &str) -> Result<(), String> {
        let mut state = Self::load(cfg);
        if state.approved_games.iter().any(|g| g == game_id) {
            return Ok(());
        }
        state.approved_games.push(game_id.to_string());
        let text = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        write_atomic(&cfg.state_path(), text.as_bytes())
    }
}

/// 先写临时文件再改名。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".kotone-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入失败：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("写入失败：{e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(home: &Path) -> ExecConfig {
        ExecConfig {
            home: home.to_path_buf(),
            game_dirs: vec![],
        }
    }

    #[test]
    fn relative_paths_land_in_the_files_area_and_dots_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(&dir.path().join("agent"));
        let p = resolve(&c, "notes\\..\\a.txt").unwrap();
        assert!(within(&p, &c.files_dir()));
        assert!(p.ends_with("a.txt"));
        let escaped = resolve(&c, "..\\..\\x.txt").unwrap();
        assert!(!within(&escaped, &c.files_dir()));
    }

    #[test]
    fn within_compares_whole_segments_case_insensitively() {
        assert!(within(
            Path::new("C:\\Games\\WoW\\a"),
            Path::new("c:\\games\\wow")
        ));
        assert!(!within(
            Path::new("C:\\Games\\WoW2\\a"),
            Path::new("C:\\Games\\WoW")
        ));
    }

    #[test]
    fn env_vars_expand() {
        std::env::set_var("KOTONE_EXEC_TEST_DIR", "C:\\Somewhere");
        assert_eq!(expand_env("%KOTONE_EXEC_TEST_DIR%\\a"), "C:\\Somewhere\\a");
        assert_eq!(
            expand_env("$env:KOTONE_EXEC_TEST_DIR\\b"),
            "C:\\Somewhere\\b"
        );
        assert_eq!(expand_env("100%"), "100%");
    }

    #[test]
    fn backup_keeps_the_drive_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(&dir.path().join("agent"));
        let src = dir.path().join("game").join("Config.wtf");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, "x").unwrap();
        let dest = backup(&c, &real(&src), 42).unwrap();
        assert!(dest.starts_with(c.backups_dir().join("42")));
        assert!(dest.ends_with("game\\Config.wtf"));
        assert_eq!(std::fs::read_to_string(dest).unwrap(), "x");
    }
}
