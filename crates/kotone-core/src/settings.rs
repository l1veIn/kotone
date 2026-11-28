//! 用户配置读写：~/.kotone/config.json（docs/development.md §5.1、§5.4）
//!
//! 首次运行生成默认配置；缺失字段用默认值合并，保证向前兼容。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::hotkey::HotkeyMode;

/// 热键后端选择（docs/development.md §3.6）。
/// Windows 上 RegisterHotKey 在部分游戏前台不投递事件且不支持鼠标侧键，
/// LL 键鼠钩子（WH_KEYBOARD_LL + WH_MOUSE_LL）是主路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyBackend {
    /// Windows 优先 LL 钩子，安装失败回退 RegisterHotKey；非 Windows 恒 RegisterHotKey
    #[default]
    Auto,
    /// 强制 LL 键鼠钩子（普通键失败仍回退并记录日志）
    Llhook,
    /// 强制 RegisterHotKey（tauri-plugin-global-shortcut）
    Register,
}

/// 用户配置（字段与 docs/development.md §5.4 config.json 对应）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: HotkeyConfig,
    /// 热键后端：auto（默认）/ llhook / register
    #[serde(default)]
    pub hotkey_backend: HotkeyBackend,
    pub audio_device_id: String,
    /// 按键录音或琴音倾听时暂时降低 Windows 默认播放设备的音量。
    #[serde(default)]
    pub lower_speaker_volume_while_listening: bool,
    /// 当前 STT 引擎 ID（由活动模型推导后回写，兼容旧配置/CLI）
    pub stt_engine: String,
    /// 当前活动模型 ID。空 = 仍按 sttEngine + engineOptions.model 推导。
    #[serde(default)]
    pub active_model_id: String,
    /// 按模型 id 存放的 configSchema 值（语言、连接 id 等）。密钥不进这里。
    #[serde(default)]
    pub model_configs: std::collections::HashMap<String, serde_json::Value>,
    /// 引擎专有配置项
    pub engine_options: serde_json::Value,
    /// 【deprecated（UI 已撤）】true: 转写完直接发；false: 先预览确认。
    /// 仅 `interaction_mode = None` 的兼容路径（InteractionPolicy::from_settings
    /// 旧字段推导）还读它；三个交互模式预设下被 PostFinalize 完全覆盖。
    /// CLI wav 模式仍会强制写 false（兼容路径使用），键保留不删。
    pub auto_send: bool,
    pub active_profile_id: Option<String>,
    pub language: String,
    /// 评测录档开关（默认关；需要留语料复现时手动开）
    pub eval_recording: bool,
    /// 交互模式预设（ADR-006）：push-to-talk / dictation / one-shot / solo；
    /// 默认「对讲机」（0.1.5 起，修复跳过引导后无模式的问题：老配置缺键时
    /// 由 load_from 合并默认值自动迁移）。显式 null = 自定义兼容路径，
    /// 由 hotkey.mode + autoSend 旧字段推导（见 InteractionPolicy::from_settings）
    #[serde(default)]
    pub interaction_mode: Option<crate::interaction::InteractionMode>,
    /// VAD 静音判停阈值（ms，ADR-007；one-shot 模式生效，默认 700，
    /// 使用时 clamp 到 vad::SILENCE_MS_RANGE）
    #[serde(default = "default_vad_silence_ms")]
    pub vad_silence_ms: u32,
    /// 启动时自动以管理员重启自身（ADR 044 D1：默认开，英雄联盟这类游戏是管理员权限，普通权限的 Kotone
    /// 在它们前台时热键收不到、打字被挡）。还没用现在的说法问过（[`Settings::admin_prompt_revision`]）时
    /// 先不重开，窗口里先说明再弹 UAC；防循环见 elevation::should_auto_elevate。
    #[serde(default = "default_true")]
    pub run_as_admin_on_start: bool,
    /// 频道切换热键（ADR-008）。按声明顺序循环切换当前
    /// profile 的聊天频道；与录制热键冲突时后端拒绝注册并在状态里给出错误。
    /// 新装默认空（ADR 040 D4；以前默认 Shift+CapsLock，老用户迁移进表照旧生效）。
    /// 这一项和下面几个热键字段、`hotkey.key`、呼叫键现在都是 [`Settings::hotkeys`] 的镜像（只读，见 `hotkey_bindings`）。
    #[serde(default = "default_channel_cycle_hotkey")]
    pub channel_cycle_hotkey: String,
    /// 重发最近一条热键（默认空 = 关闭）。Idle 时按下把历史最新一条发送成功的
    /// 文本重新注入到当前前台窗口；与录制/频道切换热键冲突时拒绝注册。
    #[serde(default)]
    pub resend_last_hotkey: String,
    /// 文字处理启用开关热键（默认空 = 关闭）。按下切换 `postProcessing.enabled`
    /// （总开关：屏蔽词 + AI 润色一并启用/停用）；与其它热键冲突时拒绝注册。
    #[serde(default)]
    pub toggle_post_processing_hotkey: String,
    /// 切换文字处理流程热键（默认空 = 关闭）。按下按声明顺序循环切换当前激活的
    /// 后处理流程（不足两条流程时无操作）；与其它热键冲突时拒绝注册。
    #[serde(default)]
    pub cycle_post_processing_hotkey: String,
    /// 启动 / 停止 Kotone 热键（默认空 = 关闭）。**应用级**热键：runtime 未运行时
    /// 也保持注册（其余热键随 runtime 启停），因此它是唯一能在停止状态下把
    /// Kotone 拉起来的热键；与其它热键冲突时拒绝注册。
    #[serde(default)]
    pub runtime_toggle_hotkey: String,
    /// 停止全部自动化（应用级热键，runtime 未运行时也有效；默认 Ctrl+Shift+F12，留空 = 关闭）。
    #[serde(default = "default_automation_stop_hotkey")]
    pub automation_stop_hotkey: String,
    /// 「以管理员权限重启」启动提示：用户勾选「不再提示」后置 true，不再弹窗（默认 false）
    #[serde(default)]
    pub admin_prompt_dismissed: bool,
    /// 提权重启成功后的「启动时自动请求管理员权限」询问：用户勾选「不再提醒」
    /// 后置 true，不再弹窗（默认 false；与 admin_prompt_dismissed 独立，两段式提示各记各的）
    #[serde(default)]
    pub auto_admin_prompt_dismissed: bool,
    /// 管理员身份的说明（ADR 044 D1）用第几版的说法问过：0 = 没问过（新装的、以前点过“不再提示”的、以前就开着的
    /// 都是 0）。比 `elevation::ADMIN_PROMPT_REVISION` 小时启动先不以管理员身份重开，窗口里说明一句再弹 UAC；
    /// 玩家点「好」或「先不用」、或者在「设置 → 系统」里拨了开关，就记成现在的版本。上面两个“不再提示”只读不再写。
    #[serde(default)]
    pub admin_prompt_revision: u32,
    /// 识别历史记录（默认 capped/1000 条/不含音频；off = 零开销不记录）
    #[serde(default)]
    pub history: crate::history::HistoryConfig,
    /// 桌面壳 UI 状态（首启向导等；缺省合并默认 = 未完成的向导会弹一次）
    #[serde(default)]
    pub ui: UiConfig,
    /// 模型存储配置（自定义目录；默认空 = ~/.kotone/models）
    #[serde(default)]
    pub models: ModelsConfig,
    /// 模型下载源配置（镜像 / 代理，见 DownloadConfig）
    #[serde(default)]
    pub download: DownloadConfig,
    /// 悬浮窗配置（显示模式等，见 OverlayConfig）
    #[serde(default)]
    pub overlay: OverlayConfig,
    /// 录音/发送音效提示（见 SoundFeedbackConfig；默认开启，音量 60）
    #[serde(default)]
    pub sound_feedback: SoundFeedbackConfig,
    /// silero VAD 内部参数（ADR-007；one-shot/solo 生效）。
    /// 与 `vad_silence_ms`（判停阈值，SilenceStopTracker 用）独立——
    /// 这里控制 silero 模型的帧级判定灵敏度。默认对齐 kotone-stt vad.rs 原硬编码值。
    #[serde(default)]
    pub vad: VadConfig,
    /// 热词命中加分（默认 3.5；越高热词越易命中，也越容易把背景噪声
    /// 识别成热词。X-ASR 等引擎 recognizer 级配置，改后需「停止→启动」重建）
    #[serde(default = "default_hotwords_score")]
    pub hotwords_score: f32,
    /// STT 最终文本与预览/发送之间的后处理；默认开启内置「屏蔽词」流程。
    #[serde(default)]
    pub post_processing: crate::postprocess::PostProcessingConfig,
    /// 后处理在线连接目录（不含 API key）。缺省空列表，旧配置零行为变化。
    #[serde(default)]
    pub connections: Vec<crate::connection::Connection>,
    /// 诊断记录（流程事件）；默认开启。关闭后不再写入 events.jsonl。
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
    /// 独立语音助手入口、AI 连接和提醒呈现偏好。
    #[serde(default)]
    pub assistant: crate::assistant::AssistantConfig,
    /// 内置 Agent（琴音）：权限模式与规则、不交给 AI 的已保存自动化。新字段，缺省 = 默认规则。
    #[serde(default)]
    pub agent: crate::agent::AgentConfig,
    /// 本机 Agent 工具服务（默认关闭；修改后需重启桌面实例才生效，见 docs/tool-service.md）。
    #[serde(default)]
    pub tool_service: ToolServiceConfig,
    /// 文字识别（统一模型清单的 OCR 类别）：当前选用的 OCR 模型。
    #[serde(default)]
    pub ocr: OcrConfig,
    /// 联网搜索（统一模型清单的 search 类别，ADR 026）。新字段。
    #[serde(default)]
    pub search: SearchConfig,
    /// 内置浏览器（ADR 031）：开关（设置 → 浏览器，ADR 038）和玩家加的扩展。新字段，旧版本会忽略。
    #[serde(default)]
    pub browser: BrowserConfig,
    /// 插件（ADR 038）：每个插件一份设置，按插件 id（`plugins.lol.sources.client`）。新字段，旧版本会忽略。
    /// 旧的 `lol.enabled`（ADR 035）读进来时搬到 `plugins.lol.sources.client`（[`migrate_plugins_json`]）。
    #[serde(default)]
    pub plugins: std::collections::BTreeMap<String, PluginConfig>,
    /// 热键表（ADR 040 D8）：`hotkeys.bindings`，每条 键 + 命令，或者玩家自己加的（跟她说一句、运行自动化、调工具、
    /// 打开页面，第 3 期）。新字段，旧版本会忽略；热键的旧字段（`hotkey.key`、`channelCycleHotkey`……、呼叫键、快捷指令
    /// `assistant.companion.quickMessages`）是它的镜像，每次保存照表写全（[`crate::hotkey_bindings`]、
    /// [`crate::hotkey_custom`]：读旧写新、旧写法照样能改）。
    #[serde(default)]
    pub hotkeys: crate::hotkey_bindings::HotkeysConfig,
}

/// 一个插件的设置（config.json `plugins.<id>`，ADR 038 D6）。没写的项按插件清单（`plugin.toml`）的默认。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    /// 插件开关；没写 = 清单的 `enabled_by_default`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// 插件里的数据开关（清单的 `sources`）；没写的按清单的默认。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sources: std::collections::BTreeMap<String, bool>,
    /// 插件自己的其余设置（插件自己解析，宿主原样存）。
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// 插件的内置推送开关（清单的 `[[pushes]]`，ADR 043 D4，插件页「设置 → 推送」）：玩家动过的才写；没写的按清单的
    /// 默认（规则在 `kotone-plugin-api` 的 `Manifest::push_switch_in`）。新字段，旧版本会忽略。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub pushes: std::collections::BTreeMap<String, bool>,
    /// 插件的“风险和许可”弹窗看过第几版（ADR 043 D8）：0 = 没看过。比界面的版本（`lib/plugins.ts` 的 `RISK_REVISION`）
    /// 小时，第一次打开有风险的开关、或者有风险的开关默认开着而第一次打开插件页，弹一次；弹出来就记成现在的版本。
    /// 只是提示，不管任何开关。新字段，旧版本会忽略。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub risk_seen: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Settings {
    /// 插件开着没有：设置里写了按设置，没写按清单的默认。
    pub fn plugin_enabled(&self, id: &str, by_default: bool) -> bool {
        self.plugins
            .get(id)
            .and_then(|p| p.enabled)
            .unwrap_or(by_default)
    }

    /// 插件里的某个数据开关开着没有（插件本身关着时也算关）。
    pub fn plugin_source_enabled(&self, id: &str, source: &str, by_default: bool) -> bool {
        self.plugins
            .get(id)
            .and_then(|p| p.sources.get(source).copied())
            .unwrap_or(by_default)
    }

    /// 玩家给插件的某条推送设的开关（`plugins.<id>.pushes.<推送>`）；没动过是 None（按清单的默认，ADR 043 D4）。
    pub fn plugin_push(&self, id: &str, push: &str) -> Option<bool> {
        self.plugins
            .get(id)
            .and_then(|p| p.pushes.get(push).copied())
    }
}

/// 内置浏览器（config.json `browser` 段，ADR 031）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConfig {
    /// 内置浏览器开着没有（设置 → 浏览器，ADR 038 “怎么定的”；默认开）。关着时右侧栏没有“网页”、
    /// 开着的网页都关掉，她的工具清单里没有 `browser.*`。新字段：旧配置没有它时按开着读。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 玩家加的扩展：已解压的 Chrome / Edge 扩展文件夹（WebView2 只支持这种，没有应用商店）。
    /// 界面上叫“扩展”，不叫插件（插件是她身上的本事，ADR 038）。
    #[serde(default)]
    pub extensions: Vec<BrowserExtensionConfig>,
    /// 浏览器没开着时删掉的扩展（WebView2 给的扩展 id），下次打开浏览器时卸掉。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_uninstall: Vec<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            extensions: Vec::new(),
            pending_uninstall: Vec::new(),
        }
    }
}

