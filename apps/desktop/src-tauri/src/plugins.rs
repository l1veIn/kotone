//! 插件的宿主（ADR 038）：插件注册、开关、交给插件的本体能力（[`DesktopHost`]），以及插件工具的三个出口——
//! 她的工具清单（[`offers`]）、工具服务 / MCP（[`service_specs`]）、插件自己的页面（`plugins_cmd.rs`）。
//!
//! - 一期都是内置插件：[`registry`] 是一张静态表，编译在一起；
//! - 开关在设置 `plugins.<id>`（`enabled`、`sources.<开关>`），没写按插件清单的默认；
//! - 插件关着时：不出现在她的工具清单、工具服务里，调用被拒；后台循环由插件自己看 [`Host::enabled`] 停下；
//! - 工具只在插件里定义一次（名字、说明、参数、`visibility`、页面、要的数据开关）。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use kotone_core::games::Observation;
use kotone_plugin_api::{BoxFuture, Host, Manifest, Offer, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager};

/// 界面上插件的事件（`{plugin, event, payload}`）。
pub const EVENT: &str = "kotone://plugin";
/// 打开一个页面（`ui.open`）：`{uri, params, title, plugin?}`。插件的页面在右侧栏，设置页在设置窗口。
pub const UI_OPEN_EVENT: &str = "kotone://ui-open";
/// 本体的设置页（`ui://kotone/settings/<分页>`，和界面 `advanced/types.ts` 的分页一致）。
/// `call` / `reply` / `permissions` / `usage` 现在是角色页的 唤醒 / 交互 / 权限 / 用量（交接第十一节第 8 条，2026-11-25）：
/// 地址不变，界面收到后打开角色页的那一页（`navigation.ts` 的 `normalizeTarget`）。
pub const SETTINGS_SECTIONS: &[&str] = &[
    "call",
    "reply",
    "permissions",
    "usage",
    "models",
    "connections",
    "sound",
    "tuning",
    "appearance",
    "system",
    "browser",
    // 旧地址：插件搬到了左侧栏的「插件」（ADR 038，2026-11-24），界面上打开那一页
    "plugins",
];

/// 语音输入插件开着没有（ADR 038）：关着时语音输入热键、悬浮窗都没有。她替你在游戏聊天里发言
/// （`game_chat.send`）是本体的，不看它（ADR 039 D2）。
pub fn voice_input_on(settings: &kotone_core::settings::Settings) -> bool {
    enabled(settings, kotone_plugin_voice_input::plugin::manifest())
}

/// 内置的插件（按侧栏里的顺序）。
pub fn registry() -> &'static [Arc<dyn Plugin>] {
    static PLUGINS: OnceLock<Vec<Arc<dyn Plugin>>> = OnceLock::new();
    PLUGINS.get_or_init(|| {
        vec![
            Arc::new(kotone_plugin_lol::LolPlugin::new()) as Arc<dyn Plugin>,
            Arc::new(kotone_plugin_game_voice::GameVoicePlugin::new()),
            Arc::new(kotone_plugin_music::MusicPlugin::new()),
            Arc::new(kotone_plugin_interpreter::InterpreterPlugin),
            Arc::new(kotone_plugin_voice_input::VoiceInputPlugin::new()),
        ]
    })
}

#[cfg(test)]
fn find(id: &str) -> Option<&'static Arc<dyn Plugin>> {
    registry().iter().find(|p| p.manifest().id == id)
}

/// 这个工具是哪个内置插件的（不管开没开）。工具服务只认这些（第三方的不转给外部 Agent）。
pub fn owner(tool: &str) -> Option<&'static Arc<dyn Plugin>> {
    registry()
        .iter()
        .find(|p| p.tools().iter().any(|t| t.name == tool))
}

/// 这个工具是哪个插件的：内置的和第三方的（MCP 方案第 2 期，`remote_plugins.rs`）都看。
fn any_owner(tool: &str) -> Option<Arc<dyn Plugin>> {
    owner(tool).cloned().or_else(|| {
        crate::remote_plugins::as_plugins()
            .into_iter()
            .find(|p| p.tools().iter().any(|t| t.name == tool))
    })
}

/// 第三方插件的工具只在 Kotone 里用（她、它自己的页面），不转给外部 Agent（MCP 方案第七节第 2 条，隐私）。
const NOT_SHARED: &str =
    "这是第三方插件的工具，只在 Kotone 里用，不转给外部 Agent（工具服务、kotone.exe mcp）";

/// 谁在调、是不是第三方插件 → 不让调的原因。
fn third_party_refusal(remote: bool, caller: Caller<'_>) -> Option<&'static str> {
    (remote && matches!(caller, Caller::Service)).then_some(NOT_SHARED)
}

/// 交给这个插件的宿主：第三方插件拿更窄的那一份（`remote_plugins::RemoteHost`）。**先看内置表**：内置插件
/// 永远拿 `DesktopHost`（`mcp.json` 里写了同名的一项也一样，那一项不进插件表，见 `remote_plugins`）。
fn host_for(app: &AppHandle, plugin: &Arc<dyn Plugin>, caller: Caller<'_>) -> Arc<dyn Host> {
    let id = &plugin.manifest().id;
    if registry().iter().any(|p| Arc::ptr_eq(p, plugin)) || !crate::remote_plugins::is_remote(id) {
        DesktopHost::for_call(app, plugin, caller)
    } else {
        crate::remote_plugins::host(app, id)
    }
}

fn settings(app: &AppHandle) -> kotone_core::settings::Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 插件开着没有（规则在契约里，事件目录也用它：`Manifest::enabled_in`）。
pub fn enabled(settings: &kotone_core::settings::Settings, manifest: &Manifest) -> bool {
    manifest.enabled_in(settings)
}

/// 插件里的某个数据开关开着没有（插件关着也算关；`Manifest::source_enabled_in`）。
pub fn source_enabled(
    settings: &kotone_core::settings::Settings,
    manifest: &Manifest,
    source: &str,
) -> bool {
    manifest.source_enabled_in(settings, source)
}

/// 插件开着、工具要的数据开关都开着。
fn tool_available(
    settings: &kotone_core::settings::Settings,
    manifest: &Manifest,
    tool: &ToolSpec,
) -> bool {
    enabled(settings, manifest)
        && tool
            .needs
            .iter()
            .all(|s| source_enabled(settings, manifest, s))
}

/// 在这个线程上临时用“低于正常”的优先级，离开作用域时改回来。
struct BelowNormalThread(#[cfg(windows)] i32);

impl BelowNormalThread {
    fn enter() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                GetCurrentThread, GetThreadPriority, SetThreadPriority,
                THREAD_PRIORITY_BELOW_NORMAL,
            };
            // SAFETY: 只改当前线程的优先级。
            unsafe {
                let old = GetThreadPriority(GetCurrentThread());
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                Self(old)
            }
        }
        #[cfg(not(windows))]
        Self()
    }
}

