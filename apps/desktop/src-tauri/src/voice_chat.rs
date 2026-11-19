//! 游戏语音（ADR 020）：琴音用虚拟麦克风替玩家在游戏语音里说话。设置页与工具共用这里的实现
//! （人在设置页能做的，琴音和外部 Agent 也能做）。
//!
//! - 开着 `voiceChat.enabled` 且装了 VB-CABLE 时，转发器把真麦克风转进 CABLE Input（[`sync`]，
//!   设置保存后经运行时状态推送点调用）；
//! - [`install`]：没装 VB-CABLE 时自动安装——从官网下载原版 `VBCABLE_Driver_Pack46.zip`（固定
//!   SHA-256），解压到 `<KOTONE_HOME>/drivers/vbcable/`，以管理员身份运行
//!   `VBCABLE_Setup_x64.exe -h -i`（照 Soundux 的安装脚本），等设备出现；
//! - [`default_mic`]：把 Windows 默认麦克风设成 CABLE Output（先记下原来的），或改回；
//! - [`say`]：一句话合成一次，送进虚拟麦克风，玩家自己也照常听到；
//! - [`test`]：往虚拟麦克风放 1 秒测试音，同时从 CABLE Output 录回来。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::voice_chat::VoiceChatConfig;
use kotone_platform_windows::voice_chat::{self as platform, CableDevices};
use kotone_platform_windows::voice_router::{self, RouterConfig, VoiceRouter};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// VB-CABLE 官方安装包（VB-Audio 捐赠软件，https://vb-cable.com）。
const PACK_URL: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip";
const PACK_FILE: &str = "VBCABLE_Driver_Pack46.zip";
/// 2026-11-17 从官网下载后计算。官网换了新版本时校验会失败，要在这里更新版本与哈希。
const PACK_SHA256: &str = "bc44e8974191a5a2427d6c979e2dab08dbb09925f3f435dfa2f0ba3e45b6eb90";
/// 静默安装参数（Soundux 的安装脚本：`Parameters: "-h -i"`）。
const SETUP_ARGS: &str = "-h -i";
/// 告诉玩家这是谁的软件（VB-Audio 的分发要求）。
pub const ATTRIBUTION: &str = "VB-CABLE 是 VB-Audio 的捐赠软件（https://vb-cable.com），安装即表示同意它的许可；欢迎到官网捐助作者。";

/// 转发器的配置（变了就重开）。
#[derive(Debug, Clone, PartialEq)]
struct RouterKey {
    output: String,
    mic: Option<String>,
    gain: u16,
}

#[derive(Default)]
struct Inner {
    router: Option<(RouterKey, Arc<VoiceRouter>)>,
    /// 最近一次打开转发失败的原因。
    error: Option<String>,
    installing: bool,
}

#[derive(Default)]
pub struct VoiceChat {
    inner: Mutex<Inner>,
    /// `sync` 串行执行（打开设备要一点时间）。
    syncing: Mutex<()>,
}

fn config(app: &AppHandle) -> VoiceChatConfig {
    let state = app.state::<crate::SharedState>();
    let settings = state.settings.read().unwrap();
    let mut config = settings.voice_chat.clone();
    if settings.ui.legacy_upgrade.uses_legacy_assistant() {
        config.enabled = false;
    }
    config
}

fn desired(config: &VoiceChatConfig, cable: &CableDevices) -> Option<RouterKey> {
    if !config.enabled {
        return None;
    }
    Some(RouterKey {
        output: cable.input.clone()?,
        mic: config.include_mic.then(|| {
            let id = config.mic_device_id.trim();
            if id.is_empty() {
                "default".to_string()
            } else {
                id.to_string()
            }
        }),
        gain: config.mic_gain,
    })
}

/// 按设置启停转发（幂等；在后台线程里做，不阻塞调用方）。
pub fn sync(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("kotone-voice-chat-sync".into())
        .spawn(move || sync_now(&app));
}