/// 一个扩展（Chrome / Edge 扩展）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserExtensionConfig {
    /// Kotone 自己的编号（插件文件夹路径的哈希），界面和命令用。
    #[serde(default)]
    pub id: String,
    /// 插件文件夹（里面有 manifest.json）。
    #[serde(default)]
    pub path: String,
    /// 插件名（manifest 里的 name）。
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// WebView2 装好后给的插件 id（装过才有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_id: Option<String>,
}

/// OCR 模型选择（config.json `ocr` 段）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrConfig {
    /// 当前 OCR 模型 ID（统一模型清单 `category=ocr`）。空 = 自动：按清单推荐顺序取第一个
    /// 已就绪的本地模型；都没有时为“未选择”。不会自动改用在线模型。
    #[serde(default)]
    pub active_model_id: String,
}

/// 联网搜索的选择（config.json `search` 段）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchConfig {
    /// 统一模型清单 `category=search` 的 ID。空 = 自动：有 DeepSeek 连接就用 DeepSeek 联网搜索，
    /// 否则不联网；`off` = 明确关掉。
    #[serde(default)]
    pub active_model_id: String,
}

/// 本机工具服务开关（config.json `toolService` 段）。
///
/// 只有显式开启时，桌面实例才在 127.0.0.1 随机端口上启动带会话凭证的工具服务。
/// 不提供固定端口或外网监听选项。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolServiceConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 工具服务的 OCR 后端配置（M2）。
    #[serde(default)]
    pub ocr: ToolOcrConfig,
}

/// 工具服务 OCR 配置（config.json `toolService.ocr` 段）。
///
/// 在线 OCR 默认关闭：只有用户显式指定一条已有连接（API key 仍在系统凭据库）后，
/// `deepseek-ocr` 后端才会就绪；Agent 不能通过参数改选其他连接或端点。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOcrConfig {
    /// 用于 DeepSeek 在线 OCR 的连接 ID；None = 不启用在线 OCR（不会外发图像）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepseek_connection_id: Option<String>,
}

/// 诊断记录开关（config.json `diagnostics` 段）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsConfig {
    /// 是否写入流程事件。默认 true。
    #[serde(default = "default_true")]
    pub recording: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self { recording: true }
    }
}

/// 旧版用户的过渡状态。缺少字段的 0.1.x 配置在合并默认值前标为 Pending（看升级欢迎页）；选了“继续使用语音输入法”
/// 是 Dismissed（她不出声）。以前启用了她的 `optedIn` 读成 NotNeeded：她的连接和呼叫键在读取时搬回
/// `assistant.connectionId` / `assistant.hotkey`（ADR 052，见 `migrate_legacy_upgrade_json`），和新用户一样。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegacyUpgrade {
    #[default]
    #[serde(alias = "optedIn")]
    NotNeeded,
    Pending,
    Dismissed,
}

impl LegacyUpgrade {
    /// 只用语音输入（还没看升级欢迎页，或选了“继续使用语音输入法”）：她不出声、没有呼叫键和停下热键。
    pub fn voice_input_only(self) -> bool {
        matches!(self, Self::Pending | Self::Dismissed)
    }
}

/// 桌面壳 UI 状态（config.json `ui` 段）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    /// 老用户的过渡说明及是否选择启用琴音。
    #[serde(default)]
    pub legacy_upgrade: LegacyUpgrade,
    /// 新用户的首启向导是否完成。
    #[serde(default)]
    pub first_run_completed: bool,
    /// app 启动后自动进入 Running（warmup 引擎 + 注册热键 + 显示悬浮窗）；默认 false
    #[serde(default)]
    pub auto_start: bool,
    /// 用户在新版本弹窗里选了「不再提醒」：启动时不再自动弹出（关于页仍可手动检查，也可重新打开）。
    #[serde(default)]
    pub update_prompt_disabled: bool,
    /// 打开软件时显示哪一页（ADR 028）：`"voiceInput"` = 语音输入；空串或任何不认识的值 = 聊天。
    /// 存成字符串而不是枚举：以后加取值时，读到新值的旧版本不会把整份配置当成损坏
    /// （docs/development.md 的配置兼容规则）。0.1.x 不认识这个字段，会忽略它。
    #[serde(default)]
    pub start_page: String,
    /// 左侧栏页面的顺序和隐藏（ADR 034 第六节）。空 = 默认。新字段，旧版本会忽略。
    #[serde(default)]
    pub sidebar: SidebarConfig,
    /// 聊天页“准备好了吗”里点过“就用这个键”（ADR 054 第 4 条：呼叫键还是默认的时候，玩家说了就用它，这一行才算 ✓）。
    /// 新字段，旧配置没有 = 没点过。
    #[serde(default)]
    pub call_key_chosen: bool,
}

/// 左侧栏（config.json `ui.sidebar`，ADR 034 第六节）：页面 id 的顺序（在各自的分组里排）和隐藏的页面。
/// 不认识的 id 忽略；新加的页面按默认位置出现；“设置”（`advanced`）不能隐藏（前端保证，这里也不信任）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarConfig {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

/// 不能隐藏的页面：设置（藏了就回不来了）。
pub const SIDEBAR_ALWAYS_SHOWN: &[&str] = &["advanced"];

impl SidebarConfig {
    /// 保存前整理：去重、去空、太长的不要，“设置”不能隐藏。
    pub fn normalize(&mut self) {
        fn clean(ids: &mut Vec<String>) {
            let mut seen = std::collections::HashSet::new();
            ids.retain(|id| {
                let id = id.trim();
                !id.is_empty() && id.len() <= 40 && seen.insert(id.to_string())
            });
            ids.truncate(40);
        }
        clean(&mut self.order);
        clean(&mut self.hidden);
        self.hidden
            .retain(|id| !SIDEBAR_ALWAYS_SHOWN.contains(&id.as_str()));
    }
}

/// `ui.startPage` 里表示“打开时显示语音输入”的值。
pub const START_PAGE_VOICE_INPUT: &str = "voiceInput";

impl UiConfig {
    /// 打开软件时是否落在语音输入（否则落在聊天）。
    pub fn starts_on_voice_input(&self) -> bool {
        self.start_page == START_PAGE_VOICE_INPUT
    }
}

/// 录音/发送音效提示（config.json `soundFeedback` 段）。
///
/// 面向不开悬浮窗/全屏游戏场景：开始监听时播放「录制音」（上扬），
/// 消息发送成功时播放「发送音」（下沉）。录制音与发送音独立开关、
/// 独立音量、各自从 3 个内置音效中选择（soundId），未来各自支持自定义上传。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundFeedbackConfig {
    #[serde(default)]
    pub record: SoundFeedbackItemConfig,
    #[serde(default)]
    pub send: SoundFeedbackItemConfig,
}

impl Default for SoundFeedbackConfig {
    fn default() -> Self {
        Self {
            record: SoundFeedbackItemConfig::default(),
            send: SoundFeedbackItemConfig {
                sound_id: "fall".into(),
                ..SoundFeedbackItemConfig::default()
            },
        }
    }
}

/// 单个音效（录制/发送）配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundFeedbackItemConfig {
    /// 是否启用（默认 true）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 音量百分比 0-100（默认 60；使用时 clamp 后转增益）
    #[serde(default = "default_sound_feedback_volume")]
    pub volume: u8,
    /// 内置音效 id（rise/ding-up/chirp-up | fall/knock/ding-down；默认 rise）
    #[serde(default = "default_record_sound_id")]
    pub sound_id: String,
}

impl Default for SoundFeedbackItemConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 60,
            sound_id: "rise".into(),
        }
    }
}

fn default_sound_feedback_volume() -> u8 {
    SoundFeedbackItemConfig::default().volume
}

fn default_record_sound_id() -> String {
    SoundFeedbackItemConfig::default().sound_id
}

/// 模型存储配置（config.json `models` 段）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsConfig {
    /// 自定义模型目录；空 = 默认 ~/.kotone/models
    #[serde(default)]
    pub dir: String,
}

/// 悬浮窗显示模式（config.json `overlay.visibility`，docs/development.md §3.6）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayVisibility {
    /// 常驻：启动即显示，停止才隐藏
    Always,
    /// 用时浮现（默认）：平时隐藏；收音/转写/发送时浮现；一次发送完成（成功或失败）
    /// 延迟 ~600ms 自动隐藏；solo 连续模式保持显示直到会话停止
    #[default]
    OnDemand,
    /// 完全不显示：热键和语音输入照常工作，所有悬浮窗唤起事件均被忽略
    Never,
}

/// 悬浮窗样式（config.json `overlay.style`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayStyle {
    /// 卡片：480×120 圆角面板，屏幕中央，贴纸/气泡等完整装饰
    Card,
    /// 胶囊（默认）：Win11 语音输入条风格——水平居中靠下（底部留 ~48px），
    /// 圆角胶囊宽度随内容伸缩（窗口上限 520px），轻装饰
    #[default]
    Capsule,
}

/// 悬浮窗固定位置（config.json `overlay.position`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    /// 跟随样式：卡片居中，胶囊底部居中。
    #[default]
    Auto,
    TopLeft,
    TopCenter,
    TopRight,
    Center,
    BottomLeft,
    BottomCenter,
    BottomRight,
    /// 用户拖动后记录物理屏幕坐标。
    Custom,
}

/// 悬浮窗配置（config.json `overlay` 段）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayConfig {
    /// 显示模式：on_demand 用时浮现（默认）/ always 常驻 / never 完全隐藏
    #[serde(default)]
    pub visibility: OverlayVisibility,
    /// 样式：capsule 胶囊（默认）/ card 卡片
    #[serde(default)]
    pub style: OverlayStyle,
    /// 固定位置预设；拖动完成后切为 custom。
    #[serde(default)]
    pub position: OverlayPosition,
    /// 是否允许直接拖动悬浮窗。
    #[serde(default)]
    pub draggable: bool,
    /// 鼠标点击穿透到游戏；开启时悬浮窗自身按钮和拖动不可用。
    #[serde(default = "default_true")]
    pub click_through: bool,
    /// custom 位置的物理屏幕坐标（支持多显示器负坐标）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_y: Option<i32>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            visibility: OverlayVisibility::default(),
            style: OverlayStyle::default(),
            position: OverlayPosition::default(),
            draggable: false,
            click_through: true,
            custom_x: None,
            custom_y: None,
        }
    }
}

impl OverlayConfig {
    /// UI 只保留一个「允许交互并拖动」开关：
    /// - 允许拖动 = 不穿透；
    /// - 不允许拖动 = 鼠标穿透。
    ///
    /// 老配置可能同时写了两个独立字段，迁移时点击穿透优先，避免升级后突然拦截游戏鼠标。
    pub fn normalize_interaction(&mut self) {
        if self.click_through {
            self.draggable = false;
        } else {
            self.click_through = !self.draggable;
        }
    }
}

/// silero VAD 内部参数（config.json `vad` 段；ADR-007）。
/// 与 `vad_silence_ms`（判停阈值，SilenceStopTracker 用）独立。
/// 默认值 = kotone-stt vad.rs 原硬编码值。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadConfig {
    /// 语音判定阈值（0.1–0.9，默认 0.5）。调高 → 更严格（背景噪声更难
    /// 被判成语音）；调低 → 更宽松、更容易误判。
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,
    /// 最短语音时长 ms（20–500，默认 50）。拉长可过滤短促噪声突发。
    #[serde(default = "default_vad_min_speech_ms")]
    pub min_speech_ms: u32,
    /// 最短静音时长 ms（20–500，默认 50）。
    #[serde(default = "default_vad_min_silence_ms")]
    pub min_silence_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: default_vad_threshold(),
            min_speech_ms: default_vad_min_speech_ms(),
            min_silence_ms: default_vad_min_silence_ms(),
        }
    }
}

fn default_vad_threshold() -> f32 {
    0.5
}

fn default_vad_min_speech_ms() -> u32 {
    50
}

fn default_vad_min_silence_ms() -> u32 {
    50
}

/// 热词命中加分默认值（Sherpa 官方示例常用强度；SessionConfig 与
/// settings.hotwords_score 的共同真源，改动需两侧同步）
pub fn default_hotwords_score() -> f32 {
    3.5
}

/// 下载源选择（config.json `download.source`）。
/// 大模型优先使用 ModelScope 国内镜像，其他资源仍可使用 HF / GitHub 镜像。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
    /// ModelScope / 其他镜像优先，失败后自动回退官方源（默认）
    #[default]
    Auto,
    /// 只用官方源（huggingface.co / github.com）
    Official,
    /// 只用镜像（ModelScope / hf-mirror.com / ghProxy），不回退
    Mirror,
}

/// 模型下载配置（config.json `download` 段）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadConfig {
    /// 下载源：auto（默认）/ official / mirror
    #[serde(default)]
    pub source: DownloadSource,
    /// GitHub 加速代理前缀（默认 https://ghfast.top/）。
    /// 此类公益代理稳定性无保障、可能随时失效，故做成可配置项：
    /// 失效时换成其他可用前缀即可，无需升级版本。
    #[serde(default = "default_gh_proxy")]
    pub gh_proxy: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            source: DownloadSource::Auto,
            gh_proxy: default_gh_proxy(),
        }
    }
}

fn default_automation_stop_hotkey() -> String {
    "Ctrl+Shift+F12".into()
}