impl Drop for BelowNormalThread {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: 改回进来时的优先级。
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
            };
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(self.0));
        }
    }
}

/// 宿主交给插件的本体能力。每个插件一份。
pub struct DesktopHost {
    app: AppHandle,
    id: String,
    manifest: Manifest,
    /// 这次是谁调了插件的工具（她、外部 Agent、玩家的热键）：插件经入口发到游戏时照谁的规矩、记成谁发的
    /// （[`Host::send_to_game`]）。插件自己的后台、页面调的、入口回头调插件的发法时是 None：不能发。
    speaker: Option<kotone_core::send_to_game::Speaker>,
}

impl DesktopHost {
    /// 凭据库里的名字：`<插件 id>:<name>`（放歌是 `music:netease`，和搬进插件以前一样）。
    fn secret_id(&self, name: &str) -> String {
        format!("{}:{name}", self.id)
    }

    /// 插件的网页登录窗口（放歌是 `music-login`，和搬进插件以前一样）。
    fn web_window_label(&self) -> String {
        format!("{}-login", self.id)
    }

    pub fn for_plugin(app: &AppHandle, plugin: &Arc<dyn Plugin>) -> Arc<dyn Host> {
        Arc::new(Self {
            app: app.clone(),
            id: plugin.manifest().id.clone(),
            manifest: plugin.manifest().clone(),
            speaker: None,
        })
    }

    /// 调插件工具时的宿主：记下是谁调的（[`speaker_of`]）。
    fn for_call(app: &AppHandle, plugin: &Arc<dyn Plugin>, caller: Caller<'_>) -> Arc<dyn Host> {
        Arc::new(Self {
            app: app.clone(),
            id: plugin.manifest().id.clone(),
            manifest: plugin.manifest().clone(),
            speaker: speaker_of(caller),
        })
    }
}

/// 现在在玩的游戏（[`Host::playing_game`]）：在跑的游戏里窗口在前台的那个；没有在前台的、只有一个在跑时就是它。
fn playing_of(states: &[kotone_core::games::GameState]) -> Option<kotone_plugin_api::Playing> {
    let state = states
        .iter()
        .find(|s| s.focused)
        .or_else(|| (states.len() == 1).then(|| &states[0]))?;
    Some(kotone_plugin_api::Playing {
        game: state.game.clone(),
        match_id: state.match_id.clone(),
    })
}

/// 谁调了插件的工具 → 插件经入口发话时算谁在发（入口按它判前台规矩、记占用者）。页面调的不算：玩家点页面时
/// Kotone 在前台，插件不该借页面往游戏里发话。
fn speaker_of(caller: Caller<'_>) -> Option<kotone_core::send_to_game::Speaker> {
    use kotone_core::send_to_game::Speaker;
    match caller {
        Caller::Her => Some(Speaker::Companion),
        Caller::Service => Some(Speaker::Agent),
        // 玩家自己加的、插件预设的热键：照“替玩家发言”的规矩（前台要是游戏），和热键调 game_chat.send 一样
        Caller::Hotkey => Some(Speaker::Automation),
        Caller::Page(_) => None,
    }
}

impl Host for DesktopHost {
    fn plugin(&self) -> &str {
        &self.id
    }

    fn enabled(&self) -> bool {
        enabled(&settings(&self.app), &self.manifest)
    }

    fn source_enabled(&self, source: &str) -> bool {
        source_enabled(&settings(&self.app), &self.manifest, source)
    }

    fn settings(&self) -> Value {
        settings(&self.app)
            .plugins
            .get(&self.id)
            .map(|p| Value::Object(p.settings.clone()))
            .unwrap_or_else(|| json!({}))
    }

    fn data_dir(&self) -> PathBuf {
        kotone_core::settings::kotone_dir()
            .join("plugins")
            .join(&self.id)
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tauri::async_runtime::spawn(task);
    }

    fn process_ids(&self, exe: &str) -> Vec<u32> {
        kotone_platform_windows::app_audio::pids_by_name(exe)
    }

    fn process_command_line(&self, pid: u32) -> Option<String> {
        kotone_platform_windows::process_cmdline::command_line(pid)
    }

    fn game_observe(&self, game: &str, observation: Observation) {
        crate::games::observe(&self.app, game, observation);
    }

    fn game_claim(&self, game: &str, claimed: bool) {
        crate::games::claim(game, claimed);
    }

    fn game_event(
        &self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        game_time: Option<f64>,
    ) {
        // 只收清单 `[[events]]` 里登记过的（事件目录只有一种登记方式；插件的测试查它报的都登记了）
        if !self.manifest.declares_event(kind) {
            kotone_core::log::log(&format!(
                "插件 {} 报了没登记的游戏事件 {kind}，没收",
                self.id
            ));
            return;
        }
        crate::games::push(&self.app, game, kind, source, data, game_time);
    }

    fn playing_game(&self) -> Option<kotone_plugin_api::Playing> {
        playing_of(&crate::games::states())
    }

    fn app_version(&self) -> String {
        self.app.package_info().version.to_string()
    }