fn sync_now(app: &AppHandle) {
    let Some(state) = app.try_state::<VoiceChat>() else {
        return;
    };
    let _serial = state.syncing.lock().unwrap();
    let config = config(app);
    let want = if config.enabled {
        desired(&config, &platform::detect())
    } else {
        None
    };
    let current = state
        .inner
        .lock()
        .unwrap()
        .router
        .as_ref()
        .map(|(k, _)| k.clone());
    if want == current {
        return;
    }
    // 先关旧的（同一个设备不能开两次）。
    let old = state.inner.lock().unwrap().router.take();
    drop(old);
    let Some(key) = want else {
        let mut inner = state.inner.lock().unwrap();
        inner.error = (config.enabled && !platform::detect().installed())
            .then(|| "还没有安装 VB-CABLE".to_string());
        return;
    };
    let started = VoiceRouter::start(RouterConfig {
        output: key.output.clone(),
        mic: key.mic.clone(),
        mic_gain: config.mic_gain_factor(),
    });
    let mut inner = state.inner.lock().unwrap();
    match started {
        Ok(router) => {
            kotone_core::log::log(&format!(
                "voice chat: routing {:?} -> {} at {} Hz",
                key.mic,
                key.output,
                router.sample_rate()
            ));
            inner.router = Some((key, Arc::new(router)));
            inner.error = None;
        }
        Err(e) => {
            kotone_core::log::log(&format!("voice chat: start failed: {e}"));
            inner.error = Some(e);
        }
    }
}

fn router(app: &AppHandle) -> Option<Arc<VoiceRouter>> {
    app.try_state::<VoiceChat>()?
        .inner
        .lock()
        .unwrap()
        .router
        .as_ref()
        .map(|(_, r)| r.clone())
}

/// 状态：装没装、两个设备、开没开、转发在不在跑、Windows 默认麦克风、当前语音引擎能不能送进去。
pub fn status(app: &AppHandle) -> Value {
    let cable = platform::detect();
    let config = config(app);
    let (running, error, installing) = match app.try_state::<VoiceChat>() {
        Some(state) => {
            let inner = state.inner.lock().unwrap();
            let device_error = inner.router.as_ref().and_then(|(_, r)| r.error());
            (
                inner.router.is_some(),
                device_error.or_else(|| inner.error.clone()),
                inner.installing,
            )
        }
        None => (false, None, false),
    };
    let (default_mic, communications_mic) = platform::default_capture().unwrap_or((None, None));
    let uses_cable = |e: &Option<platform::CaptureEndpoint>| {
        e.as_ref()
            .is_some_and(|e| platform::is_cable(&e.name, platform::CABLE_OUTPUT))
    };
    let engine = {
        let settings = app.state::<crate::SharedState>();
        let assistant = settings.settings.read().unwrap().assistant.clone();
        crate::assistant::effective_engine(app, &assistant)
    };
    json!({
        "driverInstalled": cable.installed(),
        "installing": installing,
        "devices": {"cableInput": cable.input, "cableOutput": cable.output},
        "enabled": config.enabled,
        "includeMic": config.include_mic,
        "micDeviceId": config.mic_device_id,
        "micGain": config.mic_gain,
        "running": running,
        "error": error,
        "windowsDefaultMic": default_mic,
        "windowsCommunicationsMic": communications_mic,
        "windowsDefaultIsCable": uses_cable(&default_mic) && uses_cable(&communications_mic),
        "canRestoreDefaultMic": config.previous_default_mic.is_some(),
        "ttsCanSpeak": engine != kotone_core::assistant::TtsEngine::System,
        "attribution": ATTRIBUTION,
    })
}

/// 改设置并通知设置窗口刷新。
fn update_settings(
    app: &AppHandle,
    change: impl FnOnce(&mut kotone_core::settings::Settings) + Send + 'static,
) -> Result<(), String> {
    app.state::<crate::SharedState>()
        .settings_repository
        .update(move |next| {
            change(next);
            Ok(())
        })?;
    crate::tool_config::notify_settings_changed(app);
    Ok(())
}

fn drivers_dir() -> PathBuf {
    kotone_core::settings::kotone_dir()
        .join("drivers")
        .join("vbcable")
}

