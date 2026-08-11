//! 游戏 profile：CRUD + 前台进程匹配（docs/development.md §5.1、§5.4）
//! 存储：~/.kotone/profiles/<id>.json
//!
//! 内置模板存放在 `resources/profiles/*.json`；首次运行落盘两个 profile：
//! - `lol`：League of Legends（§5.4 示例值，delay 20/20/20，Unicode 逐字）
//! - `generic`：通用，匹配任意前台窗口（processNames 为空 = 通配）

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::settings::kotone_dir;

/// Profile 与导入文件的资源上限。所有限制同时用于普通保存与 `.kprofile`
/// 导入，避免绕过 UI 直接调用 IPC 时写入异常大的配置。
pub const MAX_PROFILE_JSON_BYTES: u64 = 1024 * 1024;
pub const MAX_PROFILE_PACKAGE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PROFILE_ARCHIVE_ENTRIES: usize = 4;
pub const MAX_PROFILE_ICON_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_PROFILE_ICON_DIMENSION: u32 = 2048;
pub const MAX_HOTWORDS_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_HOTWORDS: usize = 4096;
pub const MAX_HOTWORD_CHARS: usize = 128;
pub const MAX_PROFILE_CHANNELS: usize = 32;
pub const MAX_PROFILE_DELAY_MS: u32 = 5_000;

/// 聊天频道（ADR-008）：一个游戏可有多个聊天频道，各有独立发送策略。
/// 频道切换热键按声明顺序循环；`default` 标记默认频道（缺省取第一个）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChannel {
    /// 语义 id（如 "team" / "all"）：跨游戏复用，切换热键与悬浮窗都按 id 工作
    pub id: String,
    /// 悬浮窗展示名（如「队伍」「所有人」；措辞由 profile 定，不硬编码）
    pub display_name: String,
    /// 按键策略：该频道专属的开聊天框键（如 Shift+Enter）；
    /// None = 沿用 profile 默认 openChatKey
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_chat_key: Option<String>,
    /// 前缀策略：发送文本前拼接的前缀（如 "/all "）；与 openChatKey 独立，
    /// 两者可同时设置（先按专属键开框、再发带前缀文本）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_prefix: Option<String>,
    /// 默认频道标记；多个标记时取第一个，全无标记时取列表首项
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
}

/// 频道发送策略（解析结果）：开框键 + 文本前缀，全部落定无 Option
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelStrategy {
    pub id: String,
    pub display_name: String,
    pub open_chat_key: String,
    pub text_prefix: String,
    pub is_default: bool,
}

/// 游戏 profile（默认值对齐 LeagueAkari 实测：delay 20/20/20）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameProfile {
    pub id: String,
    pub display_name: String,
    pub process_names: Vec<String>,
    pub window_title_patterns: Vec<String>,
    pub open_chat_key: String,
    pub send_key: String,
    pub pre_open_delay_ms: u32,
    pub pre_paste_delay_ms: u32,
    pub pre_send_delay_ms: u32,
    /// false = Unicode 逐字（不污染剪贴板）；true = 剪贴板粘贴
    pub prefer_clipboard_paste: bool,
    /// 图标文件名（相对 profiles/icons/ 目录；None = 无图标，UI 用占位）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub hotwords: Vec<String>,
    /// 聊天频道列表（ADR-008；空 = 单频道游戏，从 openChatKey 合成默认频道，
    /// 行为与频道功能引入前完全一致）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ProfileChannel>,
    /// 用户在内置 profile 里显式删除的内置词条：版本更新合并内置热词时
    /// 尊重删除、不复活（见 ensure_builtin_in）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_builtin_hotwords: Vec<String>,
}

impl GameProfile {
    /// 内置 LOL profile。内容由 resources/profiles/lol.json 维护，
    /// Rust 代码只负责解析与返回副本。
    pub fn builtin_lol() -> Self {
        static PROFILE: OnceLock<GameProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                parse_builtin_profile("lol", include_str!("../resources/profiles/lol.json"))
            })
            .clone()
    }

    /// 内置通用 profile。内容由 resources/profiles/generic.json 维护。
    pub fn builtin_generic() -> Self {
        static PROFILE: OnceLock<GameProfile> = OnceLock::new();
        PROFILE
            .get_or_init(|| {
                parse_builtin_profile(
                    "generic",
                    include_str!("../resources/profiles/generic.json"),
                )
            })
            .clone()
    }

    /// 有效频道列表（ADR-008）：channels 为空时从 openChatKey 合成单个
    /// 默认频道（单频道游戏行为零变化）；返回顺序 = 声明顺序（循环切换顺序）。
    pub fn channels_effective(&self) -> Vec<ChannelStrategy> {
        if self.channels.is_empty() {
            return vec![ChannelStrategy {
                id: "default".into(),
                display_name: self.display_name.clone(),
                open_chat_key: self.open_chat_key.clone(),
                text_prefix: String::new(),
                is_default: true,
            }];
        }
        let default_idx = self.channels.iter().position(|c| c.default).unwrap_or(0);
        self.channels
            .iter()
            .enumerate()
            .map(|(i, c)| ChannelStrategy {
                id: c.id.clone(),
                display_name: c.display_name.clone(),
                open_chat_key: c
                    .open_chat_key
                    .clone()
                    .unwrap_or_else(|| self.open_chat_key.clone()),
                text_prefix: c.text_prefix.clone().unwrap_or_default(),
                is_default: i == default_idx,
            })
            .collect()
    }

    /// 按 id 解析频道策略；None / 未知 id → 默认频道（发送路径的兜底）
    pub fn channel_strategy(&self, id: Option<&str>) -> ChannelStrategy {
        let channels = self.channels_effective();
        if let Some(id) = id {
            if let Some(s) = channels.iter().find(|c| c.id == id) {
                return s.clone();
            }
        }
        channels
            .into_iter()
            .find(|c| c.is_default)
            .expect("channels_effective 至少含一个默认频道")
    }

    /// 循环切换的下一个频道 id（ADR-008：按声明顺序循环，从默认频道的下一个开始）。
    /// 当前 id 缺失/未知（如切换了 profile 后的残留）→ 默认频道的下一个。
    /// 单频道返回 None（调用方据此让切换键无操作）。
    pub fn next_channel_id(&self, current: Option<&str>) -> Option<String> {
        let channels = self.channels_effective();
        if channels.len() < 2 {
            return None;
        }
        let cur_idx = current.and_then(|id| channels.iter().position(|c| c.id == id));
        let base = match cur_idx {
            Some(i) => i,
            None => channels.iter().position(|c| c.is_default).unwrap_or(0),
        };
        Some(channels[(base + 1) % channels.len()].id.clone())
    }
}

fn parse_builtin_profile(expected_id: &str, json: &str) -> GameProfile {
    let profile: GameProfile = serde_json::from_str(json)
        .unwrap_or_else(|e| panic!("内置 profile {expected_id}.json 格式错误: {e}"));
    assert_eq!(profile.id, expected_id, "内置 profile 文件名与 id 不一致");
    profile
}

/// 内置 profile 的图标（字节 + 文件名；None = 该内置无图标）。
/// 图标随本体编译打入，`ensure_builtin` 首次运行时物化到 profiles/icons/。
fn builtin_icon(id: &str) -> Option<(&'static [u8], &'static str)> {
    match id {
        "lol" => Some((
            include_bytes!("../resources/profiles/icons/lol-kotone.webp"),
            "lol-kotone.webp",
        )),
        _ => None,
    }
}