    fn screen_text(
        &self,
        process: &str,
        region: [f64; 4],
        local_only: bool,
    ) -> BoxFuture<'static, Result<Vec<kotone_plugin_api::ScreenText>, String>> {
        let app = self.app.clone();
        let process = process.to_string();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                // 照 aramhex：识别线程用低于正常的优先级（不用最低：低配机器上会饿死）
                let _priority = BelowNormalThread::enter();
                crate::computer_use::screen_text(&app, &process, region, local_only)
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    /// 本地文字识别能不能用（ADR 043 D7：海斗三选一固定用本地识别，`ocr_engines::local_status`）。
    fn local_ocr(&self) -> kotone_plugin_api::LocalOcr {
        crate::ocr_engines::local_status(&self.app)
    }

    /// 下载推荐的本地文字识别模型：和「设置 → 模型 → 文字识别」的下载同一套（`model_catalog::download`：同一时间只下
    /// 一个、逐文件校验、进度照常发 `kotone://download`），**不改**全局的文字识别选择。已经有能用的就什么都不做。
    fn download_local_ocr(&self) -> BoxFuture<'static, Result<(), String>> {
        let app = self.app.clone();
        let plugin = self.id.clone();
        Box::pin(async move {
            let model = match crate::ocr_engines::local_status(&app) {
                kotone_plugin_api::LocalOcr::Ready { .. } => return Ok(()),
                kotone_plugin_api::LocalOcr::Missing { model, .. } => model,
                kotone_plugin_api::LocalOcr::Failed { reason } => return Err(reason),
            };
            kotone_core::log::log(&format!(
                "插件 {plugin} 要下载本地文字识别模型 {model}（玩家点了下载）"
            ));
            let result = crate::model_catalog::download(
                &app,
                kotone_core::model_catalog::ModelCategory::Ocr,
                &model,
            )
            .await;
            kotone_core::log::log(&match &result {
                Ok(()) => format!("本地文字识别模型 {model} 下好了"),
                Err(e) => format!("本地文字识别模型 {model} 没下成：{e}"),
            });
            result
        })
    }

    fn emit(&self, event: &str, payload: Value) {
        let _ = self.app.emit(
            EVENT,
            json!({"plugin": self.id, "event": event, "payload": payload}),
        );
    }

    /// 发到游戏（ADR 041 第 3 期）：只给内置插件、插件开着、有人在调它的工具（[`DesktopHost::speaker`]）时；
    /// 交给和 `game_chat.send` 同一个入口（[`crate::game_chat::send_to_game_chat`]：前台、频道、锁、修饰键、执行记录
    /// 都在那里），“停下”能停掉还没发完的几行。
    fn send_to_game(
        &self,
        text: &str,
        channel: Option<&str>,
    ) -> BoxFuture<'static, Result<Value, String>> {
        let refusal = if !registry().iter().any(|p| p.manifest().id == self.id) {
            Some("发到游戏只给内置插件".to_string())
        } else if !enabled(&settings(&self.app), &self.manifest) {
            Some(format!(
                "「{}」插件关着（在左侧栏的「插件」里可以打开）",
                self.manifest.name
            ))
        } else if self.speaker.is_none() {
            Some("插件只能在她、外部 Agent 或热键调它的工具时发到游戏".to_string())
        } else {
            None
        };
        let app = self.app.clone();
        let session = format!("plugin:{}", self.id);
        let arguments = json!({"text": text, "channel": channel, "send": true});
        let speaker = self.speaker;
        Box::pin(async move {
            if let Some(refusal) = refusal {
                return Err(refusal);
            }
            let speaker = speaker.ok_or("没有发话的人")?;
            let request = kotone_agent::body::chat_request(&arguments, true, speaker)?;
            crate::game_chat::send_to_game_chat(&app, &session, request).await
        })
    }

    /// 推送（ADR 043 D3–D5）：只给内置插件（这张静态表里的，和 Kotone 一起编译）。开关由宿主执行：清单 `[[pushes]]` 里
    /// 没有这条、玩家在插件页关了、插件或它要的数据开关关着，都直接丢掉（`Off`）；只留这条声明过的样子、截字，再交给
    /// `agent::post_push`（新的、更新、一样）。
    fn push(
        &self,
        push: kotone_plugin_api::Push,
    ) -> Result<kotone_plugin_api::PushOutcome, String> {
        if !registry().iter().any(|p| p.manifest().id == self.id) {
            return Err("推送只给内置插件".into());
        }
        let Some(spec) = self.manifest.push_spec(&push.trigger) else {
            kotone_core::log::log(&format!(
                "插件 {} 推了清单里没有的推送 {}，没收",
                self.id, push.trigger
            ));
            return Ok(kotone_plugin_api::PushOutcome::Off);
        };
        if let Some(off) = self.manifest.push_off_in(&settings(&self.app), &spec.id) {
            kotone_core::log::log(&format!("push: {} {} 没推（{off}）", self.id, push.key));
            return Ok(kotone_plugin_api::PushOutcome::Off);
        }
        let push = push.shaped(spec);
        let key = push.key.clone();
        let posted = push.check(&self.manifest).and_then(|()| {
            crate::agent::post_push(&self.app, &self.id, &self.manifest.name, spec, push)
        });
        // 没推出去的宿主记一笔（插件不用自己记日志：它们只依赖插件契约）
        if let Err(e) = &posted {
            kotone_core::log::log(&format!("push: {} {key} 没推出去：{e}", self.id));
        }
        posted
    }

    /// 推送结束了（例如三选一的卡片从屏幕上消失）：悬浮窗收起那一行，聊天里的记录留着。只认清单里有的推送。
    fn push_done(&self, trigger: &str, key: &str) {
        if self.manifest.push_spec(trigger).is_some() {
            crate::agent::push_done(&self.app, &self.id, trigger, key);
        }
    }

    /// 这条推送现在推不推得出去：和 [`DesktopHost::push`] 收推送时同一条规则（内置插件、`push_off_in` 没有原因）。
    fn push_enabled(&self, trigger: &str) -> bool {
        registry().iter().any(|p| p.manifest().id == self.id)
            && self
                .manifest
                .push_off_in(&settings(&self.app), trigger)
                .is_none()
    }

    fn companion_enabled(&self) -> bool {
        !settings(&self.app)
            .ui
            .legacy_upgrade
            .uses_legacy_assistant()
    }

    fn update_settings(&self, patch: Value) -> Result<(), String> {
        // 和玩家在界面上保存一样走一遍（通知各窗口、插件的 settings_changed）
        let id = self.id.as_str();
        crate::settings_cmd::update_settings_from_backend(
            &self.app,
            json!({"plugins": {id: {"settings": patch}}}),
        )
        .map(|_| ())
    }

    fn can_synthesize(&self) -> bool {
        let assistant = settings(&self.app).assistant;
        crate::assistant::effective_engine(&self.app, &assistant)
            != kotone_core::assistant::TtsEngine::System
    }

    fn synthesize(&self, text: &str) -> BoxFuture<'static, Result<(Vec<f32>, u32), String>> {
        let app = self.app.clone();
        let text = text.to_string();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let (speaker, request) = crate::assistant::speaker_and_request(&app, &text)?;
                speaker.synthesize(request)
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn speak(&self, text: &str) -> BoxFuture<'static, Result<(), String>> {
        let app = self.app.clone();
        let text = text.to_string();
        Box::pin(async move { crate::assistant::speak_in_turn(&app, &text).await })
    }

    fn player_says(&self, text: &str) -> Result<(), String> {
        // 和在聊天窗口里打字一样（来源 chat：算玩家亲口说的，回复写在聊天里）
        crate::agent::send(
            &self.app,
            crate::agent::Outgoing {
                text: text.to_string(),
                source: "chat".into(),
                attachments: Vec::new(),
            },
        )
        .map(|_| ())
    }

    fn listening(&self) -> Vec<String> {
        crate::voice_listen::active(&self.app)
            .into_iter()
            .map(|(_, app)| app)
            .collect()
    }

    fn she_is_audible(&self) -> bool {
        crate::assistant::she_is_audible(&self.app)
    }

    fn secret(&self, name: &str) -> Option<String> {
        use kotone_core::connection::SecretStore;
        kotone_ai::secrets::KeyringSecretStore
            .get(&self.secret_id(name))
            .ok()
            .flatten()
    }

    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        use kotone_core::connection::SecretStore;
        kotone_ai::secrets::KeyringSecretStore
            .set(&self.secret_id(name), value)
            .map_err(|e| e.to_string())
    }

    fn delete_secret(&self, name: &str) {
        use kotone_core::connection::SecretStore;
        let _ = kotone_ai::secrets::KeyringSecretStore.delete(&self.secret_id(name));
    }

    /// 谁都能让插件开它（玩家在插件页上点的，也可能是外部 Agent 经工具服务调了页面的工具）：玩家正看着 Kotone 时
    /// 拿到前面；不然（玩家在游戏或别的程序里）放在后面、闪任务栏，不抢前台（[`crate::attention::present`]）。
    fn open_web_window(&self, url: &str, title: &str) -> Result<(), String> {
        let label = self.web_window_label();
        if let Some(window) = self.app.get_webview_window(&label) {
            crate::attention::present(&window);
            return Ok(());
        }
        let url = url.parse().map_err(|e| format!("网址不对：{e}"))?;
        // 建的时候不显示、不要焦点（WebView2 建好会把键盘焦点移进去，窗口就被激活了），建好再按能不能拿前台显示
        // 无痕（WebView2 InPrivate，和主窗口同一个浏览器进程）：网页数据关窗口就没了，登录凭据只在插件那边。
        // 用单独的数据目录会另起一个浏览器进程，真机测试的调试端口会被它占走。
        let window =
            tauri::WebviewWindowBuilder::new(&self.app, &label, tauri::WebviewUrl::External(url))
                .title(title)
                .inner_size(1000.0, 720.0)
                .center()
                .incognito(true)
                .visible(false)
                .focused(false)
                .build()
                .map_err(|e| format!("打不开网页窗口：{e}"))?;
        let front = crate::attention::present(&window);
        kotone_core::log::log(&format!(
            "插件 {} 的网页窗口建好了（无痕，{}）",
            self.id,
            if front {
                "拿到了前面"
            } else {
                "玩家不在 Kotone 里：放在后面、闪任务栏"
            }
        ));
        Ok(())
    }

    fn web_window_open(&self) -> bool {
        self.app
            .get_webview_window(&self.web_window_label())
            .is_some()
    }

    fn web_window_cookies(
        &self,
        url: &str,
    ) -> BoxFuture<'static, Result<Vec<(String, String)>, String>> {
        let window = self.app.get_webview_window(&self.web_window_label());
        let url = url.parse::<url::Url>();
        Box::pin(async move {
            let window = window.ok_or("网页窗口没了")?;
            let url = url.map_err(|e| format!("{e}"))?;
            // 读 Cookie 在 Windows 上不能在主线程 / 同步回调里做（会卡死）；要主线程配合，万一卡住 5 秒后不等了
            let read = tauri::async_runtime::spawn_blocking(move || window.cookies_for_url(url));
            let cookies = tokio::time::timeout(std::time::Duration::from_secs(5), read)
                .await
                .map_err(|_| "读 Cookie 超过 5 秒没回来".to_string())?
                .map_err(|e| format!("读 Cookie 的线程出错：{e}"))?
                .map_err(|e| format!("读 Cookie 失败：{e}"))?;
            Ok(cookies
                .iter()
                .map(|c| (c.name().to_string(), c.value().to_string()))
                .collect())
        })
    }

    fn close_web_window(&self) {
        if let Some(window) = self.app.get_webview_window(&self.web_window_label()) {
            let _ = window.destroy();
        }
    }

    fn download(
        &self,
        url: &str,
        dest: PathBuf,
        sha256: Option<&str>,
    ) -> BoxFuture<'static, Result<(), String>> {
        let url = url.to_string();
        let sha256 = sha256.map(str::to_string);
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let _guard = kotone_stt::download::begin_download()?;
                kotone_stt::download::download_file(&url, &dest, sha256.as_deref(), &|_, _| {})
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }
}

