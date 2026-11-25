//! 热键表的存法（ADR 040 D8；第 2 期第 1 步：格式 + 迁移，登记和界面还读旧字段）。
//!
//! config.json 里一个 `hotkeys.bindings` 列表，每条一个 JSON 对象：`key` 加上三选一——`command`
//! （`<来源>.<命令 id>`，本体的是 `kotone.*`）、`tool`（+ `args`，第 3 期）、`page`（`ui://…`，第 3 期）。
//! 照 VS Code 的 `keybindings.json`（key + command + args）。**不存“按一下 / 按住”**：由命令自己声明
//! （ADR 040 实施记录里对 D8 的修订）。
//!
//! 规矩（原则 19，读旧写新、不覆盖新字段）：
//! - **有 `hotkeys.bindings`（数组，哪怕是空的）就以它为准**；没有就从旧字段造（[`migrate_hotkeys_json`]，
//!   旧字段没写时按旧版本的默认，例如 ADR 008 以前的配置照旧有 Shift+CapsLock 切换频道）。
//! - **旧字段每次保存都照表写全**（[`Settings::sync_hotkey_mirrors`]），没绑的写空串：装回旧版本键都在，
//!   旧版本也不会用它自己的默认补回去。旧版本保存时会丢掉 `hotkeys`，再装回新版本就从旧字段重新造。
//! - **旧写法照样能改**：`settings.update`、命令行 `config set`、还没改的界面改了旧字段，就改表里对应的那条
//!   （[`Settings::reconcile_hotkeys`]，在 `SettingsRepository` 的每次修改里调）；补丁同时改了 `hotkeys` 时以它为准。
//!   代码直接改了旧字段就 `save_to` 的，保存时也照旧写法算进表（[`with_direct_edits`]）。
//! - **读不懂的条目原样留着**（将来的命令、`tool` / `page`、多出来的字段、甚至不是对象的值），保存时照写回去。
//!
//! 旧字段在内存里也照留（只读的镜像）：登记、界面、命令行这一步还读它们，第 2 期后面几步再换成读表。

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::assistant::{CompanionBindings, DEFAULT_CALL_HOTKEY};
use crate::settings::{LegacyUpgrade, Settings};

/// 呼叫 / 挂断（呼叫键）。
pub const CMD_CALL: &str = "kotone.call";
/// 停下（停下热键）。
pub const CMD_STOP: &str = "kotone.stop";
/// 启动 / 停止本地语音识别。
pub const CMD_SPEECH_TOGGLE: &str = "kotone.speech-toggle";
/// 语音输入：说话（语音输入热键）。
pub const CMD_TALK: &str = "voice-input.talk";
/// 语音输入：切换频道。
pub const CMD_CHANNEL_CYCLE: &str = "voice-input.channel-cycle";
/// 语音输入：重发上一句。
pub const CMD_RESEND_LAST: &str = "voice-input.resend-last";
/// 语音输入：开关文字处理。
pub const CMD_PP_TOGGLE: &str = "voice-input.post-process-toggle";
/// 语音输入：切换文字处理方案。
pub const CMD_PP_CYCLE: &str = "voice-input.post-process-cycle";

/// 一个有旧字段的命令：命令名、第 1 期起热键表 / `hotkeys.status` 用的 id、旧版本缺这个字段时的默认。
/// 迁移、镜像、旧写法都查这一张（顺序 = 热键表里的顺序）。提醒助手热键（ADR 040 D10）和快捷指令
/// （第 3 期，D5）不在这里：它们照旧存在原来的地方。
pub struct Legacy {
    pub command: &'static str,
    pub id: &'static str,
    pub old_default: &'static str,
}

pub const LEGACY: &[Legacy] = &[
    Legacy {
        command: CMD_CALL,
        id: "assistant",
        old_default: DEFAULT_CALL_HOTKEY,
    },
    Legacy {
        command: CMD_STOP,
        id: "automation-stop",
        old_default: "Ctrl+Shift+F12",
    },
    Legacy {
        command: CMD_SPEECH_TOGGLE,
        id: "runtime-toggle",
        old_default: "",
    },
    Legacy {
        command: CMD_TALK,
        id: "record",
        old_default: "CapsLock",
    },
    Legacy {
        command: CMD_CHANNEL_CYCLE,
        id: "channel-cycle",
        old_default: "Shift+CapsLock",
    },
    Legacy {
        command: CMD_RESEND_LAST,
        id: "resend-last",
        old_default: "",
    },
    Legacy {
        command: CMD_PP_TOGGLE,
        id: "post-process-toggle",
        old_default: "",
    },
    Legacy {
        command: CMD_PP_CYCLE,
        id: "post-process-cycle",
        old_default: "",
    },
];