fn profiles_dir() -> PathBuf {
    kotone_dir().join("profiles")
}

fn profile_path_in(dir: &Path, id: &str) -> PathBuf {
    // id 只保留文件安全字符，防路径穿越
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}.json"))
}

// ---------- profile 图标（profiles/icons/ 目录，仿 history::write_audio_in） ----------

/// profile 图标目录（dir 下的 icons/ 子目录）
fn icons_dir_in(dir: &Path) -> PathBuf {
    dir.join("icons")
}

/// icon 文件名安全化（防路径穿越）：只允许字母数字/点/下划线/连字符，
/// 禁空、禁 `..`、禁分隔符。非法返回 None。
fn sanitize_icon_name(name: &str) -> Option<String> {
    if name.is_empty()
        || name.len() > 64
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return None;
    }
    Some(name.to_string())
}

/// 写入 profile 图标（相对 icons/ 目录）
pub fn write_profile_icon_in(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let safe = sanitize_icon_name(name).ok_or_else(|| format!("非法图标文件名：{name}"))?;
    let idir = icons_dir_in(dir);
    std::fs::create_dir_all(&idir).map_err(|e| format!("创建图标目录失败: {e}"))?;
    std::fs::write(idir.join(&safe), bytes).map_err(|e| format!("写入图标失败: {e}"))
}

/// 读取 profile 图标字节（文件名非法/文件不存在 → None）
pub fn read_profile_icon_in(dir: &Path, name: &str) -> Option<Vec<u8>> {
    let safe = sanitize_icon_name(name)?;
    std::fs::read(icons_dir_in(dir).join(safe)).ok()
}

/// 读取指定 profile 的图标字节（无图标/读取失败 → None），默认目录版
pub fn profile_icon_bytes(id: &str) -> Option<Vec<u8>> {
    let profile = get(id)?;
    let name = profile.icon.as_deref()?;
    read_profile_icon_in(&profiles_dir(), name)
}

/// 确保内置 profile 已落盘（首次运行调用；不覆盖用户已有文件）。
/// 文件已存在时做「版本更新合并」：把新版扩充的内置词条追加到用户文件末尾
/// （尊重 removedBuiltinHotwords 里用户显式删除的词条，不复活）。
pub fn ensure_builtin() -> Result<(), String> {
    ensure_builtin_in(&profiles_dir())
}

pub fn ensure_builtin_in(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 profiles 目录失败: {e}"))?;
    for p in [GameProfile::builtin_lol(), GameProfile::builtin_generic()] {
        let path = profile_path_in(dir, &p.id);
        if !path.exists() {
            let json = serde_json::to_string_pretty(&p)
                .map_err(|e| format!("序列化内置 profile 失败: {e}"))?;
            std::fs::write(&path, json).map_err(|e| format!("写入内置 profile 失败: {e}"))?;
        } else {
            merge_builtin_hotwords_in(dir, &p);
        }
        // 物化内置图标（已存在则不动）
        if let Some((bytes, name)) = builtin_icon(&p.id) {
            if read_profile_icon_in(dir, name).is_none() {
                write_profile_icon_in(dir, name, bytes)?;
            }
        }
    }
    Ok(())
}

/// 版本更新合并：已有内置 profile 文件补上新版新增内容——
/// 1) 内置热词：内置词条 - 文件现有热词 - 用户显式删除的内置词条，追加到末尾
///    （用户自定义词条与其余字段原样保留）；
/// 2) 聊天频道（ADR-008）：文件缺 channels 且内置声明了频道时整体补入
///    （只补缺失，不覆盖用户已有频道定义）。
///
/// 合并失败只记日志，不阻断启动流程。
fn merge_builtin_hotwords_in(dir: &Path, builtin: &GameProfile) {
    let result = (|| -> Result<(), String> {
        let path = profile_path_in(dir, &builtin.id);
        let raw =
            std::fs::read_to_string(&path).map_err(|e| format!("读取内置 profile 失败: {e}"))?;
        let mut file: GameProfile =
            serde_json::from_str(&raw).map_err(|e| format!("解析内置 profile 失败: {e}"))?;
        let mut changed = false;
        let missing: Vec<String> = builtin
            .hotwords
            .iter()
            .filter(|w| !file.hotwords.contains(w) && !file.removed_builtin_hotwords.contains(w))
            .cloned()
            .collect();
        if !missing.is_empty() {
            file.hotwords.extend(missing);
            changed = true;
        }
        if file.channels.is_empty() && !builtin.channels.is_empty() {
            file.channels = builtin.channels.clone();
            changed = true;
        }
        if file.icon.is_none() && builtin.icon.is_some() {
            // 老安装升级：内置新增 icon 时补入（用户显式清空 icon 无法表达，
            // 视为从未设置，补默认即可）
            file.icon = builtin.icon.clone();
            changed = true;
        }
        if changed {
            save_in(dir, &file)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("合并内置 profile 失败（{}，忽略）: {e}", builtin.id);
    }
}

/// 列出全部 profile（按 id 排序，稳定输出）
pub fn list() -> Vec<GameProfile> {
    list_in(&profiles_dir())
}

pub fn list_in(dir: &Path) -> Vec<GameProfile> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(p) = serde_json::from_str::<GameProfile>(&raw) {
                out.push(p);
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 按 id 读取单个 profile
pub fn get(id: &str) -> Option<GameProfile> {
    get_in(&profiles_dir(), id)
}

pub fn get_in(dir: &Path, id: &str) -> Option<GameProfile> {
    let path = profile_path_in(dir, id);
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 保存 profile
pub fn save(profile: &GameProfile) -> Result<(), String> {
    save_in(&profiles_dir(), profile)
}

/// 校验 profile 的业务字段与资源上限。该函数是持久化的权威边界，导入包与
/// 前端普通保存均必须经过这里，不能只依赖 UI 的输入控件。
pub fn validate_profile(profile: &GameProfile) -> Result<(), String> {
    fn bounded(
        label: &str,
        value: &str,
        max_chars: usize,
        allow_empty: bool,
    ) -> Result<(), String> {
        let chars = value.chars().count();
        if (!allow_empty && value.trim().is_empty()) || chars > max_chars {
            return Err(format!(
                "{label} 长度无效：应为 {}..={max_chars} 个字符，实际 {chars}",
                if allow_empty { 0 } else { 1 }
            ));
        }
        Ok(())
    }

    fn valid_injection_key(value: &str) -> bool {
        if value.len() > 32 || value.split('+').any(|part| part.trim().is_empty()) {
            return false;
        }
        let parts: Vec<&str> = value.split('+').map(str::trim).collect();
        let Some((main, modifiers)) = parts.split_last() else {
            return false;
        };
        if modifiers.len() > 3
            || !modifiers.iter().all(|part| {
                matches!(
                    part.to_ascii_lowercase().as_str(),
                    "ctrl" | "control" | "alt" | "shift"
                )
            })
        {
            return false;
        }
        let main = main.to_ascii_lowercase();
        (main.len() == 1
            && main
                .bytes()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric()))
            || matches!(
                main.as_str(),
                "enter"
                    | "return"
                    | "esc"
                    | "escape"
                    | "tab"
                    | "space"
                    | "backspace"
                    | "delete"
                    | "del"
                    | "up"
                    | "down"
                    | "left"
                    | "right"
                    | "home"
                    | "end"
                    | "f1"
                    | "f2"
                    | "f3"
                    | "f4"
                    | "f5"
                    | "f6"
                    | "f7"
                    | "f8"
                    | "f9"
                    | "f10"
                    | "f11"
                    | "f12"
            )
    }

    bounded("profile id", &profile.id, 64, false)?;
    if !profile
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("profile id 只能包含 ASCII 字母、数字、连字符和下划线".into());
    }
    bounded("profile 显示名", &profile.display_name, 128, false)?;
    if profile.process_names.len() > 64 {
        return Err("进程名不能超过 64 个".into());
    }
    for value in &profile.process_names {
        bounded("进程名", value, 260, false)?;
    }
    if profile.window_title_patterns.len() > 64 {
        return Err("窗口标题规则不能超过 64 个".into());
    }
    for value in &profile.window_title_patterns {
        bounded("窗口标题规则", value, 512, false)?;
    }
    for (label, key) in [
        ("打开聊天框按键", profile.open_chat_key.as_str()),
        ("发送按键", profile.send_key.as_str()),
    ] {
        if !valid_injection_key(key) {
            return Err(format!("{label}格式无效：{key}"));
        }
    }
    for (label, delay) in [
        ("打开前延时", profile.pre_open_delay_ms),
        ("粘贴前延时", profile.pre_paste_delay_ms),
        ("发送前延时", profile.pre_send_delay_ms),
    ] {
        if delay > MAX_PROFILE_DELAY_MS {
            return Err(format!("{label}不能超过 {MAX_PROFILE_DELAY_MS}ms"));
        }
    }
    if let Some(icon) = &profile.icon {
        sanitize_icon_name(icon).ok_or_else(|| format!("非法图标文件名：{icon}"))?;
        let extension = Path::new(icon)
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg" | "webp")) {
            return Err("profile 图标只支持 PNG、JPEG 或 WebP".into());
        }
    }

    fn validate_words(label: &str, words: &[String]) -> Result<(), String> {
        if words.len() > MAX_HOTWORDS {
            return Err(format!("{label}不能超过 {MAX_HOTWORDS} 条"));
        }
        for word in words {
            let chars = word.chars().count();
            if word.trim().is_empty() || chars > MAX_HOTWORD_CHARS {
                return Err(format!(
                    "{label}包含空词条或超过 {MAX_HOTWORD_CHARS} 个字符的词条"
                ));
            }
        }
        Ok(())
    }
    validate_words("热词", &profile.hotwords)?;
    validate_words("已删除的内置热词", &profile.removed_builtin_hotwords)?;

    if profile.channels.len() > MAX_PROFILE_CHANNELS {
        return Err(format!("聊天频道不能超过 {MAX_PROFILE_CHANNELS} 个"));
    }
    let mut channel_ids = std::collections::HashSet::new();
    let mut defaults = 0usize;
    for channel in &profile.channels {
        bounded("频道 id", &channel.id, 64, false)?;
        if !channel
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("频道 id 格式无效：{}", channel.id));
        }
        if !channel_ids.insert(channel.id.as_str()) {
            return Err(format!("频道 id 重复：{}", channel.id));
        }
        bounded("频道显示名", &channel.display_name, 128, false)?;
        if let Some(key) = &channel.open_chat_key {
            if !valid_injection_key(key) {
                return Err(format!(
                    "频道 {} 的打开聊天框按键格式无效：{key}",
                    channel.id
                ));
            }
        }
        if let Some(prefix) = &channel.text_prefix {
            bounded("频道文字前缀", prefix, 512, true)?;
        }
        defaults += usize::from(channel.default);
    }
    if defaults > 1 {
        return Err("最多只能有一个默认聊天频道".into());
    }
    Ok(())
}