/// 设置保存了（运行时状态推送点，设置保存的最后一步也经过这里）：告诉每个插件，跟着设置启停的东西自己对一下。
pub fn settings_changed(app: &AppHandle) {
    for plugin in registry() {
        plugin.settings_changed(DesktopHost::for_plugin(app, plugin));
    }
    crate::remote_plugins::settings_changed(app);
}

/// 玩家让她停下：每个插件停掉正在替她做的事（例如游戏语音里还没说完的话）。
pub fn stop_all() {
    for plugin in registry() {
        plugin.stop();
    }
}

/// 启动时：每个插件开它的后台循环（循环里自己看开关）。
pub fn init(app: &AppHandle) {
    for plugin in registry() {
        if let Err(error) = plugin.manifest().check_tools(&plugin.tools()) {
            kotone_core::log::log(&format!(
                "插件 {} 的工具不对：{error}",
                plugin.manifest().id
            ));
        }
        plugin.clone().start(DesktopHost::for_plugin(app, plugin));
    }
    // 第三方插件（MCP 方案第 2 期）：读 KOTONE_HOME/mcp.json 登记，开着、还不知道有哪些工具的去连
    crate::remote_plugins::init(app);
}

/// 开着的插件给她的东西（她的工具清单和系统提示）。一段对话开始时取一次，之后不变（ADR 024）。
pub fn offers(app: &AppHandle) -> Vec<Offer> {
    offers_for(&settings(app))
}

