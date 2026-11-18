//! 「琴音」页的“她是谁 / 她记得什么 / 她会什么”（ADR 022 / 023）：读写琴音的工作区。
//!
//! 文件规则（上限、注入扫描、原子写入）都在 [`kotone_agent::workspace::Workspace`] 里，
//! 和琴音自己用 `memory`、`soul.write`、`skill.manage` 工具写时一样；这里只是给设置页用的入口。
//! 改动在下一段对话生效（对话开始时冻结快照，见 ADR 022）。

use kotone_agent::workspace::{self as ws, Workspace};
use serde_json::{json, Value};

/// 头像上限 2 MB；只收常见图片。
const AVATAR_LIMIT: usize = 2 * 1024 * 1024;
const AVATAR_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

fn workspace() -> Workspace {
    crate::agent::workspace()
}

/// 头像文件名：琴音（默认角色）是 `avatar.<ext>`（升级前的名字），其他角色是 `avatar-<角色 id>.<ext>`。
fn avatar_stem(role: &str) -> String {
    if role.is_empty() || role == kotone_agent::roles::DEFAULT_ROLE {
        "avatar".into()
    } else {
        format!("avatar-{role}")
    }
}

fn avatar_name(ext: &str, role: &str) -> Result<String, String> {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if !AVATAR_TYPES.contains(&ext.as_str()) {
        return Err("头像只支持 PNG、JPG、WebP、GIF".into());
    }
    if !role.is_empty() && !kotone_agent::roles::valid_role_id(role) {
        return Err("角色 id 不合法".into());
    }
    Ok(format!("{}.{ext}", avatar_stem(role)))
}

/// 某个角色的头像文件（`role` 为 None：任何角色的）。
fn is_avatar_file_of(name: &str, role: Option<&str>) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if !AVATAR_TYPES.contains(&ext) {
        return false;
    }
    match role {
        Some(role) => stem == avatar_stem(role),
        None => {
            stem == "avatar"
                || stem
                    .strip_prefix("avatar-")
                    .is_some_and(kotone_agent::roles::valid_role_id)
        }
    }
}

fn is_avatar_file(name: &str) -> bool {
    is_avatar_file_of(name, None)
}

/// 总览：名字、记忆文件、游戏记忆、Skill、上限。
#[tauri::command]
pub fn workspace_overview(app: tauri::AppHandle) -> Result<Value, String> {
    use tauri::Manager;
    let name_override = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .agent
        .identity
        .name
        .clone();
    let ws = workspace();
    let identity = ws.identity(&name_override);
    let size = |file: &str| ws.read_file(file).map(|t| t.chars().count()).unwrap_or(0);
    let games: Vec<Value> = ws
        .games()
        .into_iter()
        .map(|id| json!({"id": id, "chars": size(&format!("games/{id}"))}))
        .collect();
    let skills: Vec<Value> = ws
        .skills()
        .into_iter()
        .map(|s| json!({"name": s.name, "description": s.description}))
        .collect();
    Ok(json!({
        "name": identity.name,
        "defaultName": ws::DEFAULT_NAME,
        "dir": ws.dir(),
        "files": {
            "soul": {"chars": size(ws::SOUL), "limit": ws::SOUL_LIMIT},
            "user": {"chars": size(ws::USER), "limit": ws::USER_LIMIT},
            "memory": {"chars": size(ws::MEMORY), "limit": ws::MEMORY_LIMIT},
        },
        "gameLimit": ws::GAME_LIMIT,
        "games": games,
        "skills": skills,
    }))
}

/// 读一份：`SOUL.md`、`USER.md`、`MEMORY.md`、`games/<id>`。
#[tauri::command]
pub fn workspace_read(file: String) -> Result<String, String> {
    workspace().read_file(&file)
}

/// 整份写入（同样有上限与注入扫描）。空文本 = 清空。
#[tauri::command]
pub fn workspace_write(file: String, text: String) -> Result<(), String> {
    workspace().write_file(&file, &text)
}

/// 删除一份游戏记忆。
#[tauri::command]
pub fn workspace_delete_game(id: String) -> Result<(), String> {
    if !ws::valid_game_id(&id) {
        return Err(format!("游戏 id「{id}」不合法"));
    }
    let path = workspace()
        .dir()
        .join("memory")
        .join("games")
        .join(format!("{id}.md"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除失败：{e}")),
    }
}

/// 保存头像，返回文件名（再由设置页写进 `agent.identity.avatar`）。旧头像一并删掉。
#[tauri::command]
pub fn workspace_set_avatar(
    bytes: Vec<u8>,
    ext: String,
    role: Option<String>,
) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("图片是空的".into());
    }
    if bytes.len() > AVATAR_LIMIT {
        return Err("头像不能超过 2 MB".into());
    }
    let role = role.unwrap_or_default();
    let name = avatar_name(&ext, &role)?;
    let ws = workspace();
    if let Ok(entries) = std::fs::read_dir(ws.dir()) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| is_avatar_file_of(n, Some(&role)))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    std::fs::write(ws.dir().join(&name), &bytes).map_err(|e| format!("保存头像失败：{e}"))?;
    Ok(name)
}

/// 读头像的字节（文件名来自设置）；没有 = 用内置形象。
#[tauri::command]
pub fn workspace_avatar(name: String) -> Result<Option<Vec<u8>>, String> {
    if !is_avatar_file(&name) {
        return Ok(None);
    }
    Ok(std::fs::read(workspace().dir().join(name)).ok())
}

#[tauri::command]
pub fn workspace_delete_skill(name: String) -> Result<(), String> {
    workspace().skill_delete(&name)
}

/// 从一个文件夹导入 Skill（文件夹里要有 `SKILL.md`，agentskills.io 格式）。
#[tauri::command]
pub fn workspace_import_skill(dir: String) -> Result<Value, String> {
    let info = workspace().skill_import_dir(std::path::Path::new(&dir))?;
    Ok(json!({"name": info.name, "description": info.description}))
}

/// 在资源管理器里打开工作区（或某个 Skill 的文件夹），方便手动备份、分享。
#[tauri::command]
pub fn workspace_open_dir(skill: Option<String>) -> Result<(), String> {
    let ws = workspace();
    let dir = match skill {
        Some(name) if ws::valid_skill_name(&name) => ws.dir().join(ws::SKILLS).join(name),
        Some(name) => return Err(format!("Skill 名字「{name}」不合法")),
        None => ws.dir().to_path_buf(),
    };
    crate::open_in_file_manager(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_avatar_files_are_accepted() {
        assert_eq!(avatar_name(".PNG", "").unwrap(), "avatar.png");
        assert_eq!(avatar_name("png", "kotone").unwrap(), "avatar.png");
        assert_eq!(avatar_name("png", "xiaoyu").unwrap(), "avatar-xiaoyu.png");
        assert!(avatar_name("exe", "").is_err());
        assert!(avatar_name("png", "../x").is_err());
        assert!(is_avatar_file("avatar.webp"));
        assert!(is_avatar_file("avatar-r3.png"));
        assert!(!is_avatar_file("avatar-../x.png"));
        assert!(!is_avatar_file("../avatar.png"));
        assert!(!is_avatar_file("SOUL.md"));
        // 换一个角色的头像只删那个角色的旧头像
        assert!(is_avatar_file_of("avatar-r3.png", Some("r3")));
        assert!(!is_avatar_file_of("avatar.png", Some("r3")));
        assert!(is_avatar_file_of("avatar.png", Some("kotone")));
    }
}