pub fn save_in(dir: &Path, profile: &GameProfile) -> Result<(), String> {
    validate_profile(profile)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 profiles 目录失败: {e}"))?;
    let json =
        serde_json::to_string_pretty(profile).map_err(|e| format!("序列化 profile 失败: {e}"))?;
    if json.len() as u64 > MAX_PROFILE_JSON_BYTES {
        return Err(format!(
            "profile JSON 不能超过 {}KB",
            MAX_PROFILE_JSON_BYTES / 1024
        ));
    }
    let path = profile_path_in(dir, &profile.id);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入 profile 失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘 profile 失败: {e}"))?;
    Ok(())
}

/// 删除 profile
#[allow(dead_code)] // CRUD 接口齐备，删除入口由前端子代理接入
pub fn delete(id: &str) -> Result<(), String> {
    let path = profile_path_in(&profiles_dir(), id);
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("删除 profile 失败: {e}"))?;
    }
    Ok(())
}

// ---------- 匹配纯逻辑（可单测） ----------

/// 进程名是否命中该 profile。
/// - processNames 为空（generic）→ 匹配任意进程；
/// - 否则与进程可执行文件名做大小写不敏感精确比较。
pub fn matches_process(profile: &GameProfile, process_name: &str) -> bool {
    if profile.process_names.is_empty() {
        return true;
    }
    profile
        .process_names
        .iter()
        .any(|p| p.eq_ignore_ascii_case(process_name))
}

/// 在一组 profile 中按前台进程名找匹配：
/// 优先返回「具体 profile」（processNames 非空），其次才是通配的 generic。
pub fn find_by_process(profiles: &[GameProfile], process_name: &str) -> Option<GameProfile> {
    profiles
        .iter()
        .find(|p| !p.process_names.is_empty() && matches_process(p, process_name))
        .or_else(|| {
            profiles
                .iter()
                .find(|p| p.process_names.is_empty() && matches_process(p, process_name))
        })
        .cloned()
}

// ---------- 热词导入导出（纯逻辑，可单测；文件对话框与 IO 在壳侧） ----------

/// 热词导出格式：UTF-8 文本，每行一个词条（末尾换行；空表导出空文件）。
/// 预留权重位「词条 权重」——底层 hotwords 是 Vec<String> 不支持权重，
/// 导出永不带权重列；导入按整行词条解析（见 parse_hotwords_import）。
pub fn format_hotwords_export(hotwords: &[String]) -> String {
    let mut out = String::new();
    for w in hotwords {
        out.push_str(w);
        out.push('\n');
    }
    out
}