/// 解压官方安装包（只取包里的普通文件，防路径穿越）。
fn extract(zip_path: &std::path::Path, dir: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开安装包失败：{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("安装包损坏：{e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("安装包损坏：{e}"))?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let target = dir.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| format!("解压失败：{e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("解压失败：{e}"))?;
    }
    Ok(())
}

struct InstallingFlag(AppHandle);

impl Drop for InstallingFlag {
    fn drop(&mut self) {
        if let Some(state) = self.0.try_state::<VoiceChat>() {
            state.inner.lock().unwrap().installing = false;
        }
    }
}

/// 自动安装 VB-CABLE（阻塞，从工作线程调用）：下载官方安装包 → 校验 → 解压 → 以管理员身份
/// 静默安装 → 等设备出现。已经装了就什么也不做。
pub fn install(app: &AppHandle) -> Result<Value, String> {
    if platform::detect().installed() {
        return Ok(json!({"installed": true, "alreadyInstalled": true}));
    }
    let state = app.state::<VoiceChat>();
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.installing {
            return Err("正在安装 VB-CABLE".into());
        }
        inner.installing = true;
    }
    let _flag = InstallingFlag(app.clone());
    let dir = drivers_dir();
    let zip_path = dir.join(PACK_FILE);
    {
        let _guard = kotone_stt::download::begin_download()?;
        kotone_stt::download::download_file(PACK_URL, &zip_path, Some(PACK_SHA256), &|_, _| {})
            .map_err(|e| format!("下载 VB-CABLE 失败：{e}"))?;
    }
    let unpacked = dir.join("pack");
    let _ = std::fs::remove_dir_all(&unpacked);
    extract(&zip_path, &unpacked)?;
    let setup = unpacked.join("VBCABLE_Setup_x64.exe");
    if !setup.is_file() {
        return Err("安装包里没有 VBCABLE_Setup_x64.exe".into());
    }
    kotone_core::log::log("voice chat: running VB-CABLE setup (elevated)");
    let code = platform::run_elevated_and_wait(
        &setup.to_string_lossy(),
        SETUP_ARGS,
        Duration::from_secs(300),
    )?;
    // 设备要过一会儿才出现在列表里。
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if platform::detect().installed() {
            sync(app);
            return Ok(json!({"installed": true, "setupExitCode": code, "needsReboot": false}));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(json!({
        "installed": false,
        "setupExitCode": code,
        "needsReboot": true,
        "note": "安装程序已运行，但设备还没出现：按 VB-Audio 的说明需要重启电脑后才能用",
    }))
}

/// 把 Windows 默认麦克风设成 CABLE Output（先记下原来的），或改回原来的。
pub fn default_mic(app: &AppHandle, use_cable: bool) -> Result<Value, String> {
    let shared = app.state::<crate::SharedState>();
    if use_cable {
        let cable = platform::capture_endpoints()?
            .into_iter()
            .find(|e| platform::is_cable(&e.name, platform::CABLE_OUTPUT))
            .ok_or("没有找到 CABLE Output：先安装 VB-CABLE")?;
        let (console, communications) = platform::default_capture()?;
        let is_cable = |e: &Option<platform::CaptureEndpoint>| {
            e.as_ref()
                .is_some_and(|e| platform::is_cable(&e.name, platform::CABLE_OUTPUT))
        };
        // 已经是 CABLE 时不覆盖记下的“原来的设备”。
        if !is_cable(&console) || !is_cable(&communications) {
            let prev_console =
                console.filter(|e| !platform::is_cable(&e.name, platform::CABLE_OUTPUT));
            let prev_comm =
                communications.filter(|e| !platform::is_cable(&e.name, platform::CABLE_OUTPUT));
            update_settings(app, move |s| {
                if let Some(e) = &prev_console {
                    s.voice_chat.previous_default_mic = Some(e.id.clone());
                }
                if let Some(e) = &prev_comm {
                    s.voice_chat.previous_communications_mic = Some(e.id.clone());
                }
            })?;
        }
        platform::set_default_capture(&cable.id, true)?;
        kotone_core::log::log("voice chat: Windows default mic -> CABLE Output");
        Ok(json!({"defaultMic": cable.name, "restorable": true}))
    } else {
        let (prev, prev_comm) = {
            let s = shared.settings.read().unwrap();
            (
                s.voice_chat.previous_default_mic.clone(),
                s.voice_chat.previous_communications_mic.clone(),
            )
        };
        let prev = prev.ok_or("没有记下原来的默认麦克风（Kotone 没有改过它）")?;
        platform::set_default_capture(&prev, prev_comm.is_none())?;
        if let Some(comm) = &prev_comm {
            platform::set_default_capture(comm, true)?;
            // 控制台角色再设回原来的（上一句把三个角色都设成了通信设备）。
            platform::set_default_capture(&prev, false)?;
        }
        update_settings(app, |s| {
            s.voice_chat.previous_default_mic = None;
            s.voice_chat.previous_communications_mic = None;
        })?;
        let (console, _) = platform::default_capture()?;
        kotone_core::log::log("voice chat: Windows default mic restored");
        Ok(json!({"defaultMic": console.map(|e| e.name), "restored": true}))
    }
}

fn running_router(app: &AppHandle) -> Result<Arc<VoiceRouter>, String> {
    router(app).ok_or_else(|| {
        if !platform::detect().installed() {
            "还没有安装 VB-CABLE（voice_chat.install）".to_string()
        } else if !config(app).enabled {
            "游戏语音没有开启（设置 voiceChat.enabled）".to_string()
        } else {
            app.try_state::<VoiceChat>()
                .and_then(|s| s.inner.lock().unwrap().error.clone())
                .unwrap_or_else(|| "游戏语音还没准备好，稍后再试".to_string())
        }
    })
}

/// 把一句话说给队友听（阻塞）：合成一次送进虚拟麦克风，玩家自己也照常从耳机里听到。
pub fn say(app: &AppHandle, text: &str) -> Result<Value, String> {
    let router = running_router(app)?;
    let (speaker, request) = crate::assistant::speaker_and_request(app, text)?;
    let (samples, rate) = speaker.synthesize(request.clone())?;
    let duration_ms = router.say(&samples, rate);
    // 同一句已经在缓存里，玩家这边几乎同时出声；本机播放失败不影响队友那边。
    if let Err(e) = speaker.speak(request) {
        kotone_core::log::log(&format!("voice chat: local playback failed: {e}"));
    }
    kotone_core::log::log(&format!(
        "voice chat: said {} chars to teammates",
        text.chars().count()
    ));
    Ok(json!({"sent": true, "durationMs": duration_ms}))
}

/// 自检：往虚拟麦克风放 1 秒测试音，同时从 CABLE Output 录回来（阻塞约 1.5 秒）。
pub fn test(app: &AppHandle) -> Result<Value, String> {
    let router = running_router(app)?;
    let output = platform::detect().output.ok_or("没有找到 CABLE Output")?;
    let (tone, rate) = voice_router::test_tone(1.0);
    let probe = std::thread::spawn(move || voice_router::measure_peak(&output, 1_500));
    std::thread::sleep(Duration::from_millis(150));
    router.say(&tone, rate);
    let peak = probe.join().map_err(|_| "录音线程异常".to_string())??;
    let ok = peak > 0.15;
    Ok(json!({
        "ok": ok,
        "peak": (f64::from(peak) * 1000.0).round() / 1000.0,
        "note": if ok {
            "通了：游戏或语音软件的麦克风选 CABLE Output，队友就能听到"
        } else {
            "没录到测试音：检查 VB-CABLE 是否装好（可能要重启电脑）"
        },
    }))
}

/// 停掉还没说完的话（停止琴音时调用）。
pub fn hush(app: &AppHandle) {
    if let Some(router) = router(app) {
        router.hush();
    }
}

async fn blocking(
    app: AppHandle,
    work: impl FnOnce(&AppHandle) -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || work(&app))
        .await
        .map_err(|e| format!("游戏语音任务异常：{e}"))?
}