/// 新装时表里的键（ADR 040 D4）：只有呼叫键、停下热键和语音输入的“说话”。
pub const DEFAULT_BINDINGS: &[(&str, &str)] = &[
    (CMD_CALL, DEFAULT_CALL_HOTKEY),
    (CMD_STOP, "Ctrl+Shift+F12"),
    (CMD_TALK, "CapsLock"),
];

/// 设置里的 `hotkeys`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeysConfig {
    #[serde(default)]
    pub bindings: Vec<Binding>,
    /// 将来加在 `hotkeys` 下面的东西：原样留着。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            bindings: DEFAULT_BINDINGS
                .iter()
                .map(|(command, key)| Binding::command(key, command))
                .collect(),
            extra: Map::new(),
        }
    }
}

/// 表里的一条。存的就是 JSON 原样（读不懂的也能原样写回去），用下面的方法读。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Binding(pub Value);

impl Binding {
    /// 键 → 命令。
    pub fn command(key: &str, command: &str) -> Self {
        Self(json!({"key": key, "command": command}))
    }

    fn str_field(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str)
    }

    /// 键（没有或不是字符串时为空）。
    pub fn key(&self) -> &str {
        self.str_field("key").unwrap_or("")
    }

    /// 绑的命令（`<来源>.<命令 id>`）；绑的是工具、页面或者读不懂时为 None。
    pub fn command_name(&self) -> Option<&str> {
        self.str_field("command")
    }

    /// 绑的工具（第 3 期）。
    pub fn tool(&self) -> Option<&str> {
        self.str_field("tool")
    }

    /// 调工具时的固定参数（第 3 期）。
    pub fn args(&self) -> Option<&Value> {
        self.0.get("args")
    }

    /// 打开的页面（第 3 期）。
    pub fn page(&self) -> Option<&str> {
        self.str_field("page")
    }

    fn set_key(&mut self, key: &str) {
        if let Some(object) = self.0.as_object_mut() {
            object.insert("key".into(), key.into());
        }
    }
}

impl Settings {
    /// 表里某个命令的键（这个命令的第一条；没绑为空）。
    pub fn binding_key(&self, command: &str) -> &str {
        self.hotkeys
            .bindings
            .iter()
            .find(|b| b.command_name() == Some(command))
            .map_or("", Binding::key)
    }

    /// 改表里某个命令的键：有就改它的第一条（别的条目、别的字段不动），没有就加一条；空 = 这个命令的条目都删掉。
    /// 旧字段的镜像要再 [`Settings::sync_hotkey_mirrors`]。
    pub fn set_binding(&mut self, command: &str, key: &str) {
        let key = key.trim();
        let bindings = &mut self.hotkeys.bindings;
        if key.is_empty() {
            bindings.retain(|b| b.command_name() != Some(command));
            return;
        }
        match bindings
            .iter_mut()
            .find(|b| b.command_name() == Some(command))
        {
            Some(binding) => binding.set_key(key),
            None => bindings.push(Binding::command(key, command)),
        }
    }

    /// 把表写回旧字段（只读的镜像）。没绑的写空串。呼叫键看是哪种用户：新用户写 `assistant.hotkey`；
    /// 0.1.x 升级后启用了她的写 `assistant.companionBindings.hotkey`（`assistant.hotkey` 是他的提醒键，不动）；
    /// 还没启用她的没有呼叫键，不写。
    pub fn sync_hotkey_mirrors(&mut self) {
        for legacy in LEGACY {
            let key = self.binding_key(legacy.command).to_string();
            set_mirror(self, legacy.command, key);
        }
    }

    /// 一次修改以后（`previous` 是改之前）：表没动、旧字段变了的，照旧字段改表（旧写法）；表动了的以表为准。
    /// 最后把旧字段照表写回。调两次和调一次一样。
    pub fn reconcile_hotkeys(&mut self, previous: &Settings) {
        if self.hotkeys == previous.hotkeys {
            for legacy in LEGACY {
                let changed = match (
                    mirror(previous, legacy.command),
                    mirror(self, legacy.command),
                ) {
                    (Some(before), Some(after)) if before != after => Some(after.to_string()),
                    _ => None,
                };
                if let Some(key) = changed {
                    self.set_binding(legacy.command, &key);
                }
            }
        }
        self.sync_hotkey_mirrors();
    }
}

/// 保存前：内存里的旧字段和表对不上，说明有代码直接改了旧字段、没经过 [`Settings::reconcile_hotkeys`]
/// （例如改完直接 `save_to`）。按旧写法算进表里再存，免得这次改动下次读的时候被表盖掉。对得上时返回 None。
pub(crate) fn with_direct_edits(settings: &Settings) -> Option<Settings> {
    let mut expected = settings.clone();
    expected.sync_hotkey_mirrors();
    if LEGACY
        .iter()
        .all(|l| mirror(settings, l.command) == mirror(&expected, l.command))
    {
        return None;
    }
    let mut fixed = settings.clone();
    fixed.reconcile_hotkeys(&expected);
    Some(fixed)
}