fn default_gh_proxy() -> String {
    "https://ghfast.top/".into()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyConfig {
    pub key: String,
    pub mode: HotkeyMode,
}

impl Default for Settings {
    /// 默认值对齐 docs/development.md §5.4 示例
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig {
                key: "CapsLock".into(),
                mode: HotkeyMode::Toggle,
            },
            hotkey_backend: HotkeyBackend::Auto,
            audio_device_id: "default".into(),
            lower_speaker_volume_while_listening: false,
            stt_engine: "sherpa-streaming".into(),
            active_model_id: String::new(),
            model_configs: HashMap::new(),
            engine_options: serde_json::json!({
                "sherpa-streaming": { "provider": "cpu" }
            }),
            auto_send: false,
            active_profile_id: Some("lol".into()),
            language: "zh".into(),
            eval_recording: false,
            interaction_mode: Some(crate::interaction::InteractionMode::PushToTalk),
            vad_silence_ms: default_vad_silence_ms(),
            run_as_admin_on_start: true,
            channel_cycle_hotkey: default_channel_cycle_hotkey(),
            resend_last_hotkey: String::new(), // 默认关闭（设置里录入）
            toggle_post_processing_hotkey: String::new(), // 默认关闭（设置里录入）
            cycle_post_processing_hotkey: String::new(), // 默认关闭（设置里录入）
            runtime_toggle_hotkey: String::new(), // 默认关闭（设置里录入）
            automation_stop_hotkey: default_automation_stop_hotkey(),
            admin_prompt_dismissed: false,
            auto_admin_prompt_dismissed: false,
            admin_prompt_revision: 0,
            history: crate::history::HistoryConfig::default(),
            ui: UiConfig::default(),
            models: ModelsConfig::default(),
            download: DownloadConfig::default(),
            overlay: OverlayConfig::default(),
            sound_feedback: SoundFeedbackConfig::default(),
            vad: VadConfig::default(),
            hotwords_score: default_hotwords_score(),
            post_processing: crate::postprocess::PostProcessingConfig::default(),
            connections: Vec::new(),
            diagnostics: DiagnosticsConfig::default(),
            assistant: crate::assistant::AssistantConfig::default(),
            agent: crate::agent::AgentConfig::default(),
            tool_service: ToolServiceConfig::default(),
            ocr: OcrConfig::default(),
            search: SearchConfig::default(),
            browser: BrowserConfig::default(),
            plugins: Default::default(),
            hotkeys: Default::default(),
        }
    }
}

impl Settings {
    /// 呼叫她的键。她的连接和呼叫键只在 `assistant.connectionId` / `assistant.hotkey`（ADR 052）；
    /// 只用语音输入（0.1.x 还没启用她）时没有呼叫键。
    pub fn call_hotkey(&self) -> &str {
        if self.ui.legacy_upgrade.voice_input_only() {
            ""
        } else {
            &self.assistant.hotkey
        }
    }

    /// 她用的 API 连接；只用语音输入时没有。
    pub fn companion_connection_id(&self) -> &str {
        if self.ui.legacy_upgrade.voice_input_only() {
            ""
        } else {
            &self.assistant.connection_id
        }
    }

    /// 琴音的 DeepSeek 连接：琴音正在用的（若是 DeepSeek）→ 第一条 DeepSeek 连接。
    /// DeepSeek 的语音识别、文字识别、语音合成没单独选连接时都用它（只配一个 DeepSeek 就能用全部功能）。
    pub fn deepseek_connection_id(&self) -> Option<String> {
        let is_deepseek = |c: &&crate::connection::Connection| {
            c.provider == "deepseek" || c.base_url.contains("api.deepseek.com")
        };
        let agent = self.assistant.connection_id.trim();
        self.connections
            .iter()
            .filter(is_deepseek)
            .find(|c| c.id == agent)
            .or_else(|| self.connections.iter().find(is_deepseek))
            .map(|c| c.id.clone())
    }

    /// 引擎专有项 + 当前活动模型的 configSchema 值。
    /// 旧家族 id 与新 I/O 引擎 id 的 engineOptions 会合并；活动模型 id 写入 `model`。
    pub fn session_options(&self, engine_id: &str) -> serde_json::Value {
        let canonical = crate::stt::canonical_stt_engine(engine_id);
        let mut opts = serde_json::json!({});
        for key in crate::stt::stt_engine_option_keys(engine_id)
            .iter()
            .copied()
            .chain(std::iter::once(engine_id))
            .chain(std::iter::once(canonical))
        {
            if let Some(extra) = self.engine_options.get(key) {
                merge_json_objects(&mut opts, extra);
            }
        }
        if !opts.is_object() {
            opts = serde_json::json!({});
        }
        let model_id = if !self.active_model_id.trim().is_empty() {
            self.active_model_id.clone()
        } else {
            opts.get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string()
        };
        if let Some(cfg) = self.model_configs.get(&model_id) {
            if let (Some(base), Some(extra)) = (opts.as_object_mut(), cfg.as_object()) {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
        }
        if !model_id.is_empty() {
            if let Some(base) = opts.as_object_mut() {
                base.insert("model".into(), serde_json::json!(model_id));
            }
        }
        opts
    }
}

fn merge_json_objects(base: &mut serde_json::Value, extra: &serde_json::Value) {
    match (base.as_object_mut(), extra.as_object()) {
        (Some(dst), Some(src)) => {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        (None, Some(_)) => *base = extra.clone(),
        _ => {}
    }
}

/// 从当前 Settings 快照解析连接公开字段。密钥由后续凭据层补上。
pub struct SettingsConnectionResolver {
    settings: Arc<RwLock<Settings>>,
}

impl SettingsConnectionResolver {
    pub fn new(settings: Arc<RwLock<Settings>>) -> Self {
        Self { settings }
    }
}

impl crate::connection::ConnectionResolver for SettingsConnectionResolver {
    fn resolve(
        &self,
        connection_id: &str,
    ) -> Result<crate::connection::ResolvedConnection, String> {
        let id = connection_id.trim();
        if id.is_empty() {
            return Err("连接 ID 不能为空".into());
        }
        let settings = self.settings.read().unwrap();
        let found = settings
            .connections
            .iter()
            .find(|connection| connection.id == id)
            .ok_or_else(|| format!("未找到连接：{id}"))?;
        found.validate()?;
        Ok(crate::connection::ResolvedConnection::from_public(found))
    }
}

fn default_vad_silence_ms() -> u32 {
    crate::vad::DEFAULT_SILENCE_MS
}

/// 频道切换热键默认值：新装不带（ADR 040 D4）。旧版本的默认 Shift+CapsLock 只在迁移老配置时用
/// （[`crate::hotkey_bindings::LEGACY`]）。
fn default_channel_cycle_hotkey() -> String {
    String::new()
}

/// Kotone 用户数据目录：~/.kotone/
/// 环境变量：覆盖 Kotone 数据目录（默认 `~/.kotone`）。供 Agent 隔离运行与自动化测试使用；
/// 桌面端、CLI 与工具服务的描述文件目录都随之切换。
pub const HOME_ENV: &str = "KOTONE_HOME";

/// Kotone 数据目录：`KOTONE_HOME`，没设时是 `~/.kotone`。
///
/// 测试程序永远拿不到玩家真实的 `~/.kotone`：没设 `KOTONE_HOME` 的测试进程用自己的临时目录
/// （[`isolated_home`]）。以前一个夹具忘了关历史，每跑一次 `cargo test` 就往玩家的语音输入历史里
/// 追加一条“提醒”（2026-11-26 试玩讨论第九节）；这里兜底，哪个 crate 的测试都写不进去。
pub fn kotone_dir() -> PathBuf {
    isolated_home().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".kotone")
    })
}

/// 隔离的数据目录：设了 `KOTONE_HOME`（非空）就是它；没设但当前进程是测试程序，就是这个测试进程
/// 自己的临时目录；都不是时为 `None`（玩家真实的 `~/.kotone`）。凭据库的服务名也按它区分
/// （kotone-ai `secrets`），测试同样碰不到玩家存的 Key。
pub fn isolated_home() -> Option<PathBuf> {
    match std::env::var_os(HOME_ENV) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => test_process_home().cloned(),
    }
}

/// 测试进程的数据目录：`<target>/tmp/kotone-test-home/<进程号>`（doctest 在系统临时目录下）。
/// 不是测试程序时为 `None`。整个进程只算一次，第一次用到时在 stderr 说一句。
///
/// 为什么看 exe 的位置：`cfg!(test)` 只在被测的那个 crate 里成立，管不到集成测试和别的 crate 的测试；
/// cargo feature 会合进 `cargo test` 顺手编出来的 `target/debug/kotone.exe`；`.cargo/config.toml` 的
/// `[env]` 连 `cargo run`（`pnpm dev`）也会改。cargo 的测试程序名字总带 16 位哈希，放在固定的地方
/// （nextest 跑的是同一批程序），`pnpm dev`、真机脚本跑的 `target/debug/kotone.exe` 和安装版都不在这些地方。
fn test_process_home() -> Option<&'static PathBuf> {
    static HOME: OnceLock<Option<PathBuf>> = OnceLock::new();
    HOME.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let tmp = test_binary_tmp(&exe, &std::env::temp_dir())?;
        let home = tmp
            .join("kotone-test-home")
            .join(std::process::id().to_string());
        // 进程号会被复用：先清掉上一个同号测试进程留下的，每次都从空目录开始（和 CI 上一样）。
        let _ = std::fs::remove_dir_all(&home);
        eprintln!(
            "kotone: 测试程序没设 {HOME_ENV}，数据目录用 {}（不碰真实的 ~/.kotone）",
            home.display()
        );
        Some(home)
    })
    .as_ref()
}

/// 认出 cargo 的测试程序，返回放测试数据的目录；不是测试程序返回 `None`。
/// - 旧布局：`<target>/<profile>/deps/<名字>-<16 位哈希>.exe` → `<target>/tmp`
/// - Cargo 1.100 起（build-dir 新布局，cargo#17354）：
///   `<target>/<profile>/build/<包>/<哈希>/out/<名字>-<哈希>.exe` → `<target>/tmp`
/// - doctest：rustdoc 编到 `<系统临时目录>/rustdoctest<随机>/` 里 → 系统临时目录
fn test_binary_tmp(exe: &Path, temp_dir: &Path) -> Option<PathBuf> {
    let parent = exe.parent()?;
    let parent_name = parent.file_name()?.to_str()?;
    if parent_name.starts_with("rustdoctest") {
        return Some(temp_dir.to_path_buf());
    }
    let stem = exe.file_stem()?.to_str()?;
    let (_, hash) = stem.rsplit_once('-')?;
    if hash.len() != 16 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let profile = match parent_name {
        "deps" => parent.parent()?,
        "out" => {
            let hash_dir = parent.parent()?;
            let build = hash_dir.parent()?.parent()?;
            if hash_dir.file_name()? != hash || build.file_name()? != "build" {
                return None;
            }
            build.parent()?
        }
        _ => return None,
    };
    Some(profile.parent()?.join("tmp"))
}

fn config_path() -> PathBuf {
    kotone_dir().join("config.json")
}

/// 配置加载结果。损坏配置会被移动到 `.corrupt` 备份，并通过 `warning`
/// 交给桌面端显示；调用方不能再无声地用默认值覆盖原文件。
#[derive(Debug)]
pub struct SettingsLoadResult {
    pub settings: Settings,
    pub warning: Option<String>,
}

/// 串行化单进程内的配置修改，并保证磁盘写入、外部资源切换与内存发布按事务执行。
///
/// 外部资源（例如全局热键）无法和文件系统组成真正的原子事务，因此失败时会同时
/// 尝试恢复旧资源和旧配置；内存快照只会在全部步骤成功后发布。
pub struct SettingsRepository {
    current: Arc<RwLock<Settings>>,
    path: PathBuf,
    transaction: Mutex<()>,
}

impl SettingsRepository {
    pub fn new(current: Arc<RwLock<Settings>>) -> Self {
        Self::new_at(current, config_path())
    }

    pub fn new_at(current: Arc<RwLock<Settings>>, path: PathBuf) -> Self {
        Self {
            current,
            path,
            transaction: Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> Settings {
        self.current.read().unwrap().clone()
    }

    /// 仅修改配置的常用事务。
    pub fn update<M>(&self, mutate: M) -> Result<Settings, String>
    where
        M: FnOnce(&mut Settings) -> Result<(), String>,
    {
        self.update_with(mutate, |_, _| Ok(()), |_, _| Ok(()))
            .map(|(_, next, ())| next)
    }

    /// 带外部资源切换的配置事务。
    ///
    /// 顺序为：从最新内存快照构造 next → 写盘 next → apply → 发布 next。
    /// apply 失败时调用 rollback，并把磁盘恢复为 old；返回值同时携带 old/next，
    /// 便于调用方在提交后更新窗口等非关键 UI 副作用。
    pub fn update_with<M, A, R, T>(
        &self,
        mutate: M,
        apply: A,
        rollback: R,
    ) -> Result<(Settings, Settings, T), String>
    where
        M: FnOnce(&mut Settings) -> Result<(), String>,
        A: FnOnce(&Settings, &Settings) -> Result<T, String>,
        R: FnOnce(&Settings, &Settings) -> Result<(), String>,
    {
        let _transaction = self.transaction.lock().unwrap();
        let old = self.current.read().unwrap().clone();
        let mut next = old.clone();
        mutate(&mut next)?;
        // 热键：改了旧字段（旧写法）就改表，最后把旧字段照表写回（ADR 040 D8）
        next.reconcile_hotkeys(&old);
        save_to(&self.path, &next)?;

        let applied = match apply(&old, &next) {
            Ok(value) => value,
            Err(apply_error) => {
                let rollback_resource_error = rollback(&old, &next).err();
                let rollback_file_error = save_to(&self.path, &old).err();
                let mut message = format!("应用配置失败，已回滚: {apply_error}");
                if let Some(error) = rollback_resource_error {
                    message.push_str(&format!("；恢复外部资源失败: {error}"));
                }
                if let Some(error) = rollback_file_error {
                    message.push_str(&format!("；恢复配置文件失败: {error}"));
                }
                return Err(message);
            }
        };

        *self.current.write().unwrap() = next.clone();
        Ok((old, next, applied))
    }
}

/// 读取配置：文件不存在时生成默认配置并落盘（首次运行）；
/// 已存在时按默认值合并缺失字段（老版本配置向前兼容）。
pub fn load() -> Settings {
    load_with_diagnostic().settings
}

/// 桌面端启动使用：除配置外返回可直接展示给用户的恢复诊断。
pub fn load_with_diagnostic() -> SettingsLoadResult {
    load_from_with_diagnostic(&config_path())
}

/// 保存配置（原子写入：先写临时文件再重命名）
pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&config_path(), settings)
}