#[tauri::command]
pub fn voice_chat_status(app: AppHandle) -> Value {
    status(&app)
}

#[tauri::command]
pub async fn voice_chat_install(app: AppHandle) -> Result<Value, String> {
    blocking(app, install).await
}

#[tauri::command]
pub async fn voice_chat_default_mic(app: AppHandle, use_cable: bool) -> Result<Value, String> {
    blocking(app, move |app| default_mic(app, use_cable)).await
}

#[tauri::command]
pub async fn voice_chat_test(app: AppHandle) -> Result<Value, String> {
    blocking(app, test).await
}

#[tauri::command]
pub async fn voice_chat_say(app: AppHandle, text: String) -> Result<Value, String> {
    blocking(app, move |app| say(app, &text)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cable() -> CableDevices {
        CableDevices {
            input: Some("CABLE Input (VB-Audio Virtual Cable)".into()),
            output: Some("CABLE Output (VB-Audio Virtual Cable)".into()),
        }
    }

    #[test]
    fn routes_only_when_enabled_and_installed() {
        let mut config = VoiceChatConfig::default();
        assert_eq!(desired(&config, &cable()), None, "off by default");
        config.enabled = true;
        let key = desired(&config, &cable()).unwrap();
        assert_eq!(key.output, "CABLE Input (VB-Audio Virtual Cable)");
        assert_eq!(key.mic.as_deref(), Some("default"));
        config.include_mic = false;
        assert_eq!(desired(&config, &cable()).unwrap().mic, None);
        assert_eq!(desired(&config, &CableDevices::default()), None, "no cable");
    }
}