/// 解析热词导入文件：每行一个词条；跳过空行与纯空白行，trim 两端空白，
/// 文件内重复词条只保留首次出现（保持顺序）。
pub fn parse_hotwords_import(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let w = line.trim();
        if w.is_empty() {
            continue;
        }
        if !out.iter().any(|x| x == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// 解析不受信任的热词文本并执行资源上限校验。
pub fn parse_hotwords_import_checked(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        let word = line.trim();
        if word.is_empty() || !seen.insert(word) {
            continue;
        }
        if word.chars().count() > MAX_HOTWORD_CHARS {
            return Err(format!("热词不能超过 {MAX_HOTWORD_CHARS} 个字符"));
        }
        if out.len() >= MAX_HOTWORDS {
            return Err(format!("热词文件不能超过 {MAX_HOTWORDS} 条非重复词条"));
        }
        out.push(word.to_string());
    }
    Ok(out)
}

/// 从文件导入热词的权威后端入口。先对打开后的文件执行大小限制，再解析、
/// 合并并通过 `save_in` 的完整 profile 校验。
pub fn import_hotwords_from_file(
    profile_id: &str,
    path: &Path,
) -> Result<HotwordMergeReport, String> {
    import_hotwords_from_file_in(&profiles_dir(), profile_id, path)
}

pub fn import_hotwords_from_file_in(
    dir: &Path,
    profile_id: &str,
    path: &Path,
) -> Result<HotwordMergeReport, String> {
    use std::io::Read as _;

    let file =
        std::fs::File::open(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let size = file
        .metadata()
        .map_err(|e| format!("读取 {} 元数据失败：{e}", path.display()))?
        .len();
    if size > MAX_HOTWORDS_FILE_BYTES {
        return Err(format!(
            "热词文件不能超过 {}KB",
            MAX_HOTWORDS_FILE_BYTES / 1024
        ));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_HOTWORDS_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    if bytes.len() as u64 > MAX_HOTWORDS_FILE_BYTES {
        return Err(format!(
            "热词文件不能超过 {}KB",
            MAX_HOTWORDS_FILE_BYTES / 1024
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| "热词文件必须是 UTF-8 文本".to_string())?;
    let incoming = parse_hotwords_import_checked(&text)?;
    let mut profile =
        get_in(dir, profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    let (merged, report) = merge_hotwords(&profile.hotwords, &incoming);
    profile.hotwords = merged;
    save_in(dir, &profile)?;
    Ok(report)
}

/// 热词合并导入报告
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotwordMergeReport {
    /// 实际新增的词条数
    pub added: usize,
    /// 与现有热词重复而跳过的词条数
    pub duplicates: usize,
    /// 合并后的热词总数
    pub total: usize,
}

/// 合并导入：incoming 中与 existing 重复（精确匹配）的跳过，
/// 新增追加在末尾（保持现有顺序不变）。
pub fn merge_hotwords(
    existing: &[String],
    incoming: &[String],
) -> (Vec<String>, HotwordMergeReport) {
    let mut merged = existing.to_vec();
    let mut report = HotwordMergeReport {
        total: 0,
        added: 0,
        duplicates: 0,
    };
    for w in incoming {
        if merged.iter().any(|x| x == w) {
            report.duplicates += 1;
        } else {
            merged.push(w.clone());
            report.added += 1;
        }
    }
    report.total = merged.len();
    (merged, report)
}

// ---------- 整包导入导出（.kprofile = ZIP：profile.json + icon.<ext>） ----------

/// 导出为 .kprofile ZIP 包：profile.json（清空 removed_builtin_hotwords——
/// 用户本地删除状态不进分享包）+ icon.<ext>（若有图标）。
pub fn export_profile(id: &str, out_path: &Path) -> Result<(), String> {
    export_profile_in(&profiles_dir(), id, out_path)
}

pub fn export_profile_in(dir: &Path, id: &str, out_path: &Path) -> Result<(), String> {
    use std::io::Write as _;

    let mut profile = get_in(dir, id).ok_or_else(|| format!("profile 不存在：{id}"))?;
    profile.removed_builtin_hotwords = Vec::new();

    let file = std::fs::File::create(out_path).map_err(|e| format!("创建导出文件失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let json =
        serde_json::to_string_pretty(&profile).map_err(|e| format!("序列化 profile 失败: {e}"))?;
    zip.start_file("profile.json", opts)
        .map_err(|e| format!("写入 profile.json 失败: {e}"))?;
    zip.write_all(json.as_bytes())
        .map_err(|e| format!("写入 profile.json 失败: {e}"))?;

    if let Some(name) = &profile.icon {
        if let Some(bytes) = read_profile_icon_in(dir, name) {
            let ext = Path::new(name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("png");
            let entry = format!("icon.{ext}");
            zip.start_file(&entry, opts)
                .map_err(|e| format!("写入 {entry} 失败: {e}"))?;
            zip.write_all(&bytes)
                .map_err(|e| format!("写入 {entry} 失败: {e}"))?;
        }
    }
    zip.finish().map_err(|e| format!("收尾导出文件失败: {e}"))?;
    Ok(())
}

fn read_zip_entry_limited<R: std::io::Read>(
    reader: R,
    limit: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Read as _;

    let mut bytes = Vec::with_capacity(limit.min(64 * 1024) as usize);
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取 {label} 失败: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label} 解压后超过 {}KB", limit / 1024));
    }
    Ok(bytes)
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2usize;
    while i + 1 < bytes.len() {
        if bytes[i] != 0xff {
            i += 1;
            continue;
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        let marker = *bytes.get(i)?;
        i += 1;
        if marker == 0xd8 || marker == 0xd9 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length = u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]) as usize;
        if length < 2 || i.checked_add(length)? > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        ) {
            if length < 7 {
                return None;
            }
            let height = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            return Some((width, height));
        }
        i += length;
    }
    None
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8X" => {
            let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            Some((width, height))
        }
        b"VP8L" if bytes[20] == 0x2f => {
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8 " if bytes[23..26] == [0x9d, 0x01, 0x2a] => Some((
            u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3fff,
            u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3fff,
        )),
        _ => None,
    }
}

/// 校验实际图片格式与扩展名，并只读取轻量文件头得到像素尺寸。浏览器最终仍
/// 负责解码，但畸形文件不会被作为 profile 图标持久化。
fn validate_profile_icon(entry_name: &str, bytes: &[u8]) -> Result<&'static str, String> {
    let extension = Path::new(entry_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "图标缺少扩展名".to_string())?;
    let (actual_extension, dimensions) = if bytes.len() >= 24
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        && &bytes[12..16] == b"IHDR"
    {
        (
            "png",
            Some((
                u32::from_be_bytes(bytes[16..20].try_into().ok().unwrap()),
                u32::from_be_bytes(bytes[20..24].try_into().ok().unwrap()),
            )),
        )
    } else if let Some(dimensions) = jpeg_dimensions(bytes) {
        ("jpg", Some(dimensions))
    } else if let Some(dimensions) = webp_dimensions(bytes) {
        ("webp", Some(dimensions))
    } else {
        return Err("图标必须是有效的 PNG、JPEG 或 WebP 图片".into());
    };
    let extension_matches = match actual_extension {
        "jpg" => extension == "jpg" || extension == "jpeg",
        other => extension == other,
    };
    if !extension_matches {
        return Err(format!(
            "图标扩展名 .{extension} 与实际 {actual_extension} 格式不符"
        ));
    }
    let (width, height) = dimensions.expect("已识别的图片格式必须包含尺寸");
    if width == 0
        || height == 0
        || width > MAX_PROFILE_ICON_DIMENSION
        || height > MAX_PROFILE_ICON_DIMENSION
        || u64::from(width) * u64::from(height)
            > u64::from(MAX_PROFILE_ICON_DIMENSION) * u64::from(MAX_PROFILE_ICON_DIMENSION)
    {
        return Err(format!(
            "图标尺寸必须在 1x1 到 {MAX_PROFILE_ICON_DIMENSION}x{MAX_PROFILE_ICON_DIMENSION} 之间，实际 {width}x{height}"
        ));
    }
    Ok(actual_extension)
}

/// 导入 .kprofile ZIP 包：解析 profile.json，**生成新随机 id**（用包名
/// displayName 区分，永不与内置/既有 profile 冲突），icon.* 条目写入
/// icons 目录。返回导入后的 profile。
pub fn import_profile(zip_path: &Path) -> Result<GameProfile, String> {
    import_profile_in(&profiles_dir(), zip_path)
}

pub fn import_profile_in(dir: &Path, zip_path: &Path) -> Result<GameProfile, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开 profile 包失败: {e}"))?;
    let package_size = file
        .metadata()
        .map_err(|e| format!("读取 profile 包元数据失败: {e}"))?
        .len();
    if package_size > MAX_PROFILE_PACKAGE_BYTES {
        return Err(format!(
            "profile 包不能超过 {}MB",
            MAX_PROFILE_PACKAGE_BYTES / 1024 / 1024
        ));
    }
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("不是合法的 .kprofile 包: {e}"))?;

    if archive.len() > MAX_PROFILE_ARCHIVE_ENTRIES {
        return Err(format!(
            "profile 包条目不能超过 {MAX_PROFILE_ARCHIVE_ENTRIES} 个"
        ));
    }
    let mut profile_entries = 0usize;
    let mut icon_entry = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("读取 profile 包目录失败: {e}"))?;
        let name = entry.name().to_string();
        if entry.is_dir() || entry.enclosed_name().is_none() || name.contains(['/', '\\']) {
            return Err(format!("profile 包含非法条目：{name}"));
        }
        match name.as_str() {
            "profile.json" => {
                profile_entries += 1;
                if entry.size() > MAX_PROFILE_JSON_BYTES {
                    return Err(format!(
                        "profile.json 解压后不能超过 {}KB",
                        MAX_PROFILE_JSON_BYTES / 1024
                    ));
                }
            }
            _ if name.starts_with("icon.") => {
                if icon_entry.replace(name.clone()).is_some() {
                    return Err("profile 包最多只能包含一个图标".into());
                }
                if entry.size() > MAX_PROFILE_ICON_BYTES {
                    return Err(format!(
                        "profile 图标解压后不能超过 {}KB",
                        MAX_PROFILE_ICON_BYTES / 1024
                    ));
                }
            }
            _ => return Err(format!("profile 包包含不支持的条目：{name}")),
        }
    }
    if profile_entries != 1 {
        return Err("profile 包必须且只能包含一个 profile.json".into());
    }

    let json_bytes = read_zip_entry_limited(
        archive
            .by_name("profile.json")
            .map_err(|_| "profile 包缺少 profile.json".to_string())?,
        MAX_PROFILE_JSON_BYTES,
        "profile.json",
    )?;
    let json_str =
        String::from_utf8(json_bytes).map_err(|_| "profile.json 必须是 UTF-8 文本".to_string())?;
    let mut profile: GameProfile = serde_json::from_str(&json_str)
        .map_err(|e| format!("解析 profile.json 失败（缺必需字段或格式错误）: {e}"))?;

    // 新随机 id + 清本地删除状态
    profile.id = new_profile_id();
    profile.removed_builtin_hotwords = Vec::new();
    // 包内 JSON 的 icon 文件名不可信；只有实际随包且验证通过的图片才会写回。
    profile.icon = None;

    // icon.* 条目（若有）→ 写入 icons 目录，文件名用新 id
    if let Some(entry_name) = icon_entry {
        let buf = read_zip_entry_limited(
            archive
                .by_name(&entry_name)
                .map_err(|e| format!("读取 {entry_name} 失败: {e}"))?,
            MAX_PROFILE_ICON_BYTES,
            &entry_name,
        )?;
        let ext = validate_profile_icon(&entry_name, &buf)?;
        let icon_name = format!("{}.{}", profile.id, ext);
        profile.icon = Some(icon_name.clone());
        validate_profile(&profile)?;
        write_profile_icon_in(dir, &icon_name, &buf)?;
        if let Err(error) = save_in(dir, &profile) {
            let _ = std::fs::remove_file(icons_dir_in(dir).join(&icon_name));
            return Err(error);
        }
        return Ok(profile);
    }

    save_in(dir, &profile)?;
    Ok(profile)
}