pub(crate) fn offers_for(settings: &kotone_core::settings::Settings) -> Vec<Offer> {
    let mut offers = builtin_offers(settings);
    // 第三方插件：工具多了只给后台（`remote_plugins::offers`）
    offers.extend(crate::remote_plugins::offers(
        settings,
        &crate::remote_plugins::as_plugins(),
    ));
    offers
}

fn builtin_offers(settings: &kotone_core::settings::Settings) -> Vec<Offer> {
    registry()
        .iter()
        .filter(|p| enabled(settings, p.manifest()))
        .map(|p| {
            let m = p.manifest();
            Offer {
                plugin: m.id.clone(),
                name: m.name.clone(),
                text: m.agent.clone(),
                tools: p
                    .tools()
                    .into_iter()
                    .filter(|t| t.for_model() && tool_available(settings, m, t))
                    .collect(),
            }
        })
        .collect()
}

/// 工具服务 `system.capabilities` 里开着的插件的工具（给外部 Agent、MCP；带 `visibility`）。
pub fn service_specs(settings: &kotone_core::settings::Settings) -> Vec<Value> {
    registry()
        .iter()
        .flat_map(|p| {
            let m = p.manifest();
            p.tools()
                .into_iter()
                .filter(|t| tool_available(settings, m, t))
                .map(|t| t.service_spec(&m.id))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 谁在调插件的工具。
#[derive(Debug, Clone, Copy)]
pub enum Caller<'a> {
    /// 她（她的工具清单里只有给她的，`visibility` 含 `model`）。
    Her,
    /// 外部 Agent 经工具服务 / MCP（清单里两种都列出来，带 `visibility`，都能调）。
    Service,
    /// 插件自己的页面（只能调自己插件的、给页面的工具）。
    Page(&'a str),
    /// 玩家自己加的热键（ADR 040 第 3 期）：和她一样只调给她的工具（`visibility` 含 `model`）；第三方插件的也能调
    /// （是玩家在 Kotone 里按的，不是转给外部 Agent）。
    Hotkey,
}

/// 调一个插件的工具。
/// 插件关着时拒绝；数据开关关着时由插件自己回答（一段对话里她的工具清单不变，开关可能中途关了）。
pub async fn call(
    app: &AppHandle,
    tool: &str,
    arguments: &Value,
    caller: Caller<'_>,
) -> Result<ToolResult, String> {
    let plugin = any_owner(tool).ok_or_else(|| format!("没有叫 {tool} 的插件工具"))?;
    let manifest = plugin.manifest();
    let spec = plugin
        .tools()
        .into_iter()
        .find(|t| t.name == tool)
        .ok_or_else(|| format!("没有叫 {tool} 的插件工具"))?;
    if let Some(refusal) =
        third_party_refusal(crate::remote_plugins::is_remote(&manifest.id), caller)
    {
        return Err(refusal.to_string());
    }
    match caller {
        Caller::Page(page_plugin) if page_plugin != manifest.id => {
            return Err(format!("{page_plugin} 的页面不能调 {tool}"));
        }
        Caller::Page(_) if !spec.for_app() => return Err(format!("{tool} 不给页面用")),
        Caller::Her | Caller::Hotkey if !spec.for_model() => {
            return Err(format!("{tool} 只给插件的页面用"))
        }
        _ => {}
    }
    if !enabled(&settings(app), manifest) {
        return Ok(ToolResult::content(json!({
            "status": "disabled",
            "text": format!("「{}」插件关着（在左侧栏的「插件」里可以打开）", manifest.name),
        })));
    }
    plugin
        .call(host_for(app, &plugin, caller), tool, arguments)
        .await
}

/// `ui.open`（ADR 038 D5）：检查页面地址，再让界面打开它。不把 Kotone 窗口拉到前面（玩家可能在游戏里）。
pub fn open_page(app: &AppHandle, args: &Value) -> Result<Value, String> {
    let uri = args
        .get("page")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or("要写 page（页面地址，如 ui://kotone/lol/match）")?;
    let params = args.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err("params 要是对象".into());
    }
    let (title, plugin) = resolve_page(&settings(app), uri)?;
    let _ = app.emit(
        UI_OPEN_EVENT,
        json!({"uri": uri, "params": params, "title": title, "plugin": plugin}),
    );
    Ok(json!({"opened": true, "uri": uri, "title": title,
        "note": "在 Kotone 窗口里打开了（插件的页面在右侧栏）；玩家在游戏里时窗口不会自己跳出来"}))
}

/// 自己加的“打开页面”热键要打开的地方（ADR 040 第 3 期）。
#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyPage {
    /// 设置的一页、内置插件的页面：经 `kotone://ui-open` 交给界面（标着是玩家自己要看的）。
    Kotone {
        uri: String,
        title: String,
        plugin: Option<String>,
    },
    /// 第三方插件的页面：打开左侧栏「插件」里这个插件的页面（右侧栏还显示不了第三方的 App）。
    ThirdParty { plugin: String },
}

/// 热键的页面打不打得开：`Err(None)` = 插件关着（热键不登记、也不占键，打开插件就回来）；`Err(Some(原因))` 打不开。
pub fn hotkey_page(
    settings: &kotone_core::settings::Settings,
    uri: &str,
) -> Result<HotkeyPage, Option<String>> {
    let uri = uri.trim();
    if !uri.starts_with("ui://kotone/") {
        let owner = crate::remote_plugins::plugins()
            .into_iter()
            .find(|p| p.server().owns_uri(uri));
        return match owner {
            Some(p) if settings.plugin_enabled(&p.manifest().id, true) => {
                Ok(HotkeyPage::ThirdParty {
                    plugin: p.manifest().id.clone(),
                })
            }
            Some(_) => Err(None),
            None => Err(Some(format!("没有这个页面：{uri}（第三方插件删掉了？）"))),
        };
    }
    if registry()
        .iter()
        .find(|p| p.manifest().owns_uri(uri))
        .is_some_and(|p| !enabled(settings, p.manifest()))
    {
        return Err(None);
    }
    resolve_page(settings, uri)
        .map(|(title, plugin)| HotkeyPage::Kotone {
            uri: uri.to_string(),
            title,
            plugin,
        })
        .map_err(Some)
}

/// 页面地址 → (标题, 插件 id)。设置页的插件 id 是 None。
fn resolve_page(
    settings: &kotone_core::settings::Settings,
    uri: &str,
) -> Result<(String, Option<String>), String> {
    if let Some(section) = uri.strip_prefix("ui://kotone/settings/") {
        return if SETTINGS_SECTIONS.contains(&section) {
            Ok((section.to_string(), None))
        } else {
            Err(format!(
                "没有设置页 {section}（有：{}）",
                SETTINGS_SECTIONS.join("、")
            ))
        };
    }
    // 第三方插件的 MCP App（`ui://` 不限于 `ui://kotone/`）：界面的右侧栏只认 `ui://kotone/<插件>/<页面>`，
    // 这里说“打开了”界面却什么都不开。现在明说打不开、去哪打开（审查后改的，2026-11-25）
    if !uri.starts_with("ui://kotone/") {
        let owner = crate::remote_plugins::plugins()
            .into_iter()
            .find(|p| p.server().owns_uri(uri));
        return Err(match owner {
            Some(p) => format!(
                "第三方插件的页面现在只能在插件页里打开：请玩家在左侧栏的「插件」里点「{}」",
                p.manifest().id
            ),
            None => format!("没有这个页面：{uri}"),
        });
    }
    let plugin = registry()
        .iter()
        .find(|p| p.manifest().owns_uri(uri))
        .ok_or_else(|| format!("没有这个页面：{uri}"))?;
    let m = plugin.manifest();
    let page = m
        .pages
        .iter()
        .find(|p| p.uri == uri)
        .ok_or_else(|| format!("「{}」插件没有页面 {uri}", m.name))?;
    if !enabled(settings, m) {
        return Err(format!(
            "「{}」插件关着（在左侧栏的「插件」里可以打开）",
            m.name
        ));
    }
    if let Some(off) = page.needs.iter().find(|s| !source_enabled(settings, m, s)) {
        let name = m.source(off).map(|s| s.name.as_str()).unwrap_or(off);
        return Err(format!("「{}」插件的“{name}”关着，这个页面用不了", m.name));
    }
    Ok((page.title.clone(), Some(m.id.clone())))
}

/// 界面用的插件清单：清单、开关、工具（名字、给谁、页面）。内置的在前，第三方的在后（多一个 `remote`：
/// 怎么连、要哪些 Key、信任没有、连接状态、服务器说的自己，`remote_plugins::describe`）。
pub fn list(app: &AppHandle) -> Value {
    let settings = settings(app);
    let remote = crate::remote_plugins::plugins();
    let all: Vec<Arc<dyn Plugin>> = registry()
        .iter()
        .cloned()
        .chain(remote.iter().map(|p| p.clone() as Arc<dyn Plugin>))
        .collect();
    Value::Array(
        all.iter()
            .map(|p| {
                let m = p.manifest();
                let sources: Map<String, Value> = m
                    .sources
                    .iter()
                    .map(|s| (s.id.clone(), json!(source_enabled(&settings, m, &s.id))))
                    .collect();
                let mut item = json!({
                    "manifest": m,
                    "enabled": enabled(&settings, m),
                    "sources": sources,
                    "tools": p.tools().iter().map(|t| json!({
                        "name": t.name,
                        "visibility": t.visibility,
                        "page": t.page,
                        "available": tool_available(&settings, m, t),
                    })).collect::<Vec<_>>(),
                });
                if let Some(r) = remote.iter().find(|r| r.manifest().id == m.id) {
                    item["remote"] = crate::remote_plugins::describe(r);
                }
                item
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同传、游戏语音报事件、按局分卡片时问“现在在玩哪个游戏”：前台的那个；都不在前台时只有一个在跑才算。
    #[test]
    fn the_playing_game_is_the_focused_one_or_the_only_one() {
        use kotone_core::games::{GameState, Phase};
        let state = |game: &str, focused: bool, match_id: &str| GameState {
            game: game.into(),
            phase: Phase::InMatch,
            native_phase: String::new(),
            match_id: match_id.into(),
            since: 0,
            started: 0,
            match_started: None,
            focused,
            attrs: Default::default(),
        };
        assert_eq!(playing_of(&[]), None);
        let only = playing_of(&[state("pubg", false, "")]).unwrap();
        assert_eq!((only.game.as_str(), only.match_id.as_str()), ("pubg", ""));
        let two = [state("pubg", false, ""), state("lol", true, "m3")];
        let front = playing_of(&two).unwrap();
        assert_eq!(
            (front.game.as_str(), front.match_id.as_str()),
            ("lol", "m3")
        );
        assert_eq!(
            playing_of(&[state("pubg", false, ""), state("lol", false, "m3")]),
            None,
            "两个都在跑、都不在前台：不知道是哪个"
        );
    }

    /// 插件经入口发话（ADR 041 第 3 期）照调它的人的规矩：她、外部 Agent、热键；页面调的不能发。
    #[test]
    fn plugins_speak_for_whoever_called_their_tool() {
        use kotone_core::send_to_game::Speaker;
        assert_eq!(speaker_of(Caller::Her), Some(Speaker::Companion));
        assert_eq!(speaker_of(Caller::Service), Some(Speaker::Agent));
        assert_eq!(speaker_of(Caller::Hotkey), Some(Speaker::Automation));
        assert_eq!(speaker_of(Caller::Page("lol")), None);
        // 英雄联盟的一键喊话只给她（和热键、外部 Agent），页面调不到
        let shout = find("lol")
            .and_then(|p| p.tools().into_iter().find(|t| t.name == "lol.shout"))
            .expect("lol.shout");
        assert!(shout.for_model() && !shout.for_app());
    }

    #[test]
    fn game_voice_is_a_plugin_and_keeps_its_tool_names() {
        let gv = find("game-voice").expect("游戏语音插件");
        assert!(!gv.manifest().enabled_by_default);
        let names: Vec<String> = gv.tools().into_iter().map(|t| t.name).collect();
        assert!(
            names.iter().all(|n| n.starts_with("voice_chat.")),
            "{names:?}"
        );
        assert_eq!(owner("voice_chat.say").unwrap().manifest().id, "game-voice");
    }

    /// 语音输入搬进插件（ADR 038 第 5 步）：插件默认开，关掉以后语音输入热键、频道 / 重发 / 文字处理热键
    /// 都不登记，呼叫键照常。它不给她工具：`game_chat.send` 是本体的（ADR 039 D2，见下一个测试）；只有推送卡片上的
    /// “重发”给页面用（ADR 043 D10）。
    #[test]
    fn voice_input_is_a_plugin_and_its_switch_takes_the_hotkeys_with_it() {
        let vi = find("voice-input").expect("语音输入插件");
        assert!(vi.manifest().enabled_by_default);
        assert!(vi.tools().iter().all(|t| !t.for_model() && t.for_app()));
        assert!(owner("game_chat.send").is_none());

        // 语音输入的键存在热键表里（ADR 040 D8）
        let mut s = kotone_core::settings::Settings::default();
        s.set_binding(
            kotone_core::hotkey_bindings::CMD_CHANNEL_CYCLE,
            "Shift+CapsLock",
        );
        s.set_binding(kotone_core::hotkey_bindings::CMD_RESEND_LAST, "Alt+F6");
        s.sync_hotkey_mirrors();
        assert!(voice_input_on(&s));
        s.plugins.entry("voice-input".into()).or_default().enabled = Some(false);
        assert!(!voice_input_on(&s));
        let named = crate::hotkey::named_list_for_tests(&s);
        for id in [
            kotone_core::hotkey::HK_CHANNEL_CYCLE,
            kotone_core::hotkey::HK_RESEND_LAST,
        ] {
            assert!(
                named.iter().any(|(i, combo)| *i == id && combo.is_empty()),
                "{id}"
            );
        }
        assert!(named
            .iter()
            .any(|(i, combo)| *i == kotone_core::hotkey::HK_ASSISTANT && !combo.is_empty()));
    }

    /// ADR 039 D2：往游戏聊天里打字归本体。关掉语音输入插件，`game_chat.send` 照样在她的工具清单里
    /// （主会话也拿），也照样在工具服务里给外部 Agent（不属于哪个插件，只有一份）。
    #[test]
    fn game_chat_send_stays_when_voice_input_is_off() {
        let mut s = kotone_core::settings::Settings::default();
        s.plugins.entry("voice-input".into()).or_default().enabled = Some(false);
        assert!(!voice_input_on(&s));
        let specs = crate::tool_service::hosted_tool_specs(&s);
        let sends: Vec<&Value> = specs
            .iter()
            .filter(|spec| spec["name"] == "game_chat.send")
            .collect();
        assert_eq!(sends.len(), 1, "工具服务里有，只有一份");
        assert!(sends[0].get("plugin").is_none(), "{}", sends[0]);
        let offers = offers_for(&s);
        assert!(offers.iter().all(|o| o.plugin != "voice-input"));
        let catalog = kotone_agent::catalog::Catalog::build(&specs, &[], &offers);
        let send = catalog.get("game_chat.send").expect("她的清单里还有");
        assert!(kotone_agent::catalog::main_offers(send), "主会话也拿");
        assert_eq!(
            catalog
                .entries()
                .iter()
                .filter(|e| e.name == "game_chat.send")
                .count(),
            1
        );
    }

    /// 每个插件的每条内置推送，她的设置说明（`plugins.*`）里都写着开关：`pushes.<id> <名字>`（她照说明替玩家开关，
    /// 改了清单忘了改说明她就不知道）。插件只依赖插件契约，所以在宿主这里对所有插件一起查。
    #[test]
    fn every_push_switch_is_in_her_settings_guide() {
        let guide = kotone_core::settings_guide::FIELDS
            .iter()
            .find(|f| f.path == "plugins.*")
            .expect("设置说明里要有 plugins.*")
            .description;
        for p in registry() {
            let m = p.manifest();
            assert!(
                guide.contains(&format!("{}（{}", m.id, m.name)),
                "设置说明里没有插件 {}",
                m.id
            );
            for push in &m.pushes {
                let path = format!("pushes.{} {}", push.id, push.title);
                assert!(guide.contains(&path), "设置说明里没有 {} 的 {path}", m.id);
            }
        }
    }

    #[test]
    fn registry_manifests_and_tools_are_valid() {
        let mut ids = std::collections::HashSet::new();
        let mut tools = std::collections::HashSet::new();
        for p in registry() {
            let m = p.manifest();
            assert!(ids.insert(m.id.clone()), "插件 {} 重名", m.id);
            m.check_tools(&p.tools()).unwrap();
            for t in p.tools() {
                assert!(tools.insert(t.name.clone()), "工具 {} 重名", t.name);
                // 插件的工具不能和本体的工具重名（工具服务、她的清单都按名字找）
                assert!(
                    !kotone_tools::catalog::implemented_tools().contains(&t.name.as_str()),
                    "{} 和工具服务的工具重名",
                    t.name
                );
                assert!(
                    !kotone_agent::catalog::builtin_entries()
                        .iter()
                        .chain(&kotone_agent::catalog::agent_entries())
                        .any(|e| e.name == t.name)
                        && !kotone_agent::body::owns(&t.name),
                    "{} 和她的内置工具重名",
                    t.name
                );
            }
        }
        assert!(find("lol").is_some());
        assert_eq!(
            owner("lol.scout").map(|p| p.manifest().id.as_str()),
            Some("lol")
        );
        // 放歌搬进了插件（ADR 038 第 4 步）
        assert_eq!(
            owner("music").map(|p| p.manifest().id.as_str()),
            Some("music")
        );
        assert!(owner("browser.open").is_none());
    }

    /// MCP 方案第七节第 2 条：第三方插件的工具不进工具服务（外部 Agent、`kotone.exe mcp` 看不到、调不了），
    /// 她和它自己的页面能调；它的 App 资源也不进 `resources/list`。
    #[test]
    fn third_party_tools_stay_inside_kotone() {
        let id = "zz-remote-test";
        crate::remote_plugins::insert_for_test(crate::remote_plugins::test_plugin(id, 2));
        let s = kotone_core::settings::Settings::default();
        let tool = format!("{id}.t0");
        let specs = crate::tool_service::hosted_tool_specs(&s);
        assert!(
            specs
                .iter()
                .all(|spec| !spec["name"].as_str().unwrap_or("").starts_with(id)),
            "工具服务里没有第三方的工具"
        );
        assert!(owner(&tool).is_none(), "工具服务只认内置插件的工具");
        assert_eq!(
            any_owner(&tool).map(|p| p.manifest().id.clone()).as_deref(),
            Some(id)
        );
        let offer = offers_for(&s)
            .into_iter()
            .find(|o| o.plugin == id)
            .expect("她拿得到");
        assert_eq!(offer.tools.len(), 2, "只给页面的不进她的清单");
        assert!(offer.tools.iter().all(|t| t.untrusted));
        let catalog = kotone_agent::catalog::Catalog::build(&specs, &[], &[offer]);
        assert!(catalog.get(&tool).unwrap().untrusted);
        assert_eq!(third_party_refusal(true, Caller::Service), Some(NOT_SHARED));
        assert!(third_party_refusal(true, Caller::Her).is_none());
        assert!(
            third_party_refusal(true, Caller::Hotkey).is_none(),
            "玩家自己加的热键能调第三方的工具（在 Kotone 里按的）"
        );
        assert!(third_party_refusal(true, Caller::Page(id)).is_none());
        assert!(third_party_refusal(false, Caller::Service).is_none());
        assert!(crate::plugin_apps::list(&s)
            .iter()
            .all(|r| !r["uri"].as_str().unwrap_or("").contains(id)));
        crate::remote_plugins::remove_for_test(id);
    }

    #[test]
    fn ui_open_checks_the_page() {
        let s = kotone_core::settings::Settings::default();
        assert_eq!(
            resolve_page(&s, "ui://kotone/settings/plugins").unwrap(),
            ("plugins".to_string(), None)
        );
        // 设置里的“浏览器”一节（ADR 038：浏览器留在本体，开关在设置里）
        assert_eq!(
            resolve_page(&s, "ui://kotone/settings/browser").unwrap(),
            ("browser".to_string(), None)
        );
        assert!(resolve_page(&s, "ui://kotone/settings/nope").is_err());
        assert_eq!(
            resolve_page(&s, "ui://kotone/lol/settings")
                .unwrap()
                .1
                .as_deref(),
            Some("lol")
        );
        assert!(resolve_page(&s, "ui://kotone/lol/nope").is_err());
        assert!(resolve_page(&s, "ui://kotone/nope/x").is_err());
        assert!(resolve_page(&s, "https://example.com").is_err());
        // 第三方插件的页面：界面的右侧栏打不开，明说去插件页（不回“打开了”）
        let id = "zz-ui-open-test";
        crate::remote_plugins::insert_for_test(crate::remote_plugins::test_plugin(id, 1));
        let refused = resolve_page(&s, "ui://zz-ui-open-test/app").unwrap_err();
        assert!(
            refused.contains("只能在插件页里打开") && refused.contains(id),
            "{refused}"
        );
        crate::remote_plugins::remove_for_test(id);
        let mut off = kotone_core::settings::Settings::default();
        off.plugins.entry("lol".into()).or_default().enabled = Some(false);
        assert!(resolve_page(&off, "ui://kotone/lol/settings")
            .unwrap_err()
            .contains("关着"));
    }

    #[test]
    fn switches_follow_settings_and_manifest_defaults() {
        let lol = find("lol").unwrap().manifest();
        let mut s = kotone_core::settings::Settings::default();
        assert!(enabled(&s, lol) && source_enabled(&s, lol, "client"));
        let scout = find("lol")
            .unwrap()
            .tools()
            .into_iter()
            .find(|t| t.name == "lol.scout")
            .unwrap();
        assert!(tool_available(&s, lol, &scout));
        // 关掉数据开关：工具不出现
        s.plugins
            .entry("lol".into())
            .or_default()
            .sources
            .insert("client".into(), false);
        assert!(enabled(&s, lol) && !source_enabled(&s, lol, "client"));
        assert!(!tool_available(&s, lol, &scout));
        // 数据开关开着、插件关着：都算关
        let mut s = kotone_core::settings::Settings::default();
        s.plugins.entry("lol".into()).or_default().enabled = Some(false);
        assert!(!enabled(&s, lol) && !source_enabled(&s, lol, "client"));
        assert!(!source_enabled(
            &kotone_core::settings::Settings::default(),
            lol,
            "nope"
        ));
    }
}