/// 从指定路径读取配置（测试可用临时目录）
pub fn load_from(path: &Path) -> Settings {
    load_from_with_diagnostic(path).settings
}

pub fn load_from_with_diagnostic(path: &Path) -> SettingsLoadResult {
    if !path.exists() {
        let defaults = Settings::default();
        // 首次运行落盘默认配置；失败不致命，下次启动再试
        let _ = save_to(path, &defaults);
        return SettingsLoadResult {
            settings: defaults,
            warning: None,
        };
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(error) => return recover_invalid_config(path, format!("读取失败: {error}")),
    };
    // 以默认值为底、用户配置覆盖，实现缺失字段合并
    let mut merged = serde_json::to_value(Settings::default()).unwrap_or_default();
    let user = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(user) => user,
        Err(error) => return recover_invalid_config(path, format!("JSON 解析失败: {error}")),
    };
    let mut user = user;
    lift_voice_input_json(&mut user);
    migrate_legacy_post_processing_json(&mut user);
    migrate_legacy_upgrade_json(&mut user);
    migrate_call_hotkey_json(&mut user);
    migrate_call_idle_json(&mut user);
    migrate_call_mode_json(&mut user);
    migrate_plugins_json(&mut user);
    crate::hotkey_bindings::migrate_hotkeys_json(&mut user);
    merge_json(&mut merged, &user);
    let mut settings: Settings = match serde_json::from_value(merged) {
        Ok(settings) => settings,
        Err(error) => return recover_invalid_config(path, format!("配置项不合法: {error}")),
    };
    settings.overlay.normalize_interaction();
    settings.assistant.normalize_voices();
    migrate_legacy_engine_ids(&mut settings);
    settings.post_processing.normalize_active_id();
    // 快捷指令搬进热键表（ADR 040 第 3 期）：表里没有这个 id 的加进去，有的以表为准；再照表写回旧字段
    settings.adopt_quick_messages_on_load();
    settings.sync_hotkey_mirrors();
    SettingsLoadResult {
        settings,
        warning: None,
    }
}

/// 0.1.x 从未写过 Agent、工具服务或呼叫键迁移标记。已试用过 0.2 的配置保留现状。
///
/// 提醒助手退掉以后（ADR 052），她的连接和呼叫键只在 `assistant.connectionId` / `assistant.hotkey`：
/// - `optedIn`（启用了她）：两个字段换成 `companionBindings` 里她自己的，`legacyUpgrade` 写成 `notNeeded`；
/// - `pending` / `dismissed`（还没启用她）：两个字段原来是旧提醒助手的连接和键，清空，旧连接不会变成她的模型
///   （连接本身还在「API 连接」里）；以前启用过又选回只用语音输入的，`companionBindings` 一样去掉（再启用时重新设）。
///
/// `companionBindings` 读完就去掉。只写旧版本认识的值：两个字符串字段、`legacyUpgrade` 的 `notNeeded`。
fn migrate_legacy_upgrade_json(user: &mut serde_json::Value) {
    let Some(object) = user.as_object_mut() else {
        return;
    };
    let has_agent_settings = object.contains_key("agent")
        || object.contains_key("toolService")
        || object
            .get("assistant")
            .and_then(|assistant| assistant.get("callHotkeyDefaulted"))
            .is_some();
    let Some(ui) = object
        .entry("ui")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return;
    };
    let opted_in = match ui.get("legacyUpgrade").and_then(|value| value.as_str()) {
        None if !has_agent_settings => {
            ui.insert("legacyUpgrade".into(), "pending".into());
            false
        }
        Some("pending" | "dismissed") => false,
        Some("optedIn") => {
            ui.insert("legacyUpgrade".into(), "notNeeded".into());
            true
        }
        _ => return,
    };
    let Some(assistant) = object
        .entry("assistant")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return;
    };
    let bindings = assistant.remove("companionBindings");
    let hers = |field: &str| {
        bindings
            .as_ref()
            .and_then(|bindings| bindings.get(field))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    };
    // 还没启用她的一律清空：每次读都会走到这里，留着 `companionBindings` 里的值下次读又会被清掉，不如说一不二
    let (hotkey, connection_id) = if opted_in {
        (hers("hotkey"), hers("connectionId"))
    } else {
        (String::new(), String::new())
    };
    assistant.insert("hotkey".into(), hotkey.into());
    assistant.insert("connectionId".into(), connection_id.into());
}

/// 默认呼叫键从 ` 改成 Ctrl+`（[`crate::assistant::CALL_HOTKEY_REVISION`]）：更早的配置里还是旧默认的
/// 呼叫键跟着改，并记上版本，之后玩家自己设回 ` 不会再被改。在 [`migrate_legacy_upgrade_json`] 之后跑：
/// 0.1.x 升级用户的呼叫键那时已经搬到 `assistant.hotkey`（还没启用她的是空的）。
fn migrate_call_hotkey_json(user: &mut serde_json::Value) {
    use crate::assistant::{CALL_HOTKEY_REVISION, DEFAULT_CALL_HOTKEY, OLD_DEFAULT_CALL_HOTKEY};
    let Some(assistant) = user
        .get_mut("assistant")
        .and_then(|assistant| assistant.as_object_mut())
    else {
        return;
    };
    let revision = assistant
        .get("callHotkeyRevision")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    if revision >= u64::from(CALL_HOTKEY_REVISION) {
        return;
    }
    if let Some(value) = assistant
        .get_mut("hotkey")
        .filter(|value| value.as_str() == Some(OLD_DEFAULT_CALL_HOTKEY))
    {
        *value = DEFAULT_CALL_HOTKEY.into();
    }
    assistant.insert("callHotkeyRevision".into(), CALL_HOTKEY_REVISION.into());
}

/// 通话（ADR 033）取代“回答完接着听”：以前把接着听关掉（`followUpSeconds: 0`）的玩家不想她一直听，
/// 新字段缺省成“不通话”；其余用缺省 30 秒。已经有 `callIdleSeconds` 的不动。
fn migrate_call_idle_json(user: &mut serde_json::Value) {
    let Some(assistant) = user
        .get_mut("assistant")
        .and_then(|assistant| assistant.as_object_mut())
    else {
        return;
    };
    if assistant.contains_key("callIdleSeconds") {
        return;
    }
    if assistant.get("followUpSeconds").and_then(|v| v.as_u64()) == Some(0) {
        assistant.insert("callIdleSeconds".into(), 0.into());
    }
}

/// 呼叫以后怎么听的缺省改成了自动（ADR 054），只给新装：配置文件里没有 `assistant.callMode` 的老用户照旧是说一句就走，
/// 读取时写明（旧版本认识这个值）。玩家选过的不动。
fn migrate_call_mode_json(user: &mut serde_json::Value) {
    let Some(object) = user.as_object_mut() else {
        return;
    };
    let Some(assistant) = object
        .entry("assistant")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return;
    };
    assistant
        .entry("callMode")
        .or_insert_with(|| "one-shot".into());
}

/// 插件重构（ADR 038）：旧的 `lol.enabled`（ADR 035，英雄联盟对局数据）搬到 `plugins.lol.sources.client`
/// （英雄联盟插件的“读客户端数据”开关）。已经有新字段的按新字段；旧段落读完就丢掉，下次保存不再写
/// （旧版本读不到 `lol` 时按默认“开”，这是装回旧版的代价）。
pub fn migrate_plugins_json(user: &mut serde_json::Value) {
    migrate_voice_chat_json(user);
    migrate_music_json(user);
    let Some(object) = user.as_object_mut() else {
        return;
    };
    let Some(old) = object.remove("lol") else {
        return;
    };
    let Some(enabled) = old.get("enabled").and_then(|v| v.as_bool()) else {
        return;
    };
    let plugins = object
        .entry("plugins")
        .or_insert_with(|| serde_json::json!({}));
    let Some(lol) = plugins
        .as_object_mut()
        .map(|p| p.entry("lol").or_insert_with(|| serde_json::json!({})))
        .and_then(|lol| lol.as_object_mut())
    else {
        return;
    };
    let Some(sources) = lol
        .entry("sources")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return;
    };
    sources.entry("client").or_insert(enabled.into());
}

/// 放歌（`music` 段：音质、音量，ADR 034）搬进放歌插件（ADR 038 第 4 步）：→ `plugins.music.settings`；
/// 左侧栏里的音乐页（`music`）成了插件页（`plugin:music`）。新字段已经有了的不覆盖。
fn migrate_music_json(user: &mut serde_json::Value) {
    let Some(object) = user.as_object_mut() else {
        return;
    };
    if let Some(serde_json::Value::Object(old)) = object.remove("music") {
        if !old.is_empty() {
            let plugins = object
                .entry("plugins")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(settings) = plugins
                .as_object_mut()
                .map(|p| p.entry("music").or_insert_with(|| serde_json::json!({})))
                .and_then(|p| p.as_object_mut())
                .map(|p| p.entry("settings").or_insert_with(|| serde_json::json!({})))
                .and_then(|s| s.as_object_mut())
            {
                for (key, value) in old {
                    settings.entry(key).or_insert(value);
                }
            }
        }
    }
    let sidebar = object
        .get_mut("ui")
        .and_then(|ui| ui.get_mut("sidebar"))
        .and_then(|s| s.as_object_mut());
    if let Some(sidebar) = sidebar {
        for key in ["order", "hidden"] {
            if let Some(serde_json::Value::Array(ids)) = sidebar.get_mut(key) {
                for id in ids.iter_mut() {
                    if id.as_str() == Some("music") {
                        *id = serde_json::Value::String("plugin:music".into());
                    }
                }
            }
        }
    }
}

/// 0.2 以前的游戏语音（`voiceChat` 段，ADR 020）搬进游戏语音插件（ADR 038 第 4 步）：
/// `voiceChat.enabled` → `plugins.game-voice.enabled`，其余字段（自己的麦克风、音量、记下的默认麦克风）
/// → `plugins.game-voice.settings`。新字段已经有了的不覆盖。
fn migrate_voice_chat_json(user: &mut serde_json::Value) {
    let Some(object) = user.as_object_mut() else {
        return;
    };
    let Some(serde_json::Value::Object(mut old)) = object.remove("voiceChat") else {
        return;
    };
    let enabled = old.remove("enabled");
    let plugins = object
        .entry("plugins")
        .or_insert_with(|| serde_json::json!({}));
    let Some(plugin) = plugins
        .as_object_mut()
        .map(|p| {
            p.entry("game-voice")
                .or_insert_with(|| serde_json::json!({}))
        })
        .and_then(|p| p.as_object_mut())
    else {
        return;
    };
    if let Some(enabled @ serde_json::Value::Bool(_)) = enabled {
        plugin.entry("enabled").or_insert(enabled);
    }
    if old.is_empty() {
        return;
    }
    let Some(settings) = plugin
        .entry("settings")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    else {
        return;
    };
    for (key, value) in old {
        settings.entry(key).or_insert(value);
    }
}

/// 语音输入插件的设置（ADR 038 第 5 步，读旧写新）：文字处理、识别记录、悬浮窗和语音输入自己的几个热键。
/// 配置文件里写在 `plugins.voice-input.settings` 下面；读进来时搬回顶层，程序里仍是 [`Settings`] 上的字段
/// （本体的“听一句”、宿主和 `tests/orchestrator.rs` 的检查都直接读它们）。
/// 装回旧版本时旧版本找不到这几项，会按它的默认值来（和 `lol.enabled`、`voiceChat`、`music` 搬家一样的代价）。
pub const VOICE_INPUT_SETTINGS: &[&str] = &[
    "postProcessing",
    "history",
    "overlay",
    "channelCycleHotkey",
    "resendLastHotkey",
    "togglePostProcessingHotkey",
    "cyclePostProcessingHotkey",
];
const VOICE_INPUT_PLUGIN: &str = "voice-input";

/// 读：`plugins.voice-input.settings` 里有的搬回顶层（新位置为准，盖过旧版本写的同名顶层字段）；
/// 没有的用顶层的旧字段。
pub fn lift_voice_input_json(user: &mut serde_json::Value) {
    let Some(object) = user.as_object_mut() else {
        return;
    };
    let lifted: Vec<(String, serde_json::Value)> = object
        .get_mut("plugins")
        .and_then(|p| p.get_mut(VOICE_INPUT_PLUGIN))
        .and_then(|p| p.get_mut("settings"))
        .and_then(|s| s.as_object_mut())
        .map(|settings| {
            VOICE_INPUT_SETTINGS
                .iter()
                .filter_map(|key| settings.remove(*key).map(|v| (key.to_string(), v)))
                .collect()
        })
        .unwrap_or_default();
    for (key, value) in lifted {
        object.insert(key, value);
    }
}

/// 写：顶层的这几项搬进 `plugins.voice-input.settings`（插件自己的其余设置、开关不动）。
pub fn stash_voice_input_json(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let moved: Vec<(&str, serde_json::Value)> = VOICE_INPUT_SETTINGS
        .iter()
        .filter_map(|key| object.remove(*key).map(|v| (*key, v)))
        .collect();
    if moved.is_empty() {
        return;
    }
    let plugins = object
        .entry("plugins")
        .or_insert_with(|| serde_json::json!({}));
    let Some(settings) = plugins
        .as_object_mut()
        .map(|p| {
            p.entry(VOICE_INPUT_PLUGIN)
                .or_insert_with(|| serde_json::json!({}))
        })
        .and_then(|p| p.as_object_mut())
        .map(|p| p.entry("settings").or_insert_with(|| serde_json::json!({})))
        .and_then(|s| s.as_object_mut())
    else {
        return;
    };
    for (key, value) in moved {
        settings.insert(key.to_string(), value);
    }
}