/// 导入用新 id：时间戳（复用 eval::new_session_id 的 utc 方案）+ 进程内
/// 自增计数——时间戳保证跨进程唯一，计数保证同进程同毫秒多次导入不撞。
/// 不引入 uuid/rand 依赖。
fn new_profile_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("p{}-{n}", crate::eval::new_session_id())
}

/// 删除结果
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProfileDeleteOutcome {
    /// 内置 profile：恢复出厂（用 bundled 覆盖磁盘文件并重写 icon）
    Reset,
    /// 导入的自定义 profile：永久删除
    Deleted,
}

/// 删除 profile：内置 = 恢复出厂；导入的 = 删文件与 icon
pub fn delete_profile(id: &str) -> Result<ProfileDeleteOutcome, String> {
    delete_profile_in(&profiles_dir(), id)
}

pub fn delete_profile_in(dir: &Path, id: &str) -> Result<ProfileDeleteOutcome, String> {
    let builtin = match id {
        "lol" => Some(GameProfile::builtin_lol()),
        "generic" => Some(GameProfile::builtin_generic()),
        _ => None,
    };
    if let Some(builtin) = builtin {
        // 恢复出厂：覆盖磁盘文件（清空 removed_builtin_hotwords 等本地改动）+ 物化 icon
        save_in(dir, &builtin)?;
        if let Some((bytes, name)) = builtin_icon(&builtin.id) {
            write_profile_icon_in(dir, name, bytes)?;
        }
        Ok(ProfileDeleteOutcome::Reset)
    } else {
        let path = profile_path_in(dir, id);
        if !path.exists() {
            return Err(format!("profile 不存在：{id}"));
        }
        let icon = get_in(dir, id).and_then(|p| p.icon);
        std::fs::remove_file(&path).map_err(|e| format!("删除 profile 失败: {e}"))?;
        if let Some(icon) = icon {
            if let Some(safe) = sanitize_icon_name(&icon) {
                let _ = std::fs::remove_file(icons_dir_in(dir).join(safe));
            }
        }
        Ok(ProfileDeleteOutcome::Deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profiles() -> Vec<GameProfile> {
        vec![GameProfile::builtin_generic(), GameProfile::builtin_lol()]
    }

    #[test]
    fn lol_matches_its_process_case_insensitive() {
        let lol = GameProfile::builtin_lol();
        assert!(matches_process(&lol, "League of Legends.exe"));
        assert!(matches_process(&lol, "league of legends.exe"));
        assert!(matches_process(&lol, "LEAGUE OF LEGENDS.EXE"));
    }

    #[test]
    fn lol_does_not_match_other_processes() {
        let lol = GameProfile::builtin_lol();
        assert!(!matches_process(&lol, "notepad.exe"));
        assert!(!matches_process(&lol, "LeagueClient.exe"));
    }

    #[test]
    fn generic_matches_anything() {
        let generic = GameProfile::builtin_generic();
        assert!(matches_process(&generic, "notepad.exe"));
        assert!(matches_process(&generic, "anything at all"));
    }

    #[test]
    fn find_prefers_specific_over_generic() {
        let profiles = sample_profiles();
        let hit = find_by_process(&profiles, "league of legends.exe").unwrap();
        assert_eq!(hit.id, "lol", "LOL 进程应命中具体 profile 而非 generic");
    }

    #[test]
    fn find_falls_back_to_generic() {
        let profiles = sample_profiles();
        let hit = find_by_process(&profiles, "notepad.exe").unwrap();
        assert_eq!(hit.id, "generic");
    }

    #[test]
    fn find_returns_none_without_generic() {
        let profiles = vec![GameProfile::builtin_lol()];
        assert!(find_by_process(&profiles, "notepad.exe").is_none());
    }

    #[test]
    fn builtin_values_match_doc() {
        let lol = GameProfile::builtin_lol();
        assert_eq!(lol.pre_open_delay_ms, 20);
        assert_eq!(lol.pre_paste_delay_ms, 20);
        assert_eq!(lol.pre_send_delay_ms, 20);
        assert!(!lol.prefer_clipboard_paste);
        assert_eq!(lol.open_chat_key, "Enter");
        assert_eq!(lol.send_key, "Enter");
        assert!(lol.hotwords.contains(&"打野".to_string()));
        // 内置热词应去重
        let mut sorted = lol.hotwords.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), lol.hotwords.len(), "内置热词不应有重复");
    }

    #[test]
    fn crud_roundtrip_in_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();

        ensure_builtin_in(&dir).unwrap();
        let mut all = list_in(&dir);
        assert_eq!(all.len(), 2, "首次运行落盘 lol + generic");
        // 再跑一遍不重复、不覆盖
        ensure_builtin_in(&dir).unwrap();
        all = list_in(&dir);
        assert_eq!(all.len(), 2);

        let mut custom = GameProfile::builtin_lol();
        custom.id = "valorant".into();
        custom.display_name = "Valorant".into();
        custom.process_names = vec!["VALORANT-Win64-Shipping.exe".into()];
        save_in(&dir, &custom).unwrap();

        let loaded = get_in(&dir, "valorant").unwrap();
        assert_eq!(loaded.display_name, "Valorant");
        assert_eq!(list_in(&dir).len(), 3);
    }

    #[test]
    fn profile_json_uses_camel_case() {
        let lol = GameProfile::builtin_lol();
        let v = serde_json::to_value(&lol).unwrap();
        assert!(v.get("displayName").is_some());
        assert!(v.get("processNames").is_some());
        assert!(v.get("preferClipboardPaste").is_some());
        assert!(v.get("preOpenDelayMs").is_some());
        // 空删除表不序列化；非空时按 camelCase 落盘
        assert!(v.get("removedBuiltinHotwords").is_none());
        let mut lol2 = GameProfile::builtin_lol();
        lol2.removed_builtin_hotwords = vec!["提莫".into()];
        let v2 = serde_json::to_value(&lol2).unwrap();
        assert_eq!(v2["removedBuiltinHotwords"][0], "提莫");
    }

    // ---------- 内置热词版本更新合并 ----------

    /// 模拟一份「旧版本」lol 文件：内置词条的子集 + 一个用户自定义词条
    fn write_legacy_lol(dir: &Path, hotwords: &[&str], removed: &[&str]) {
        let mut legacy = GameProfile::builtin_lol();
        legacy.hotwords = hotwords.iter().map(|s| s.to_string()).collect();
        legacy.removed_builtin_hotwords = removed.iter().map(|s| s.to_string()).collect();
        save_in(dir, &legacy).unwrap();
    }

    #[test]
    fn ensure_builtin_merges_new_hotwords_into_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        write_legacy_lol(&dir, &["闪现"], &[]);

        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        // 新版内置词条全部补齐，追加在末尾（旧词条「闪现」保持在最前）
        assert_eq!(lol.hotwords.first().map(String::as_str), Some("闪现"));
        for w in &GameProfile::builtin_lol().hotwords {
            assert!(lol.hotwords.contains(w), "合并后缺少内置词条 {w}");
        }
        // 再跑一遍幂等：数量稳定不重复追加
        let n = lol.hotwords.len();
        ensure_builtin_in(&dir).unwrap();
        assert_eq!(get_in(&dir, "lol").unwrap().hotwords.len(), n);
    }

    #[test]
    fn ensure_builtin_respects_user_removed_hotwords() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        // 用户删掉了「提莫」「悠米」（记进 removedBuiltinHotwords）
        write_legacy_lol(&dir, &["闪现"], &["提莫", "悠米"]);

        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        assert!(
            !lol.hotwords.contains(&"提莫".to_string()),
            "删除的词条不应复活"
        );
        assert!(!lol.hotwords.contains(&"悠米".to_string()));
        assert!(
            lol.hotwords.contains(&"亚索".to_string()),
            "其余新词条照常合并"
        );
    }

    #[test]
    fn ensure_builtin_keeps_user_custom_hotwords() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        write_legacy_lol(&dir, &["闪现", "我的自定义词"], &[]);

        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        assert!(
            lol.hotwords.contains(&"我的自定义词".to_string()),
            "用户自定义词条应保留"
        );
        // 其他字段不被合并改动
        assert_eq!(lol.open_chat_key, "Enter");
        assert_eq!(lol.pre_send_delay_ms, 20);
    }

    // ---------- 热词导入导出 ----------

    #[test]
    fn hotwords_export_one_per_line() {
        let words = vec![
            "打野".to_string(),
            "gank".to_string(),
            "Blind Monk".to_string(),
        ];
        assert_eq!(format_hotwords_export(&words), "打野\ngank\nBlind Monk\n");
        assert_eq!(format_hotwords_export(&[]), "", "空表导出空文件");
    }

    #[test]
    fn hotwords_import_skips_empty_and_inner_duplicates() {
        let text = "打野\n\n  \ngank\n打野\n  推塔  \r\n";
        assert_eq!(parse_hotwords_import(text), vec!["打野", "gank", "推塔"]);
        assert!(parse_hotwords_import("").is_empty());
        assert!(parse_hotwords_import("  \n\n").is_empty());
    }

    #[test]
    fn hotwords_merge_dedupes_against_existing() {
        let existing = vec!["打野".to_string(), "gank".to_string()];
        let incoming = vec!["gank".to_string(), "推塔".to_string(), "回城".to_string()];
        let (merged, report) = merge_hotwords(&existing, &incoming);
        assert_eq!(merged, vec!["打野", "gank", "推塔", "回城"]);
        assert_eq!(
            report,
            HotwordMergeReport {
                added: 2,
                duplicates: 1,
                total: 4
            }
        );
        // 全重复：零新增，现有顺序不变
        let (m2, r2) = merge_hotwords(&existing, &["打野".to_string()]);
        assert_eq!(m2, existing);
        assert_eq!(r2.added, 0);
        assert_eq!(r2.duplicates, 1);
    }

    #[test]
    fn hotwords_export_import_roundtrip() {
        let words = GameProfile::builtin_lol().hotwords;
        let text = format_hotwords_export(&words);
        assert_eq!(parse_hotwords_import(&text), words);
    }

    // ---------- 聊天频道（ADR-008） ----------

    #[test]
    fn channels_effective_empty_channels_synthesizes_default() {
        let g = GameProfile::builtin_generic();
        let ch = g.channels_effective();
        assert_eq!(ch.len(), 1);
        assert!(ch[0].is_default);
        assert_eq!(ch[0].open_chat_key, g.open_chat_key);
        assert_eq!(ch[0].text_prefix, "");
        assert_eq!(g.next_channel_id(None), None, "单频道游戏不可切换");
    }

    #[test]
    fn lol_builtin_declares_team_and_all_channels() {
        let lol = GameProfile::builtin_lol();
        let ch = lol.channels_effective();
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[0].id, "team");
        assert!(ch[0].is_default);
        assert_eq!(ch[0].open_chat_key, "Enter", "默认频道沿用 profile 开框键");
        assert_eq!(ch[1].id, "all");
        assert_eq!(ch[1].display_name, "所有人");
        assert_eq!(ch[1].open_chat_key, "Shift+Enter");
    }

    #[test]
    fn channel_strategy_falls_back_to_default() {
        let lol = GameProfile::builtin_lol();
        assert_eq!(lol.channel_strategy(None).id, "team");
        assert_eq!(
            lol.channel_strategy(Some("ghost")).id,
            "team",
            "未知 id 回落默认"
        );
        assert_eq!(
            lol.channel_strategy(Some("all")).open_chat_key,
            "Shift+Enter"
        );
    }

    #[test]
    fn next_channel_cycles_in_declaration_order() {
        let lol = GameProfile::builtin_lol();
        // 无当前频道（= 位于默认）→ 默认的下一个
        assert_eq!(lol.next_channel_id(None).as_deref(), Some("all"));
        assert_eq!(lol.next_channel_id(Some("all")).as_deref(), Some("team"));
        assert_eq!(lol.next_channel_id(Some("team")).as_deref(), Some("all"));
        // 未知 id（切 profile 后的残留）→ 默认的下一个
        assert_eq!(lol.next_channel_id(Some("ghost")).as_deref(), Some("all"));
    }

    #[test]
    fn channel_prefix_strategy_uses_default_open_key() {
        // 前缀策略：未声明 openChatKey 的频道沿用 profile 默认开框键
        let mut p = GameProfile::builtin_generic();
        p.channels = vec![
            ProfileChannel {
                id: "team".into(),
                display_name: "队伍".into(),
                open_chat_key: None,
                text_prefix: None,
                default: true,
            },
            ProfileChannel {
                id: "all".into(),
                display_name: "所有人".into(),
                open_chat_key: None,
                text_prefix: Some("/all ".into()),
                default: false,
            },
        ];
        let s = p.channel_strategy(Some("all"));
        assert_eq!(s.open_chat_key, p.open_chat_key);
        assert_eq!(s.text_prefix, "/all ");
        assert!(!s.is_default);
    }

    #[test]
    fn ensure_builtin_backfills_channels_into_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        // 真·旧版文件：无 channels（序列化时整个字段被跳过）
        let mut legacy = GameProfile::builtin_lol();
        legacy.channels.clear();
        save_in(&dir, &legacy).unwrap();
        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        assert_eq!(lol.channels.len(), 2, "缺 channels 的旧文件应补入内置频道");
        // 幂等 + 尊重用户后续编辑：已有频道不被覆盖
        let mut edited = lol.clone();
        edited.channels[1].display_name = "全体（自定义）".into();
        save_in(&dir, &edited).unwrap();
        ensure_builtin_in(&dir).unwrap();
        assert_eq!(
            get_in(&dir, "lol").unwrap().channels[1].display_name,
            "全体（自定义）"
        );
    }

    // ---------- 整包导入导出（.kprofile ZIP） ----------

    #[test]
    fn export_import_roundtrip_in_temp_dir() {
        use std::io::Read as _;

        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        let pkg = dir.join("lol.kprofile");

        // 源 profile：带 icon + removed_builtin_hotwords（导出应剥离本地删除状态）
        let mut src = GameProfile::builtin_lol();
        src.removed_builtin_hotwords = vec!["提莫".into()];
        save_in(&dir, &src).unwrap();
        let icon = builtin_icon("lol").unwrap();
        write_profile_icon_in(&dir, icon.1, icon.0).unwrap();

        export_profile_in(&dir, "lol", &pkg).unwrap();

        // 包内 profile.json 不应含 removedBuiltinHotwords，但含 icon
        let mut zipfile = std::fs::File::open(&pkg).unwrap();
        let mut archive = zip::ZipArchive::new(&mut zipfile).unwrap();
        let mut json_str = String::new();
        archive
            .by_name("profile.json")
            .unwrap()
            .read_to_string(&mut json_str)
            .unwrap();
        assert!(
            !json_str.contains("removedBuiltinHotwords"),
            "导出剥离用户本地删除状态"
        );
        assert!(json_str.contains("\"icon\": \"lol-kotone.webp\""));
        let icon_entry = archive
            .file_names()
            .find(|n| n.starts_with("icon."))
            .unwrap()
            .to_string();
        let mut icon_bytes = Vec::new();
        archive
            .by_name(&icon_entry)
            .unwrap()
            .read_to_end(&mut icon_bytes)
            .unwrap();
        assert_eq!(icon_bytes, icon.0, "包内 icon 字节与源一致");

        // 导入到空目录 → 新 id、内容保留、icon 落盘
        let dir2 = tempfile::tempdir().unwrap();
        let dir2 = dir2.path().to_path_buf();
        let imported = import_profile_in(&dir2, &pkg).unwrap();
        assert_ne!(imported.id, "lol", "导入生成新 id");
        assert!(imported.id.starts_with('p'));
        assert_eq!(imported.display_name, src.display_name);
        assert_eq!(imported.process_names, src.process_names);
        assert_eq!(imported.hotwords, src.hotwords);
        assert!(imported.removed_builtin_hotwords.is_empty());
        let icon_name = imported.icon.as_deref().unwrap();
        assert_eq!(read_profile_icon_in(&dir2, icon_name).unwrap(), icon.0);
        assert!(get_in(&dir2, &imported.id).is_some(), "导入后落盘可读回");
    }

    #[test]
    fn import_assigns_fresh_id_twice() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        save_in(&dir, &GameProfile::builtin_lol()).unwrap();
        let pkg = dir.join("lol.kprofile");
        export_profile_in(&dir, "lol", &pkg).unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let dir2 = dir2.path().to_path_buf();
        let a = import_profile_in(&dir2, &pkg).unwrap();
        let b = import_profile_in(&dir2, &pkg).unwrap();
        assert_ne!(a.id, b.id, "同进程连续两次导入 id 也不同（计数兜底）");
        assert_ne!(a.id, "lol");
        assert_ne!(b.id, "lol");
    }

    #[test]
    fn import_rejects_malformed_package() {
        use std::io::Write as _;

        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        // 不是 zip
        let not_zip = dir.join("bad.kprofile");
        std::fs::write(&not_zip, b"not a zip").unwrap();
        assert!(import_profile_in(&dir, &not_zip).is_err());
        // 是 zip 但缺 profile.json
        let no_json = dir.join("nojson.kprofile");
        let f = std::fs::File::create(&no_json).unwrap();
        let mut z = zip::ZipWriter::new(f);
        z.start_file("something.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        assert!(import_profile_in(&dir, &no_json).is_err());
    }

    fn write_test_package(path: &Path, profile: &GameProfile, extra: &[(&str, &[u8])]) {
        use std::io::Write as _;

        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("profile.json", options).unwrap();
        zip.write_all(serde_json::to_string(profile).unwrap().as_bytes())
            .unwrap();
        for (name, bytes) in extra {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn save_rejects_unbounded_or_invalid_profile_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut profile = GameProfile::builtin_generic();

        profile.pre_send_delay_ms = MAX_PROFILE_DELAY_MS + 1;
        assert!(save_in(dir.path(), &profile)
            .unwrap_err()
            .contains("发送前延时"));

        profile.pre_send_delay_ms = 20;
        profile.open_chat_key = "Ctrl+NotAKey".into();
        assert!(save_in(dir.path(), &profile)
            .unwrap_err()
            .contains("按键格式无效"));

        profile.open_chat_key = "Enter".into();
        profile.hotwords = (0..=MAX_HOTWORDS).map(|i| format!("word-{i}")).collect();
        assert!(save_in(dir.path(), &profile)
            .unwrap_err()
            .contains("不能超过"));

        profile.hotwords.clear();
        profile.display_name = "x".repeat(129);
        assert!(save_in(dir.path(), &profile)
            .unwrap_err()
            .contains("显示名"));

        profile.display_name = "generic".into();
        profile.channels = (0..=MAX_PROFILE_CHANNELS)
            .map(|i| ProfileChannel {
                id: format!("channel-{i}"),
                display_name: format!("Channel {i}"),
                open_chat_key: None,
                text_prefix: None,
                default: false,
            })
            .collect();
        assert!(save_in(dir.path(), &profile)
            .unwrap_err()
            .contains("频道不能超过"));
    }

    #[test]
    fn hotword_file_import_enforces_size_count_and_word_length() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &GameProfile::builtin_generic()).unwrap();

        let too_large = dir.path().join("too-large.txt");
        std::fs::write(&too_large, vec![b'x'; MAX_HOTWORDS_FILE_BYTES as usize + 1]).unwrap();
        assert!(
            import_hotwords_from_file_in(dir.path(), "generic", &too_large)
                .unwrap_err()
                .contains("不能超过")
        );

        let too_many = (0..=MAX_HOTWORDS)
            .map(|i| format!("word-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_hotwords_import_checked(&too_many)
            .unwrap_err()
            .contains("不能超过"));
        assert!(
            parse_hotwords_import_checked(&"x".repeat(MAX_HOTWORD_CHARS + 1))
                .unwrap_err()
                .contains("不能超过")
        );
    }

    #[test]
    fn import_rejects_too_many_archive_entries_and_multiple_icons() {
        let dir = tempfile::tempdir().unwrap();
        let profile = GameProfile::builtin_generic();

        let too_many = dir.path().join("too-many.kprofile");
        write_test_package(
            &too_many,
            &profile,
            &[("a", b"x"), ("b", b"x"), ("c", b"x"), ("d", b"x")],
        );
        assert!(import_profile_in(dir.path(), &too_many)
            .unwrap_err()
            .contains("条目不能超过"));

        let two_icons = dir.path().join("two-icons.kprofile");
        write_test_package(
            &two_icons,
            &profile,
            &[("icon.png", b"bad"), ("icon.webp", b"bad")],
        );
        assert!(import_profile_in(dir.path(), &two_icons)
            .unwrap_err()
            .contains("一个图标"));
    }

    #[test]
    fn import_rejects_compressed_oversized_json_entry() {
        use std::io::Write as _;

        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("json-bomb.kprofile");
        let file = std::fs::File::create(&package).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("profile.json", options).unwrap();
        zip.write_all(&vec![b' '; MAX_PROFILE_JSON_BYTES as usize + 1])
            .unwrap();
        zip.finish().unwrap();

        assert!(std::fs::metadata(&package).unwrap().len() < MAX_PROFILE_PACKAGE_BYTES);
        assert!(import_profile_in(dir.path(), &package)
            .unwrap_err()
            .contains("解压后"));
    }

    #[test]
    fn import_validates_icon_extension_format_and_dimensions() {
        // 足够供轻量头校验使用的 1x1 PNG IHDR。
        let mut png = vec![0u8; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&1u32.to_be_bytes());
        png[20..24].copy_from_slice(&1u32.to_be_bytes());

        assert_eq!(validate_profile_icon("icon.png", &png), Ok("png"));
        assert!(validate_profile_icon("icon.jpg", &png)
            .unwrap_err()
            .contains("扩展名"));

        png[16..20].copy_from_slice(&(MAX_PROFILE_ICON_DIMENSION + 1).to_be_bytes());
        assert!(validate_profile_icon("icon.png", &png)
            .unwrap_err()
            .contains("尺寸"));
        assert!(validate_profile_icon("icon.exe", b"not an image").is_err());
    }

    #[test]
    fn import_ignores_json_icon_without_verified_archive_entry() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("stale-icon.kprofile");
        let mut profile = GameProfile::builtin_generic();
        profile.icon = Some("missing.png".into());
        write_test_package(&package, &profile, &[]);

        let imported = import_profile_in(dir.path(), &package).unwrap();
        assert!(imported.icon.is_none());
    }

    #[test]
    fn icon_write_read_roundtrip_and_traversal_guard() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        write_profile_icon_in(&dir, "lol-kotone.webp", b"WEBPBYTES").unwrap();
        assert_eq!(
            read_profile_icon_in(&dir, "lol-kotone.webp").unwrap(),
            b"WEBPBYTES"
        );
        assert!(write_profile_icon_in(&dir, "../evil.png", b"x").is_err());
        assert!(write_profile_icon_in(&dir, "a/b.png", b"x").is_err());
        assert!(write_profile_icon_in(&dir, "", b"x").is_err());
        assert!(read_profile_icon_in(&dir, "../evil.png").is_none());
    }

    #[test]
    fn delete_removes_imported_profile_and_icon() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        save_in(&dir, &GameProfile::builtin_lol()).unwrap();
        let (bytes, name) = builtin_icon("lol").unwrap();
        write_profile_icon_in(&dir, name, bytes).unwrap();
        let pkg = dir.join("lol.kprofile");
        export_profile_in(&dir, "lol", &pkg).unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let dir2 = dir2.path().to_path_buf();
        let imported = import_profile_in(&dir2, &pkg).unwrap();
        let icon = imported.icon.clone().unwrap();
        assert!(get_in(&dir2, &imported.id).is_some());
        let outcome = delete_profile_in(&dir2, &imported.id).unwrap();
        assert_eq!(outcome, ProfileDeleteOutcome::Deleted);
        assert!(get_in(&dir2, &imported.id).is_none());
        assert!(
            read_profile_icon_in(&dir2, &icon).is_none(),
            "icon 一并删除"
        );
    }

    #[test]
    fn reset_builtin_restores_factory() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        save_in(&dir, &GameProfile::builtin_lol()).unwrap();
        // 用户改坏：删热词、清 icon、留删除标记
        let mut corrupted = GameProfile::builtin_lol();
        corrupted.hotwords = vec!["闪现".into()];
        corrupted.removed_builtin_hotwords = vec!["ADC".into()];
        corrupted.icon = None;
        save_in(&dir, &corrupted).unwrap();
        let outcome = delete_profile_in(&dir, "lol").unwrap();
        assert_eq!(outcome, ProfileDeleteOutcome::Reset);
        let restored = get_in(&dir, "lol").unwrap();
        assert_eq!(restored.hotwords, GameProfile::builtin_lol().hotwords);
        assert!(restored.removed_builtin_hotwords.is_empty());
        assert_eq!(restored.icon, GameProfile::builtin_lol().icon);
        assert!(
            read_profile_icon_in(&dir, "lol-kotone.webp").is_some(),
            "恢复出厂重新物化 icon"
        );
    }

    #[test]
    fn ensure_builtin_materializes_icons_and_backfills_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().to_path_buf();
        // 老文件：无 icon 字段
        let mut legacy = GameProfile::builtin_lol();
        legacy.icon = None;
        save_in(&dir, &legacy).unwrap();
        ensure_builtin_in(&dir).unwrap();
        let lol = get_in(&dir, "lol").unwrap();
        assert_eq!(
            lol.icon.as_deref(),
            Some("lol-kotone.webp"),
            "升级补 icon 字段"
        );
        assert!(
            read_profile_icon_in(&dir, "lol-kotone.webp").is_some(),
            "内置 icon 已物化"
        );
    }

    #[test]
    fn builtin_lol_has_icon_field() {
        let lol = GameProfile::builtin_lol();
        assert_eq!(lol.icon.as_deref(), Some("lol-kotone.webp"));
        assert!(builtin_icon("lol").is_some());
        assert!(builtin_icon("generic").is_none());
    }
}