/// 这个命令在旧字段里的镜像（这种用户没有这个镜像时为 None）。
fn mirror<'a>(settings: &'a Settings, command: &str) -> Option<&'a str> {
    Some(match command {
        CMD_CALL => match settings.ui.legacy_upgrade {
            LegacyUpgrade::NotNeeded => settings.assistant.hotkey.as_str(),
            LegacyUpgrade::OptedIn => settings
                .assistant
                .companion_bindings
                .as_ref()
                .map_or("", |c| c.hotkey.as_str()),
            LegacyUpgrade::Pending | LegacyUpgrade::Dismissed => return None,
        },
        CMD_STOP => &settings.automation_stop_hotkey,
        CMD_SPEECH_TOGGLE => &settings.runtime_toggle_hotkey,
        CMD_TALK => &settings.hotkey.key,
        CMD_CHANNEL_CYCLE => &settings.channel_cycle_hotkey,
        CMD_RESEND_LAST => &settings.resend_last_hotkey,
        CMD_PP_TOGGLE => &settings.toggle_post_processing_hotkey,
        CMD_PP_CYCLE => &settings.cycle_post_processing_hotkey,
        _ => return None,
    })
}

fn set_mirror(settings: &mut Settings, command: &str, key: String) {
    match command {
        CMD_CALL => match settings.ui.legacy_upgrade {
            LegacyUpgrade::NotNeeded => settings.assistant.hotkey = key,
            LegacyUpgrade::OptedIn => match settings.assistant.companion_bindings.as_mut() {
                Some(bindings) => bindings.hotkey = key,
                None if !key.is_empty() => {
                    settings.assistant.companion_bindings = Some(CompanionBindings {
                        hotkey: key,
                        ..Default::default()
                    })
                }
                None => {}
            },
            LegacyUpgrade::Pending | LegacyUpgrade::Dismissed => {}
        },
        CMD_STOP => settings.automation_stop_hotkey = key,
        CMD_SPEECH_TOGGLE => settings.runtime_toggle_hotkey = key,
        CMD_TALK => settings.hotkey.key = key,
        CMD_CHANNEL_CYCLE => settings.channel_cycle_hotkey = key,
        CMD_RESEND_LAST => settings.resend_last_hotkey = key,
        CMD_PP_TOGGLE => settings.toggle_post_processing_hotkey = key,
        CMD_PP_CYCLE => settings.cycle_post_processing_hotkey = key,
        _ => {}
    }
}

/// 读配置文件时（和默认值合并之前，读的是文件里原样的 JSON；语音输入插件的设置已经搬回顶层）：
/// 没有 `hotkeys.bindings`（或者它不是数组）就从旧字段造一份。旧字段没写时按旧版本的默认（[`LEGACY`]），
/// 写了空串就是没绑。`hotkeys` 下面别的东西不动。
pub fn migrate_hotkeys_json(user: &mut Value) {
    if !user.is_object()
        || user
            .pointer("/hotkeys/bindings")
            .is_some_and(Value::is_array)
    {
        return;
    }
    let text = |path: &str| {
        user.pointer(path)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let legacy_mode = text("/ui/legacyUpgrade").unwrap_or_else(|| "notNeeded".into());
    let mut bindings = Vec::new();
    for legacy in LEGACY {
        let found = match legacy.command {
            CMD_CALL => match legacy_mode.as_str() {
                // 还没启用她：没有呼叫键（`assistant.hotkey` 是提醒键）
                "pending" | "dismissed" => continue,
                "optedIn" => Some(text("/assistant/companionBindings/hotkey").unwrap_or_default()),
                _ => text("/assistant/hotkey"),
            },
            CMD_STOP => text("/automationStopHotkey"),
            CMD_SPEECH_TOGGLE => text("/runtimeToggleHotkey"),
            CMD_TALK => text("/hotkey/key"),
            CMD_CHANNEL_CYCLE => text("/channelCycleHotkey"),
            CMD_RESEND_LAST => text("/resendLastHotkey"),
            CMD_PP_TOGGLE => text("/togglePostProcessingHotkey"),
            CMD_PP_CYCLE => text("/cyclePostProcessingHotkey"),
            _ => None,
        };
        let key = found.unwrap_or_else(|| legacy.old_default.to_string());
        let key = key.trim();
        if !key.is_empty() {
            bindings.push(Binding::command(key, legacy.command).0);
        }
    }
    let Some(object) = user.as_object_mut() else {
        return;
    };
    let hotkeys = object.entry("hotkeys").or_insert_with(|| json!({}));
    if !hotkeys.is_object() {
        *hotkeys = json!({});
    }
    if let Some(hotkeys) = hotkeys.as_object_mut() {
        hotkeys.insert("bindings".into(), Value::Array(bindings));
    }
}

#[cfg(test)]
#[path = "hotkey_bindings_tests.rs"]
mod tests;