/// 旧配置只有单条 `pipeline`：空流程改种默认屏蔽词；有步骤则收成一条可切换流程。
fn migrate_legacy_post_processing_json(user: &mut serde_json::Value) {
    let Some(post) = user.get("postProcessing") else {
        return;
    };
    if post.get("pipelines").is_some() {
        return;
    }
    let enabled = post.get("enabled").and_then(|value| value.as_bool());
    let legacy = post
        .get("pipeline")
        .cloned()
        .and_then(|value| serde_json::from_value::<crate::postprocess::PipelineConfig>(value).ok());
    match legacy {
        Some(pipeline) if !pipeline.steps.is_empty() => {
            if let Some(post) = user.get_mut("postProcessing") {
                *post =
                    serde_json::to_value(crate::postprocess::PostProcessingConfig::with_pipeline(
                        enabled.unwrap_or(true),
                        pipeline,
                    ))
                    .unwrap_or_else(|_| serde_json::json!({}));
            }
        }
        _ => {
            if let Some(object) = user.as_object_mut() {
                object.remove("postProcessing");
            }
        }
    }
}

/// 旧家族引擎 id / `online-asr` 伞：加载时改写成现在的引擎 id，并把旧键选项抄到新键。
fn migrate_legacy_engine_ids(s: &mut Settings) {
    let from = s.stt_engine.clone();
    let to = match from.as_str() {
        "sherpa-onnx-x-asr-zh-en" => "sherpa-streaming",
        "sherpa-onnx-sensevoice" | "sherpa-onnx-funasr-nano" => "sherpa-offline",
        "online-asr" => {
            let hinted = s
                .engine_options
                .get("online-asr")
                .and_then(|v| v.get("model"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if s.active_model_id == "volcano-asr" || hinted == "volcano-asr" {
                "volcano-asr"
            } else if s.active_model_id == "iflytek-asr" || hinted == "iflytek-asr" {
                "iflytek-asr"
            } else {
                "volcano-asr"
            }
        }
        _ => return,
    };
    copy_engine_options_key(&mut s.engine_options, &from, to);
    if s.active_model_id.trim().is_empty() {
        if let Some(model) = s
            .engine_options
            .get(&from)
            .and_then(|v| v.get("model"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            s.active_model_id = model.to_string();
        }
    }
    s.stt_engine = to.to_string();
}

fn copy_engine_options_key(opts: &mut serde_json::Value, from: &str, to: &str) {
    let Some(src) = opts.get(from).cloned() else {
        return;
    };
    if from == to {
        return;
    }
    if !opts.is_object() {
        *opts = serde_json::json!({});
    }
    let Some(base) = opts.as_object_mut() else {
        return;
    };
    let dest = base
        .entry(to.to_string())
        .or_insert_with(|| serde_json::json!({}));
    merge_json_objects(dest, &src);
}

fn recover_invalid_config(path: &Path, reason: String) -> SettingsLoadResult {
    let backup = next_corrupt_path(path);
    let defaults = Settings::default();
    let warning = match std::fs::rename(path, &backup) {
        Ok(()) => {
            let save_note = save_to(path, &defaults)
                .err()
                .map(|error| format!("；默认配置写入失败: {error}"))
                .unwrap_or_default();
            format!(
                "配置文件损坏，已恢复默认配置。原文件已保留为 {}（{reason}）{save_note}",
                backup.display()
            )
        }
        Err(backup_error) => format!(
            "配置文件无法读取，已在本次运行中使用默认配置，但未覆盖原文件（{reason}；备份失败: {backup_error}）"
        ),
    };
    crate::log::log(&format!("settings recovery: {warning}"));
    SettingsLoadResult {
        settings: defaults,
        warning: Some(warning),
    }
}

fn next_corrupt_path(path: &Path) -> PathBuf {
    let first = path.with_extension("json.corrupt");
    if !first.exists() {
        return first;
    }
    for index in 1..=9999 {
        let candidate = path.with_extension(format!("json.corrupt.{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.with_extension(format!("json.corrupt.{}", std::process::id()))
}

/// 写入指定路径（原子写入）
pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    let save_lock = settings_file_lock(path);
    let _save = save_lock.lock().unwrap();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    // 热键的旧字段是表的镜像：有代码直接改了旧字段（没经过 reconcile）时，先算进表里（ADR 040 D8）
    let fixed = crate::hotkey_bindings::with_direct_edits(settings);
    let settings = fixed.as_ref().unwrap_or(settings);
    let mut value = serde_json::to_value(settings).map_err(|e| format!("序列化配置失败: {e}"))?;
    stash_voice_input_json(&mut value);
    let json = serde_json::to_string_pretty(&value).map_err(|e| format!("序列化配置失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入配置失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘配置失败: {e}"))?;
    Ok(())
}

fn settings_file_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    locks
        .lock()
        .unwrap()
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 浅层递归合并：patch 中的对象键覆盖 base，其余类型整体替换
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(slot) => merge_json(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试程序拿不到玩家真实的 `~/.kotone`：没设 `KOTONE_HOME` 时是这个测试进程自己的临时目录。
    #[test]
    fn test_binary_never_resolves_the_real_kotone_home() {
        let home = test_process_home().expect("cargo 的测试程序应被认出来");
        assert!(
            home.ends_with(Path::new("kotone-test-home").join(std::process::id().to_string())),
            "{}",
            home.display()
        );
        let real = dirs::home_dir().unwrap().join(".kotone");
        assert_ne!(home, &real);
        if !matches!(std::env::var_os(HOME_ENV), Some(v) if !v.is_empty()) {
            assert_eq!(kotone_dir(), *home);
            assert_eq!(isolated_home().as_ref(), Some(home));
            assert_eq!(config_path(), home.join("config.json"));
        }
    }

    /// 只认 cargo 的测试程序（新旧两种布局、doctest）；`pnpm dev` 和真机脚本跑的
    /// `target/debug/kotone.exe`、安装版、example 都照旧用 `~/.kotone`。
    #[test]
    fn test_binary_tmp_only_matches_cargo_test_binaries() {
        let temp = Path::new("/tmp-root");
        let tmp = |exe: &str| test_binary_tmp(Path::new(exe), temp);
        let target_tmp = Some(PathBuf::from("/r/target/tmp"));
        assert_eq!(
            tmp("/r/target/debug/deps/orchestrator-0123456789abcdef.exe"),
            target_tmp
        );
        assert_eq!(
            tmp("/r/target/debug/build/kotone-core/0123456789abcdef/out/kotone_core-0123456789abcdef.exe"),
            target_tmp
        );
        assert_eq!(
            tmp("/r/target/x86_64-pc-windows-msvc/release/deps/history-fedcba9876543210"),
            Some(PathBuf::from("/r/target/x86_64-pc-windows-msvc/tmp"))
        );
        assert_eq!(
            tmp("/tmp-root/rustdoctestAbC123/rust_out.exe"),
            Some(temp.to_path_buf())
        );
        for not_test in [
            "/r/target/debug/kotone.exe",
            "/r/target/debug/kotone-cli.exe",
            "/r/target/release/kotone.exe",
            "/r/target/debug/examples/stdio_server.exe",
            "/r/target/debug/deps/kotone-cli.exe",
            "/r/target/debug/deps/kotone-0123456789abcdeg.exe",
            "/r/target/debug/build/kotone-core/0123456789abcdef/out/kotone_core-fedcba9876543210.exe",
            "/r/target/debug/other/kotone-core/0123456789abcdef/out/kotone_core-0123456789abcdef.exe",
            "/Program Files/Kotone/kotone.exe",
            "/Users/A/AppData/Local/Kotone/kotone-0123456789abcdef.exe",
        ] {
            assert_eq!(tmp(not_test), None, "{not_test}");
        }
    }

    /// 放歌和左侧栏是新字段（ADR 034）：旧配置没有它们时用默认；旧的 `agent.musicSite` 读到就丢掉；
    /// 不认识的音质按默认；“设置”不能隐藏、重复的去掉。
    #[test]
    fn sidebar_is_a_new_section_and_music_site_is_dropped() {
        // 旧配置：有 agent.musicSite，没有 ui.sidebar
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json["agent"]["musicSite"] = serde_json::json!("qq");
        json["ui"].as_object_mut().unwrap().remove("sidebar");
        let old: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(old.ui.sidebar, SidebarConfig::default());
        assert!(!serde_json::to_string(&old).unwrap().contains("musicSite"));
        let mut sidebar = SidebarConfig {
            order: vec![
                "plugin:music".into(),
                "plugin:music".into(),
                " ".into(),
                "assistant".into(),
            ],
            hidden: vec!["advanced".into(), "general".into()],
        };
        sidebar.normalize();
        assert_eq!(sidebar.order, vec!["plugin:music", "assistant"]);
        assert_eq!(sidebar.hidden, vec!["general"]);
    }

    /// 放歌搬进插件（ADR 038 第 4 步）：旧的 `music` 段搬进 `plugins.music.settings`，左侧栏的 `music`
    /// 换成 `plugin:music`；新字段已经有了的不覆盖。
    #[test]
    fn admin_prompt_revision_is_a_new_field_and_old_answers_are_kept() {
        // ADR 044 D1：旧配置没有 adminPromptRevision = 0（还没用新说法问过）；写过的 runAsAdminOnStart 照原样读，
        // 没写的按新默认（开）；两个旧的“不再提示”照原样留着（只加不改，原则 19）
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"runAsAdminOnStart":false,"adminPromptDismissed":true,"autoAdminPromptDismissed":true}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert!(!s.run_as_admin_on_start);
        assert_eq!(s.admin_prompt_revision, 0);
        assert!(s.admin_prompt_dismissed && s.auto_admin_prompt_dismissed);
        std::fs::write(&path, r#"{"activeProfileId":"lol"}"#).unwrap();
        let s = load_from(&path);
        assert!(s.run_as_admin_on_start, "没写的按新默认");
        assert_eq!(s.admin_prompt_revision, 0);
        // 记下问过：写回去、读回来
        let mut answered = s.clone();
        answered.admin_prompt_revision = 1;
        answered.run_as_admin_on_start = false;
        let json = serde_json::to_value(&answered).unwrap();
        assert_eq!(json["adminPromptRevision"], 1);
        assert_eq!(json["runAsAdminOnStart"], false);
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back.admin_prompt_revision, 1);
        assert!(!back.run_as_admin_on_start);
    }

    #[test]
    fn old_music_settings_and_sidebar_move_into_the_music_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"music":{"quality":"lossless","volume":12},"ui":{"sidebar":{"order":["music","assistant"],"hidden":["music"]}}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        let settings = &s.plugins["music"].settings;
        assert_eq!(settings["quality"], "lossless");
        assert_eq!(settings["volume"], 12);
        assert!(s.plugin_enabled("music", true), "开关没写：按清单默认");
        assert_eq!(s.ui.sidebar.order, vec!["plugin:music", "assistant"]);
        assert_eq!(s.ui.sidebar.hidden, vec!["plugin:music"]);
        let text = serde_json::to_string(&s).unwrap();
        assert!(!text.contains(r#""music":{"quality""#), "{text}");

        std::fs::write(
            &path,
            r#"{"music":{"volume":12},"plugins":{"music":{"settings":{"volume":40}}}}"#,
        )
        .unwrap();
        assert_eq!(load_from(&path).plugins["music"].settings["volume"], 40);

        // 没有旧字段：不写空段
        assert!(!serde_json::to_string(&Settings::default())
            .unwrap()
            .contains(r#""music""#));
    }

    /// 插件（ADR 038）：旧的 `lol.enabled` 读进来时搬到 `plugins.lol.sources.client`，之后不再写 `lol`；
    /// 已经有新字段的不被旧字段覆盖；没写的开关按清单默认。
    #[test]
    fn old_voice_chat_moves_into_the_game_voice_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"voiceChat":{"enabled":true,"includeMic":false,"micGain":150,"previousDefaultMic":"{0.0.1}"}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert!(s.plugin_enabled("game-voice", false));
        let settings = &s.plugins["game-voice"].settings;
        assert_eq!(settings["includeMic"], false);
        assert_eq!(settings["micGain"], 150);
        assert_eq!(settings["previousDefaultMic"], "{0.0.1}");
        assert!(!settings.contains_key("enabled"));
        let text = serde_json::to_string(&s).unwrap();
        assert!(!text.contains("voiceChat"), "{text}");

        // 旧版里关着（默认）：插件也关着
        std::fs::write(&path, r#"{"voiceChat":{"enabled":false}}"#).unwrap();
        let s = load_from(&path);
        assert!(!s.plugin_enabled("game-voice", true));
        assert!(s.plugins["game-voice"].settings.is_empty());

        // 新字段已经有了：旧字段不覆盖
        std::fs::write(
            &path,
            r#"{"voiceChat":{"enabled":true,"micGain":150},"plugins":{"game-voice":{"enabled":false,"settings":{"micGain":80}}}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert!(!s.plugin_enabled("game-voice", true));
        assert_eq!(s.plugins["game-voice"].settings["micGain"], 80);

        // 没有旧字段：不写空段
        let fresh = Settings::default();
        assert!(!serde_json::to_string(&fresh)
            .unwrap()
            .contains("game-voice"));
    }

    #[test]
    fn old_lol_switch_moves_into_the_lol_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"lol":{"enabled":false}}"#).unwrap();
        let s = load_from(&path);
        assert!(!s.plugin_source_enabled("lol", "client", true));
        assert!(s.plugin_source_enabled("lol", "stats", true));
        assert!(s.plugin_enabled("lol", true));
        let text = serde_json::to_string(&s).unwrap();
        assert!(!text.contains("\"lol\":{\"enabled\""), "{text}");
        assert!(
            text.contains(r#""plugins":{"lol":{"sources":{"client":false}}}"#),
            "{text}"
        );

        // 新字段已经有了：旧字段不覆盖
        std::fs::write(
            &path,
            r#"{"lol":{"enabled":false},"plugins":{"lol":{"sources":{"client":true},"settings":{"x":1}}}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert!(s.plugin_source_enabled("lol", "client", false));
        assert_eq!(s.plugins["lol"].settings["x"], 1);

        // 旧版里开着（默认）：也搬过去，等于默认
        std::fs::write(&path, r#"{"lol":{"enabled":true}}"#).unwrap();
        assert!(load_from(&path).plugin_source_enabled("lol", "client", false));

        // 没有旧字段、没有新字段：按清单默认，不写空段
        let fresh = Settings::default();
        assert!(fresh.plugins.is_empty());
        assert!(fresh.plugin_enabled("music", true));
        assert!(!fresh.plugin_enabled("music", false));
        assert!(serde_json::to_string(&fresh)
            .unwrap()
            .contains(r#""plugins":{}"#));
    }

    /// 插件的“风险和许可”弹窗看过第几版（ADR 043 D8）：新字段，旧配置读进来是 0（没看过）、不写出去；
    /// 记过的写回去、读回来；别的字段照旧。
    #[test]
    fn plugin_risk_seen_is_a_new_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"plugins":{"lol":{"sources":{"control":true},"pushes":{"opening":false}}}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.plugins["lol"].risk_seen, 0);
        assert!(s.plugin_source_enabled("lol", "control", false));
        let text = serde_json::to_string(&s).unwrap();
        assert!(!text.contains("riskSeen"), "{text}");

        let mut seen = s.clone();
        seen.plugins.get_mut("lol").unwrap().risk_seen = 1;
        let json = serde_json::to_value(&seen).unwrap();
        assert_eq!(json["plugins"]["lol"]["riskSeen"], 1);
        assert_eq!(json["plugins"]["lol"]["pushes"]["opening"], false);
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back.plugins["lol"].risk_seen, 1);
        assert_eq!(back.plugin_push("lol", "opening"), Some(false));
    }

    /// 浏览器插件是新字段（ADR 031）：旧配置没有它时为空；写进去的能原样读回；只写了路径的条目默认启用。
    #[test]
    fn browser_extensions_are_a_new_optional_section() {
        let old: Settings = serde_json::from_value(serde_json::json!({
            "hotkey": {"key": "F8", "mode": "hold"},
            "audioDeviceId": "",
            "sttEngine": "",
            "engineOptions": {},
            "autoSend": true,
            "activeProfileId": null,
            "language": "zh",
            "evalRecording": false,
            "runAsAdminOnStart": false
        }))
        .unwrap();
        assert!(old.browser.extensions.is_empty() && old.browser.pending_uninstall.is_empty());
        let mut s = Settings::default();
        s.browser.extensions.push(BrowserExtensionConfig {
            id: "abc".into(),
            path: r"C:\ext\adblock".into(),
            name: "去广告".into(),
            enabled: false,
            installed_id: Some("edge-id".into()),
        });
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["browser"]["extensions"][0]["installedId"], "edge-id");
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back.browser, s.browser);
        let minimal: BrowserExtensionConfig =
            serde_json::from_value(serde_json::json!({"path": "C:/ext"})).unwrap();
        assert!(minimal.enabled && minimal.installed_id.is_none());
    }

    /// 内置浏览器的开关（设置 → 浏览器，ADR 038）是新字段：默认开；旧配置没有 `browser` 段、或者段里只有扩展
    /// （0.2.0-rc 写的）都按开着读，扩展原样留着；关掉写成 `browser.enabled = false`，能读回。
    #[test]
    fn browser_switch_is_a_new_field_that_defaults_to_on() {
        assert!(Settings::default().browser.enabled);
        assert_eq!(
            serde_json::to_value(Settings::default()).unwrap()["browser"]["enabled"],
            true
        );
        let base = serde_json::json!({
            "hotkey": {"key": "F8", "mode": "hold"},
            "audioDeviceId": "",
            "sttEngine": "",
            "engineOptions": {},
            "autoSend": true,
            "activeProfileId": null,
            "language": "zh",
            "evalRecording": false,
            "runAsAdminOnStart": false
        });
        let old: Settings = serde_json::from_value(base.clone()).unwrap();
        assert!(old.browser.enabled);
        let mut rc = base.clone();
        rc["browser"] =
            serde_json::json!({"extensions": [{"path": "C:/ext/adblock", "name": "去广告"}]});
        let rc: Settings = serde_json::from_value(rc).unwrap();
        assert!(rc.browser.enabled);
        assert_eq!(rc.browser.extensions[0].name, "去广告");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut off = base;
        off["browser"] = serde_json::json!({"enabled": false});
        std::fs::write(&path, off.to_string()).unwrap();
        let loaded = load_from(&path);
        assert!(!loaded.browser.enabled);
        assert!(loaded.browser.extensions.is_empty());
        let json = serde_json::to_value(&loaded).unwrap();
        assert_eq!(json["browser"]["enabled"], false);
    }

    const OLD_QWEN: &str = r#"{"id":"old","displayName":"千问","provider":"dashscope","baseUrl":"https://dashscope.aliyuncs.com/compatible-mode/v1","model":"qwen-plus"}"#;

    #[test]
    fn old_profiles_keep_their_voice_input_settings_but_not_the_reminder_key_or_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let original: serde_json::Value = serde_json::from_str(&format!(
            r#"{{
                "hotkey": {{"key": "F8", "mode": "hold"}},
                "audioDeviceId": "Test microphone",
                "assistant": {{"hotkey": "F9", "interactionMode": "dictation", "connectionId": "old",
                              "speechEngine": "kokoro", "defaultDelivery": ["danmaku"]}},
                "connections": [{OLD_QWEN}],
                "ui": {{"firstRunCompleted": false, "autoStart": true}},
                "lowerSpeakerVolumeWhileListening": true
            }}"#
        ))
        .unwrap();
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let old = load_from(&path);
        assert_eq!(old.ui.legacy_upgrade, LegacyUpgrade::Pending);
        assert!(!old.ui.first_run_completed);
        assert!(old.ui.auto_start);
        assert_eq!(old.hotkey.key, "F8");
        assert_eq!(old.audio_device_id, "Test microphone");
        // 旧提醒助手的键和连接不留给她（ADR 052）；连接本身还在「API 连接」里
        assert_eq!(old.assistant.hotkey, "");
        assert_eq!(old.assistant.connection_id, "");
        assert_eq!(old.call_hotkey(), "");
        assert_eq!(old.companion_connection_id(), "");
        assert_eq!(old.connections.len(), 1);
        assert_eq!(old.connections[0].id, "old");
        assert_eq!(
            old.assistant.speech_engine,
            crate::assistant::SpeechEngine::Kokoro
        );
        assert_eq!(
            old.assistant.default_delivery,
            vec![crate::reminders::DeliveryKind::Danmaku]
        );
        assert!(old.lower_speaker_volume_while_listening);
        // 读取不改文件
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            original
        );
    }

    #[test]
    fn dismissed_profiles_lose_the_reminder_key_and_connection_and_stay_voice_input_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"ui":{{"legacyUpgrade":"dismissed","startPage":"voiceInput"}},
                    "assistant":{{"hotkey":"F9","connectionId":"old"}},"connections":[{OLD_QWEN}]}}"#
            ),
        )
        .unwrap();
        let dismissed = load_from(&path);
        assert_eq!(dismissed.ui.legacy_upgrade, LegacyUpgrade::Dismissed);
        assert_eq!(dismissed.ui.start_page, "voiceInput");
        assert_eq!(dismissed.assistant.hotkey, "");
        assert_eq!(dismissed.assistant.connection_id, "");
        assert_eq!(dismissed.call_hotkey(), "");
        assert_eq!(dismissed.connections[0].id, "old");
        // 存一次再读：还是只用语音输入，旧连接还是不是她的
        save_to(&path, &dismissed).unwrap();
        let again = load_from(&path);
        assert_eq!(again.ui.legacy_upgrade, LegacyUpgrade::Dismissed);
        assert_eq!(again.assistant.connection_id, "");
    }

    #[test]
    fn opted_in_profiles_move_her_connection_and_call_key_into_assistant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"ui":{{"legacyUpgrade":"optedIn"}},"connections":[{OLD_QWEN}],
                    "assistant":{{"hotkey":"F9","connectionId":"old","interactionMode":"dictation",
                                 "companionBindings":{{"hotkey":"Ctrl+Backquote","connectionId":"game"}}}}}}"#
            ),
        )
        .unwrap();
        let opted_in = load_from(&path);
        assert_eq!(opted_in.ui.legacy_upgrade, LegacyUpgrade::NotNeeded);
        assert_eq!(opted_in.assistant.hotkey, "Ctrl+Backquote");
        assert_eq!(opted_in.assistant.connection_id, "game");
        assert_eq!(opted_in.call_hotkey(), "Ctrl+Backquote");
        assert_eq!(opted_in.companion_connection_id(), "game");
        assert_eq!(opted_in.connections[0].id, "old");
        save_to(&path, &opted_in).unwrap();
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["ui"]["legacyUpgrade"], "notNeeded");
        assert_eq!(disk["assistant"]["hotkey"], "Ctrl+Backquote");
        assert_eq!(disk["assistant"]["connectionId"], "game");
        assert!(disk["assistant"].get("companionBindings").is_none());
        assert_eq!(load_from(&path).companion_connection_id(), "game");
    }

    #[test]
    fn opted_in_without_bindings_never_hands_her_the_reminder_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            br#"{"ui":{"legacyUpgrade":"optedIn"},"assistant":{"hotkey":"F9","connectionId":"old"}}"#,
        )
        .unwrap();
        let opted_in = load_from(&path);
        assert_eq!(opted_in.ui.legacy_upgrade, LegacyUpgrade::NotNeeded);
        assert_eq!(opted_in.assistant.hotkey, "");
        assert_eq!(opted_in.assistant.connection_id, "");
    }

    #[test]
    fn going_back_to_voice_input_clears_her_bindings_too() {
        // 启用过她、又选回“继续使用语音输入法”：她不出声，companionBindings 一样去掉，再启用时重新设
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            br#"{"ui":{"legacyUpgrade":"dismissed"},"assistant":{"hotkey":"F9","connectionId":"old",
                "companionBindings":{"hotkey":"Ctrl+Backquote","connectionId":"game"}}}"#,
        )
        .unwrap();
        let dismissed = load_from(&path);
        assert_eq!(dismissed.ui.legacy_upgrade, LegacyUpgrade::Dismissed);
        assert_eq!(dismissed.assistant.hotkey, "");
        assert_eq!(dismissed.assistant.connection_id, "");
        assert_eq!(dismissed.call_hotkey(), "");
        save_to(&path, &dismissed).unwrap();
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(disk["assistant"].get("companionBindings").is_none());
    }

    #[test]
    fn opted_in_written_by_an_older_build_still_reads() {
        // 设置接口传进来的旧值（不经过读文件的迁移）也认得
        let ui: UiConfig = serde_json::from_str(r#"{"legacyUpgrade":"optedIn"}"#).unwrap();
        assert_eq!(ui.legacy_upgrade, LegacyUpgrade::NotNeeded);
        assert_eq!(
            serde_json::to_value(LegacyUpgrade::NotNeeded).unwrap(),
            "notNeeded"
        );
    }

    #[test]
    fn turning_follow_up_off_also_turns_calls_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        for (raw, expected) in [
            (r#"{"assistant":{"followUpSeconds":0}}"#, 0),
            (r#"{"assistant":{"followUpSeconds":8}}"#, 30),
            (r#"{"assistant":{"voiceId":"v"}}"#, 30),
            (
                r#"{"assistant":{"followUpSeconds":0,"callIdleSeconds":45}}"#,
                45,
            ),
        ] {
            std::fs::write(&path, raw).unwrap();
            assert_eq!(
                load_from(&path).assistant.call_idle_seconds,
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_old_default_call_key_moves_to_ctrl_backquote_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // rc1 的默认 `：改成 Ctrl+`（英雄联盟的“只攻击英雄”键不再被吞）。
        std::fs::write(
            &path,
            br#"{"ui":{"firstRunCompleted":true,"legacyUpgrade":"notNeeded"},"assistant":{"hotkey":"Backquote","callHotkeyDefaulted":true}}"#,
        )
        .unwrap();
        let migrated = load_from(&path);
        assert_eq!(migrated.assistant.hotkey, "Ctrl+Backquote");
        assert_eq!(
            migrated.assistant.call_hotkey_revision,
            crate::assistant::CALL_HOTKEY_REVISION
        );
        // 之后玩家自己设回 `：不再改。
        let mut back = migrated.clone();
        back.assistant.hotkey = "Backquote".into();
        save_to(&path, &back).unwrap();
        assert_eq!(load_from(&path).assistant.hotkey, "Backquote");
        // 自己设过别的键：不动。
        std::fs::write(
            &path,
            br#"{"assistant":{"hotkey":"F9","callHotkeyDefaulted":true}}"#,
        )
        .unwrap();
        assert_eq!(load_from(&path).assistant.hotkey, "F9");
        // 0.1.x 升级用户：启用她以后的呼叫键（以前在 companionBindings）搬过来以后照样改。
        std::fs::write(
            &path,
            br#"{"ui":{"legacyUpgrade":"optedIn"},"assistant":{"hotkey":"F9","companionBindings":{"hotkey":"Backquote","connectionId":"game"}}}"#,
        )
        .unwrap();
        let upgraded = load_from(&path);
        assert_eq!(upgraded.assistant.hotkey, "Ctrl+Backquote");
        assert_eq!(upgraded.call_hotkey(), "Ctrl+Backquote");
        // 全新安装直接是新默认。
        let fresh = load_from(&dir.path().join("fresh.json"));
        assert_eq!(fresh.call_hotkey(), "Ctrl+Backquote");
    }

    #[test]
    fn fresh_profiles_and_previously_opted_in_preview_profiles_keep_new_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let fresh = load_from(&path);
        assert_eq!(fresh.ui.legacy_upgrade, LegacyUpgrade::NotNeeded);
        assert_eq!(
            fresh.assistant.hotkey,
            crate::assistant::DEFAULT_CALL_HOTKEY
        );
        assert_eq!(load_from(&path).ui.legacy_upgrade, LegacyUpgrade::NotNeeded);

        std::fs::write(
            &path,
            br#"{"ui":{"firstRunCompleted":true},"assistant":{"hotkey":"","callHotkeyDefaulted":true}}"#,
        )
        .unwrap();
        let preview = load_from(&path);
        assert_eq!(preview.ui.legacy_upgrade, LegacyUpgrade::NotNeeded);
        assert!(preview.ui.first_run_completed);
        assert!(preview.assistant.hotkey.is_empty());

        std::fs::write(
            &path,
            br#"{"toolService":{"enabled":true},"ui":{"firstRunCompleted":true}}"#,
        )
        .unwrap();
        let isolated_preview = load_from(&path);
        assert_eq!(isolated_preview.ui.legacy_upgrade, LegacyUpgrade::NotNeeded);
        assert!(isolated_preview.tool_service.enabled);

        std::fs::write(&path, br#"{"assistant":{"hotkey":"F9"}}"#).unwrap();
        let old = load_from(&path);
        assert_eq!(old.ui.legacy_upgrade, LegacyUpgrade::Pending);
        assert_eq!(old.assistant.hotkey, "", "旧提醒键不留给她（ADR 052）");
    }

    #[test]
    fn legacy_config_without_interaction_mode_defaults_to_push_to_talk() {
        // 0.1.5 迁移：老配置缺 interactionMode 键 → 合并默认值得到「对讲机」；
        // 显式 null（自定义兼容路径）则原样保留
        let dir = std::env::temp_dir().join(format!("kotone-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let s = load_from(&path);
        assert_eq!(
            s.interaction_mode,
            Some(crate::interaction::InteractionMode::PushToTalk)
        );
        std::fs::write(&path, r#"{"interactionMode": null}"#).unwrap();
        let s = load_from(&path);
        assert_eq!(s.interaction_mode, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_values_match_doc() {
        let s = Settings::default();
        assert_eq!(
            s.assistant.interaction_mode,
            crate::assistant::AssistantInteractionMode::PushToTalk
        );
        assert_eq!(
            s.assistant.speech_engine,
            crate::assistant::SpeechEngine::System
        );
        assert_eq!(s.hotkey.key, "CapsLock");
        assert_eq!(s.hotkey.mode, HotkeyMode::Toggle);
        assert_eq!(s.hotkey_backend, HotkeyBackend::Auto);
        assert_eq!(s.audio_device_id, "default");
        assert!(!s.lower_speaker_volume_while_listening);
        assert_eq!(s.stt_engine, "sherpa-streaming");
        assert!(s.active_model_id.is_empty());
        assert!(s.model_configs.is_empty());
        assert!(!s.auto_send);
        assert_eq!(s.active_profile_id.as_deref(), Some("lol"));
        assert_eq!(s.language, "zh");
        assert!(!s.eval_recording);
        assert_eq!(s.vad_silence_ms, 700);
        assert_eq!(s.vad.threshold, 0.5);
        assert_eq!(s.vad.min_speech_ms, 50);
        assert_eq!(s.vad.min_silence_ms, 50);
        assert_eq!(s.hotwords_score, 3.5);
        assert!(s.engine_options["sherpa-streaming"]["provider"] == "cpu");
        assert_eq!(s.session_options("sherpa-streaming")["provider"], "cpu");
        assert_eq!(
            s.session_options("sherpa-onnx-x-asr-zh-en")["provider"],
            "cpu"
        );
        // ADR 044 D1：默认以管理员身份运行，但还没问过（第一次先说明再弹 UAC）
        assert!(s.run_as_admin_on_start);
        assert_eq!(s.admin_prompt_revision, 0);
        // 新装不再默认带频道切换（ADR 040 D4）；表里正好呼叫键、停下热键、说话三条
        assert_eq!(s.channel_cycle_hotkey, "");
        assert_eq!(s.hotkeys.bindings.len(), 3);
        assert!(!s.admin_prompt_dismissed);
        assert!(!s.auto_admin_prompt_dismissed);
        assert_eq!(s.history.mode, crate::history::HistoryMode::Capped);
        assert_eq!(s.history.max_records, 1000);
        assert!(!s.history.include_audio);
        assert!(!s.ui.first_run_completed);
        assert!(!s.ui.auto_start);
        assert!(
            s.models.dir.is_empty(),
            "默认模型目录为空 = ~/.kotone/models"
        );
        assert_eq!(s.download.source, DownloadSource::Auto);
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
        assert_eq!(s.overlay.visibility, OverlayVisibility::OnDemand);
        assert_eq!(s.overlay.style, OverlayStyle::Capsule);
        assert_eq!(s.overlay.position, OverlayPosition::Auto);
        assert!(!s.overlay.draggable);
        assert!(s.overlay.click_through);
        assert_eq!(s.overlay.custom_x, None);
        assert_eq!(s.overlay.custom_y, None);
        assert!(s.diagnostics.recording);
        assert!(s.post_processing.enabled);
        assert_eq!(s.post_processing.active_pipeline_id, "blocklist");
        assert_eq!(s.post_processing.pipelines.len(), 1);
        assert_eq!(s.post_processing.pipelines[0].display_name, "屏蔽词");
        assert_eq!(
            s.post_processing.pipelines[0].steps[0].processor_id,
            "builtin.blocklist-filter"
        );
        assert!(!s.tool_service.enabled, "工具服务必须默认关闭");
    }

    #[test]
    fn tool_service_setting_defaults_off_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 旧配置缺 toolService 段：合并默认值 = 关闭
        std::fs::write(&path, r#"{"language":"zh"}"#).unwrap();
        let loaded = load_from_with_diagnostic(&path);
        assert!(loaded.warning.is_none());
        assert!(!loaded.settings.tool_service.enabled);

        std::fs::write(&path, r#"{"toolService":{"enabled":true}}"#).unwrap();
        assert!(load_from(&path).tool_service.enabled);

        let mut settings = Settings::default();
        settings.tool_service.enabled = true;
        save_to(&path, &settings).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["toolService"]["enabled"], serde_json::json!(true));
        assert!(load_from(&path).tool_service.enabled);

        // 类型错误不被静默当成开启
        let bad: Result<Settings, _> = serde_json::from_value({
            let mut merged = serde_json::to_value(Settings::default()).unwrap();
            merge_json(
                &mut merged,
                &serde_json::json!({"toolService":{"enabled":"yes"}}),
            );
            merged
        });
        assert!(bad.is_err());

        // 在线 OCR 连接默认未配置，配置后 round-trip
        assert!(Settings::default()
            .tool_service
            .ocr
            .deepseek_connection_id
            .is_none());
        std::fs::write(
            &path,
            r#"{"toolService":{"enabled":true,"ocr":{"deepseekConnectionId":"ds"}}}"#,
        )
        .unwrap();
        assert_eq!(
            load_from(&path)
                .tool_service
                .ocr
                .deepseek_connection_id
                .as_deref(),
            Some("ds")
        );
    }

    #[test]
    fn load_seeds_default_blocklist_pipeline_from_legacy_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"postProcessing":{"enabled":false,"pipeline":{"id":"default","steps":[]}}}"#,
        )
        .unwrap();
        let settings = load_from(&path);
        assert!(settings.post_processing.enabled);
        assert_eq!(settings.post_processing.active_pipeline_id, "blocklist");
        assert_eq!(
            settings.post_processing.pipelines[0].steps[0].processor_id,
            "builtin.blocklist-filter"
        );
    }

    #[test]
    fn load_wraps_legacy_custom_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
              "postProcessing":{
                "enabled":true,
                "pipeline":{
                  "id":"mine",
                  "steps":[{"id":"s1","processorId":"mock.append-exclamation","enabled":true}]
                }
              }
            }"#,
        )
        .unwrap();
        let settings = load_from(&path);
        assert!(settings.post_processing.enabled);
        assert_eq!(settings.post_processing.active_pipeline_id, "mine");
        assert_eq!(settings.post_processing.pipelines.len(), 1);
        assert_eq!(settings.post_processing.pipelines[0].display_name, "mine");
        assert_eq!(
            settings.post_processing.pipelines[0].steps[0].processor_id,
            "mock.append-exclamation"
        );
    }

    #[test]
    fn overlay_visibility_never_roundtrips() {
        let raw = serde_json::json!({ "visibility": "never" });
        let overlay: OverlayConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(overlay.visibility, OverlayVisibility::Never);
        assert_eq!(
            serde_json::to_value(&overlay).unwrap()["visibility"],
            "never"
        );
    }

    #[test]
    fn roundtrip_serialize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.hotkey.key = "Alt+V".into();
        s.hotkey.mode = HotkeyMode::Hold;
        s.auto_send = true;
        s.stt_engine = "mock-stream".into();
        save_to(&path, &s).unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded.hotkey.key, "Alt+V");
        assert_eq!(loaded.hotkey.mode, HotkeyMode::Hold);
        assert!(loaded.auto_send);
        assert_eq!(loaded.stt_engine, "mock-stream");
    }

    #[test]
    fn session_options_merge_family_and_io_keys() {
        let mut s = Settings {
            engine_options: serde_json::json!({
                "sherpa-onnx-sensevoice": { "threads": 2 },
                "sherpa-offline": { "model": "sense-voice-zh-en-ja-ko-yue-2024-07-17" }
            }),
            active_model_id: "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            ..Settings::default()
        };
        s.model_configs.insert(
            "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            serde_json::json!({ "language": "yue" }),
        );
        let opts = s.session_options("sherpa-onnx-sensevoice");
        assert_eq!(opts["threads"], 2);
        assert_eq!(opts["language"], "yue");
        assert_eq!(opts["model"], "sense-voice-zh-en-ja-ko-yue-2024-07-17");
        let via_io = s.session_options("sherpa-offline");
        assert_eq!(via_io["language"], "yue");
        assert_eq!(via_io["model"], "sense-voice-zh-en-ja-ko-yue-2024-07-17");
    }

    #[test]
    fn session_options_do_not_mix_legacy_offline_families() {
        let s = Settings {
            active_model_id: String::new(),
            engine_options: serde_json::json!({
                "sherpa-onnx-sensevoice": {
                    "model": "sense-voice-zh-en-ja-ko-yue-2024-07-17",
                    "language": "yue"
                }
            }),
            ..Settings::default()
        };

        let sensevoice = s.session_options("sherpa-onnx-sensevoice");
        assert_eq!(
            sensevoice["model"],
            "sense-voice-zh-en-ja-ko-yue-2024-07-17"
        );
        assert_eq!(sensevoice["language"], "yue");
    }

    #[test]
    fn load_rewrites_legacy_online_asr_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{ "sttEngine": "online-asr", "activeModelId": "iflytek-asr" }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "iflytek-asr");

        std::fs::write(
            &path,
            r#"{ "sttEngine": "online-asr", "engineOptions": { "online-asr": { "model": "volcano-asr" } } }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "volcano-asr");
    }

    #[test]
    fn load_rewrites_legacy_sherpa_family_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
                "sttEngine": "sherpa-onnx-x-asr-zh-en",
                "engineOptions": {
                    "sherpa-onnx-x-asr-zh-en": { "provider": "cpu", "model": "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05" }
                }
            }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "sherpa-streaming");
        assert_eq!(
            s.active_model_id,
            "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
        );
        assert_eq!(s.engine_options["sherpa-streaming"]["provider"], "cpu");

        std::fs::write(&path, r#"{ "sttEngine": "sherpa-onnx-sensevoice" }"#).unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "sherpa-offline");
    }

    #[test]
    fn first_run_creates_default_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(!path.exists());
        let s = load_from(&path);
        assert_eq!(s.hotkey.key, "CapsLock");
        assert!(path.exists(), "首次运行应落盘默认配置");
    }

    #[test]
    fn missing_fields_merged_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 老版本配置只写了部分字段
        std::fs::write(&path, r#"{ "hotkey": { "key": "F9" }, "language": "en" }"#).unwrap();
        let s = load_from(&path);
        assert_eq!(s.hotkey.key, "F9");
        assert_eq!(s.hotkey.mode, HotkeyMode::Toggle, "缺失字段用默认值合并");
        assert_eq!(s.language, "en");
        assert_eq!(s.stt_engine, "sherpa-streaming");
        assert!(!s.eval_recording);
        assert!(!s.lower_speaker_volume_while_listening);
        assert_eq!(
            s.download.source,
            DownloadSource::Auto,
            "老配置缺 download 段合并默认"
        );
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
        assert!(!s.ui.first_run_completed, "老配置缺 ui 段合并默认 = 未完成");
        assert_eq!(s.overlay.position, OverlayPosition::Auto);
        assert!(!s.overlay.draggable, "老配置升级后默认关闭拖动");
        assert!(s.overlay.click_through);
        assert_eq!(s.vad.threshold, 0.5, "老配置缺 vad 段合并默认");
        assert_eq!(s.vad.min_speech_ms, 50);
        assert_eq!(s.vad.min_silence_ms, 50);
        assert_eq!(s.hotwords_score, 3.5, "老配置缺 hotwordsScore 合并默认");
        assert!(
            s.sound_feedback.record.enabled && s.sound_feedback.send.enabled,
            "老配置缺 soundFeedback 段合并默认 = 两者开启"
        );
        assert_eq!(s.sound_feedback.record.volume, 60);
        assert_eq!(s.sound_feedback.send.volume, 60);
        assert_eq!(s.sound_feedback.record.sound_id, "rise", "录制音默认上扬");
        assert_eq!(s.sound_feedback.send.sound_id, "fall", "发送音默认下沉");
    }

    #[test]
    fn speaker_volume_setting_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let settings = Settings {
            lower_speaker_volume_while_listening: true,
            ..Settings::default()
        };
        save_to(&path, &settings).unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["lowerSpeakerVolumeWhileListening"], true);
        assert!(load_from(&path).lower_speaker_volume_while_listening);
    }

    #[test]
    fn sound_feedback_partial_patch_preserves_siblings() {
        // 音效卡片只 patch 单项的某个键：其余键与另一项全部保留
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{ "soundFeedback": { "record": { "enabled": false, "soundId": "chirp-up" }, "send": { "volume": 40 } } }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert!(!s.sound_feedback.record.enabled);
        assert_eq!(s.sound_feedback.record.sound_id, "chirp-up");
        assert_eq!(s.sound_feedback.record.volume, 60);
        assert_eq!(s.sound_feedback.send.volume, 40);
        assert_eq!(s.sound_feedback.send.sound_id, "fall");
        assert!(s.sound_feedback.send.enabled);

        let mut merged = serde_json::to_value(&s).unwrap();
        merge_json(
            &mut merged,
            &serde_json::json!({ "soundFeedback": { "record": { "volume": 25 } } }),
        );
        let patched: Settings = serde_json::from_value(merged).unwrap();
        assert!(
            !patched.sound_feedback.record.enabled,
            "未 patch 的 enabled 保留"
        );
        assert_eq!(patched.sound_feedback.record.sound_id, "chirp-up");
        assert_eq!(patched.sound_feedback.record.volume, 25);
        assert_eq!(patched.sound_feedback.send.volume, 40, "另一项不受影响");
    }

    #[test]
    fn vad_partial_patch_preserves_siblings() {
        // 高级页滑块只 patch vad 段里的单个键：merge_json 嵌套合并应保留兄弟键
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 手写一段含 vad 段的 JSON，模拟用户已改过阈值
        std::fs::write(
            &path,
            r#"{ "vad": { "threshold": 0.7 }, "hotwordsScore": 6 }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.vad.threshold, 0.7);
        assert_eq!(s.vad.min_speech_ms, 50, "兄弟键保留默认");
        assert_eq!(s.vad.min_silence_ms, 50);
        assert_eq!(s.hotwords_score, 6.0);

        // 模拟前端 patch 只带一个子键：vad.threshold 覆盖，其余不动
        let mut merged = serde_json::to_value(&s).unwrap();
        merge_json(
            &mut merged,
            &serde_json::json!({ "vad": { "minSpeechMs": 120 } }),
        );
        let patched: Settings = serde_json::from_value(merged).unwrap();
        assert_eq!(patched.vad.threshold, 0.7, "未 patch 的键保留");
        assert_eq!(patched.vad.min_speech_ms, 120);
        assert_eq!(patched.vad.min_silence_ms, 50);
    }

    #[test]
    fn legacy_overlay_interaction_fields_are_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        std::fs::write(
            &path,
            r#"{ "overlay": { "draggable": false, "clickThrough": false } }"#,
        )
        .unwrap();
        let locked = load_from(&path);
        assert!(!locked.overlay.draggable);
        assert!(locked.overlay.click_through);

        std::fs::write(
            &path,
            r#"{ "overlay": { "draggable": true, "clickThrough": true } }"#,
        )
        .unwrap();
        let click_through = load_from(&path);
        assert!(!click_through.overlay.draggable);
        assert!(click_through.overlay.click_through);
    }

    #[test]
    fn download_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.download.source = DownloadSource::Mirror;
        s.download.gh_proxy = "https://gh-proxy.example.com/".into();
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.download.source, DownloadSource::Mirror);
        assert_eq!(loaded.download.gh_proxy, "https://gh-proxy.example.com/");
        // 序列化键名 camelCase：ghProxy / source 小写枚举
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"ghProxy\""));
        assert!(raw.contains("\"mirror\""));
    }

    #[test]
    fn download_source_rejects_unknown_value() {
        // 非法枚举值整体回退默认（serde 反序列化失败 → unwrap_or_default）
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{ "download": { "source": "fastest", "ghProxy": "https://x/" } }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.download.source, DownloadSource::Auto);
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
    }

    #[test]
    fn ui_first_run_completed_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.ui.first_run_completed = true;
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert!(loaded.ui.first_run_completed);
    }

    #[test]
    fn start_page_roundtrips_tolerates_unknown_values_and_is_empty_for_old_configs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 新装：空 = 聊天
        let fresh = load_from(&path);
        assert_eq!(fresh.ui.start_page, "");
        assert!(!fresh.ui.starts_on_voice_input());

        let mut s = Settings::default();
        s.ui.start_page = START_PAGE_VOICE_INPUT.into();
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.ui.start_page, "voiceInput");
        assert!(loaded.ui.starts_on_voice_input());
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk["ui"]["startPage"], "voiceInput");

        // 以后加的取值：照样读进来（不当成坏配置），按聊天处理
        std::fs::write(
            &path,
            br#"{"agent":{},"toolService":{},"ui":{"firstRunCompleted":true,"startPage":"somethingNew"}}"#,
        )
        .unwrap();
        let future = load_from(&path);
        assert_eq!(future.ui.start_page, "somethingNew");
        assert!(future.ui.first_run_completed);
        assert!(!future.ui.starts_on_voice_input());

        // 0.1.x 配置里没有这个字段
        std::fs::write(
            &path,
            br#"{"hotkey":{"key":"F8","mode":"hold"},"ui":{"firstRunCompleted":true,"autoStart":true}}"#,
        )
        .unwrap();
        let old = load_from(&path);
        assert_eq!(old.ui.start_page, "");
        assert_eq!(old.ui.legacy_upgrade, LegacyUpgrade::Pending);
    }

    #[test]
    fn auto_start_and_models_dir_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.ui.auto_start = true;
        s.models.dir = "D:\\kotone-models".into();
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert!(loaded.ui.auto_start);
        assert_eq!(loaded.models.dir, "D:\\kotone-models");
    }

    #[test]
    fn corrupt_file_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "not json {{{").unwrap();
        let loaded = load_from_with_diagnostic(&path);
        assert_eq!(loaded.settings.hotkey.key, "CapsLock");
        assert!(loaded.warning.is_some());
        let backup = path.with_extension("json.corrupt");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "not json {{{");
        assert!(path.exists(), "恢复后应写入一份新的默认配置");
    }

    #[test]
    fn corrupt_backup_never_overwrites_an_existing_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(path.with_extension("json.corrupt"), "older broken config").unwrap();
        std::fs::write(&path, "new broken config").unwrap();

        let loaded = load_from_with_diagnostic(&path);
        assert!(loaded.warning.is_some());
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt")).unwrap(),
            "older broken config"
        );
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt.1")).unwrap(),
            "new broken config"
        );
    }

    #[test]
    fn unreadable_config_is_preserved_before_default_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("original"), "keep me").unwrap();

        let loaded = load_from_with_diagnostic(&path);

        assert!(loaded.warning.is_some());
        assert!(path.is_file(), "恢复后的 config.json 应为默认配置文件");
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt").join("original")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn repository_does_not_publish_or_persist_failed_external_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = Settings::default();
        save_to(&path, &old).unwrap();
        let shared = Arc::new(RwLock::new(old.clone()));
        let repository = SettingsRepository::new_at(shared.clone(), path.clone());

        let error = repository
            .update_with(
                |next| {
                    next.hotkey.key = "Alt+V".into();
                    Ok(())
                },
                |_, _| Err::<(), _>("模拟热键注册冲突".into()),
                |_, _| Ok(()),
            )
            .unwrap_err();

        assert!(error.contains("模拟热键注册冲突"));
        assert_eq!(shared.read().unwrap().hotkey.key, "CapsLock");
        assert_eq!(load_from(&path).hotkey.key, "CapsLock");
    }

    #[test]
    fn repository_merges_each_serial_update_from_latest_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = Settings::default();
        save_to(&path, &old).unwrap();
        let shared = Arc::new(RwLock::new(old));
        let repository = SettingsRepository::new_at(shared.clone(), path.clone());

        repository
            .update(|next| {
                next.language = "en".into();
                Ok(())
            })
            .unwrap();
        repository
            .update(|next| {
                next.audio_device_id = "mic-2".into();
                Ok(())
            })
            .unwrap();

        let memory = shared.read().unwrap().clone();
        let disk = load_from(&path);
        assert_eq!(memory.language, "en");
        assert_eq!(memory.audio_device_id, "mic-2");
        assert_eq!(disk.language, memory.language);
        assert_eq!(disk.audio_device_id, memory.audio_device_id);
    }

    #[test]
    fn repository_does_not_publish_when_config_save_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap(); // rename(tmp, directory) 必然失败
        let shared = Arc::new(RwLock::new(Settings::default()));
        let repository = SettingsRepository::new_at(shared.clone(), path);

        let result = repository.update(|next| {
            next.language = "en".into();
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(shared.read().unwrap().language, "zh");
    }

    #[test]
    fn repository_serializes_concurrent_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let initial = Settings::default();
        save_to(&path, &initial).unwrap();
        let shared = Arc::new(RwLock::new(initial));
        let repository = Arc::new(SettingsRepository::new_at(shared.clone(), path.clone()));
        let mut workers = Vec::new();
        for _ in 0..16 {
            let repository = repository.clone();
            workers.push(std::thread::spawn(move || {
                repository.update(|next| {
                    next.history.max_records += 1;
                    Ok(())
                })
            }));
        }
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        assert_eq!(shared.read().unwrap().history.max_records, 1016);
        assert_eq!(load_from(&path).history.max_records, 1016);
    }

    /// 语音输入搬进插件（ADR 038 第 5 步）：旧配置的顶层字段读得进来，保存时写进
    /// `plugins.voice-input.settings`，再读回来一样；插件自己的开关和别的设置不动。
    #[test]
    fn voice_input_settings_are_read_old_and_written_new() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "resendLastHotkey": "Alt+F6",
                "history": {"mode": "off"},
                "overlay": {"visibility": "never"},
                "postProcessing": {"enabled": false, "activePipelineId": "blocklist", "pipelines": [
                    {"id": "blocklist", "displayName": "我的屏蔽词", "steps": []}
                ]},
                "plugins": {"voice-input": {"enabled": true, "settings": {"note": "插件自己的"}}}
            })
            .to_string(),
        )
        .unwrap();
        let old = load_from(&path);
        assert_eq!(old.resend_last_hotkey, "Alt+F6");
        assert_eq!(old.history.mode, crate::history::HistoryMode::Off);
        assert_eq!(old.overlay.visibility, OverlayVisibility::Never);
        assert!(!old.post_processing.enabled);
        assert_eq!(old.post_processing.pipelines[0].display_name, "我的屏蔽词");

        save_to(&path, &old).unwrap();
        let file: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for key in VOICE_INPUT_SETTINGS {
            assert!(file.get(*key).is_none(), "{key} 不再写在顶层");
            assert!(
                file["plugins"]["voice-input"]["settings"]
                    .get(*key)
                    .is_some(),
                "{key} 写进插件"
            );
        }
        assert_eq!(file["plugins"]["voice-input"]["enabled"], true);
        assert_eq!(
            file["plugins"]["voice-input"]["settings"]["note"],
            "插件自己的"
        );

        let again = load_from(&path);
        assert_eq!(again.resend_last_hotkey, "Alt+F6");
        assert_eq!(again.overlay.visibility, OverlayVisibility::Never);
        assert_eq!(
            again.post_processing.pipelines[0].display_name,
            "我的屏蔽词"
        );
        assert!(again.plugins["voice-input"]
            .settings
            .get("history")
            .is_none());
    }

    /// 新旧两处都有（装回过旧版本又升级）：新位置为准。
    #[test]
    fn voice_input_settings_in_the_new_place_win() {
        let mut user = serde_json::json!({
            "resendLastHotkey": "Alt+F1",
            "plugins": {"voice-input": {"settings": {"resendLastHotkey": "Alt+F6"}}}
        });
        lift_voice_input_json(&mut user);
        assert_eq!(user["resendLastHotkey"], "Alt+F6");
        assert!(user["plugins"]["voice-input"]["settings"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn save_to_serializes_the_shared_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("config.json"));
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let mut workers = Vec::new();
        for index in 0..16 {
            let path = path.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let settings = Settings {
                    language: format!("lang-{index}"),
                    ..Settings::default()
                };
                barrier.wait();
                save_to(&path, &settings)
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        let loaded = load_from(&path);
        assert!(loaded.language.starts_with("lang-"));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn legacy_config_without_connections_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let settings = load_from(&path);
        assert!(settings.connections.is_empty());
    }

    #[test]
    fn settings_connection_resolver_reads_public_fields_only() {
        use crate::connection::{Connection, ConnectionKind, ConnectionResolver};

        let mut settings = Settings::default();
        settings.connections.push(Connection {
            id: "dashscope-main".into(),
            display_name: "通义".into(),
            kind: ConnectionKind::Remote,
            provider: "dashscope".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            model: "qwen-turbo".into(),
        });
        let resolver = SettingsConnectionResolver::new(Arc::new(RwLock::new(settings)));
        let resolved = resolver.resolve("dashscope-main").unwrap();
        assert_eq!(resolved.model, "qwen-turbo");
        assert!(resolved.api_key.is_none());
        assert!(resolver
            .resolve("missing")
            .unwrap_err()
            .contains("未找到连接"));
    }
}
