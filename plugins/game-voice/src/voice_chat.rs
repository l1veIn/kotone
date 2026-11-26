//! 游戏语音（ADR 020）：她用虚拟麦克风替玩家在游戏语音里说话。插件页面与工具共用这里的实现
//! （人在页面上能做的，她和外部 Agent 也能做）。
//!
//! - 插件开着且装了 VB-CABLE 时，转发器把真麦克风转进 CABLE Input（[`sync`]：启动时和每次保存设置后）；
//! - [`install`]：没装 VB-CABLE 时自动安装——从官网下载原版 `VBCABLE_Driver_Pack46.zip`（固定
//!   SHA-256），解压到 `<KOTONE_HOME>/drivers/vbcable/`，以管理员身份运行
//!   `VBCABLE_Setup_x64.exe -h -i`（照 Soundux 的安装脚本），等设备出现；
//! - [`default_mic`]：把 Windows 默认麦克风设成 CABLE Output（先记下原来的），或改回；
//! - [`say`]：一句话合成一次，送进虚拟麦克风，玩家自己也照常听到；聊天里留一张“她对队友说了”（[`crate::pushes`]）；
//! - [`test`]：往虚拟麦克风放 1 秒测试音，同时从 CABLE Output 录回来。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::cancel::CancellationToken;
use kotone_plugin_api::Host;
use serde_json::{json, Value};

use crate::cable::{self as platform, CableDevices};
use crate::config::VoiceChatConfig;
use crate::router::{self, RouterConfig, VoiceRouter};

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
    /// 她对队友说的话：推送“她对队友说了”、事件 `game-voice.said`（ADR 043 D11）。
    said: Arc<crate::pushes::SaidDesk>,
    /// 玩家这边那一遍（[`echo_to_player`]）：停下她时还在排队、正在放的都不放了。
    echoes: Mutex<CancellationToken>,
}

impl VoiceChat {
    fn router(&self) -> Option<Arc<VoiceRouter>> {
        self.inner
            .lock()
            .unwrap()
            .router
            .as_ref()
            .map(|(_, r)| r.clone())
    }

    /// 停掉还没说完的话（停止她时调用）：队友那边的，和玩家这边还在排队、正在放的那一遍。
    pub fn hush(&self) {
        std::mem::take(&mut *self.echoes.lock().unwrap()).cancel();
        if let Some(router) = self.router() {
            router.hush();
        }
    }
}

/// 转发开没开：插件开着，而且她开着（0.1.x 升级上来、还没打开她的玩家不转发）。
fn enabled(host: &dyn Host) -> bool {
    host.enabled() && host.companion_enabled()
}

fn config(host: &dyn Host) -> VoiceChatConfig {
    VoiceChatConfig::from_settings(&host.settings())
}

fn desired(enabled: bool, config: &VoiceChatConfig, cable: &CableDevices) -> Option<RouterKey> {
    if !enabled {
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
pub fn sync(state: Arc<VoiceChat>, host: Arc<dyn Host>) {
    let _ = std::thread::Builder::new()
        .name("kotone-voice-chat-sync".into())
        .spawn(move || sync_now(&state, host.as_ref()));
}

fn sync_now(state: &VoiceChat, host: &dyn Host) {
    let _serial = state.syncing.lock().unwrap();
    let on = enabled(host);
    let config = config(host);
    let want = if on {
        desired(on, &config, &platform::detect())
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
        inner.error =
            (on && !platform::detect().installed()).then(|| "还没有安装 VB-CABLE".to_string());
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

/// 状态：装没装、两个设备、开没开、转发在不在跑、Windows 默认麦克风、当前语音引擎能不能送进去。
pub fn status(state: &VoiceChat, host: &dyn Host) -> Value {
    let cable = platform::detect();
    let config = config(host);
    let (running, error, installing) = {
        let inner = state.inner.lock().unwrap();
        let device_error = inner.router.as_ref().and_then(|(_, r)| r.error());
        (
            inner.router.is_some(),
            device_error.or_else(|| inner.error.clone()),
            inner.installing,
        )
    };
    let (default_mic, communications_mic) = platform::default_capture().unwrap_or((None, None));
    let uses_cable = |e: &Option<platform::CaptureEndpoint>| {
        e.as_ref()
            .is_some_and(|e| platform::is_cable_output(&e.name))
    };
    json!({
        "driverInstalled": cable.installed(),
        "installing": installing,
        "devices": {"cableInput": cable.input, "cableOutput": cable.output},
        "enabled": enabled(host),
        "includeMic": config.include_mic,
        "micDeviceId": config.mic_device_id,
        "micGain": config.mic_gain,
        "running": running,
        "error": error,
        "windowsDefaultMic": default_mic,
        "windowsCommunicationsMic": communications_mic,
        "windowsDefaultIsCable": uses_cable(&default_mic) && uses_cable(&communications_mic),
        "canRestoreDefaultMic": config.previous_default_mic.is_some(),
        "ttsCanSpeak": host.can_synthesize(),
        "attribution": ATTRIBUTION,
    })
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

struct InstallingFlag(Arc<VoiceChat>);

impl Drop for InstallingFlag {
    fn drop(&mut self) {
        self.0.inner.lock().unwrap().installing = false;
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("游戏语音任务异常：{e}"))?
}

/// 自动安装 VB-CABLE：下载官方安装包 → 校验 → 解压 → 以管理员身份静默安装 → 等设备出现。
/// 已经装了就什么也不做。
pub async fn install(state: &Arc<VoiceChat>, host: Arc<dyn Host>) -> Result<Value, String> {
    if blocking(|| Ok(platform::detect().installed())).await? {
        return Ok(json!({"installed": true, "alreadyInstalled": true}));
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.installing {
            return Err("正在安装 VB-CABLE".into());
        }
        inner.installing = true;
    }
    let flag = InstallingFlag(state.clone());
    let dir = drivers_dir();
    let zip_path = dir.join(PACK_FILE);
    host.download(PACK_URL, zip_path.clone(), Some(PACK_SHA256))
        .await
        .map_err(|e| format!("下载 VB-CABLE 失败：{e}"))?;
    let code = blocking(move || {
        let unpacked = dir.join("pack");
        let _ = std::fs::remove_dir_all(&unpacked);
        extract(&zip_path, &unpacked)?;
        let setup = unpacked.join("VBCABLE_Setup_x64.exe");
        if !setup.is_file() {
            return Err("安装包里没有 VBCABLE_Setup_x64.exe".into());
        }
        kotone_core::log::log("voice chat: running VB-CABLE setup (elevated)");
        platform::run_elevated_and_wait(
            &setup.to_string_lossy(),
            SETUP_ARGS,
            Duration::from_secs(300),
        )
    })
    .await?;
    // 设备要过一会儿才出现在列表里。
    let appeared = blocking(|| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if platform::detect().installed() {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Ok(false)
    })
    .await?;
    drop(flag);
    if appeared {
        sync(state.clone(), host);
        return Ok(json!({"installed": true, "setupExitCode": code, "needsReboot": false}));
    }
    Ok(json!({
        "installed": false,
        "setupExitCode": code,
        "needsReboot": true,
        "note": "安装程序已运行，但设备还没出现：按 VB-Audio 的说明需要重启电脑后才能用",
    }))
}

/// 把 Windows 默认麦克风设成 CABLE Output（先记下原来的），或改回原来的。
pub async fn default_mic(host: Arc<dyn Host>, use_cable: bool) -> Result<Value, String> {
    blocking(move || default_mic_now(host.as_ref(), use_cable)).await
}

fn default_mic_now(host: &dyn Host, use_cable: bool) -> Result<Value, String> {
    if use_cable {
        let cable = platform::capture_endpoints()?
            .into_iter()
            .find(|e| platform::is_cable_output(&e.name))
            .ok_or("没有找到 CABLE Output：先安装 VB-CABLE")?;
        let (console, communications) = platform::default_capture()?;
        let is_cable = |e: &Option<platform::CaptureEndpoint>| {
            e.as_ref()
                .is_some_and(|e| platform::is_cable_output(&e.name))
        };
        // 已经是 CABLE 时不覆盖记下的“原来的设备”。
        if !is_cable(&console) || !is_cable(&communications) {
            let prev_console = console.filter(|e| !platform::is_cable_output(&e.name));
            let prev_comm = communications.filter(|e| !platform::is_cable_output(&e.name));
            let mut patch = serde_json::Map::new();
            if let Some(e) = &prev_console {
                patch.insert("previousDefaultMic".into(), e.id.clone().into());
            }
            if let Some(e) = &prev_comm {
                patch.insert("previousCommunicationsMic".into(), e.id.clone().into());
            }
            if !patch.is_empty() {
                host.update_settings(Value::Object(patch))?;
            }
        }
        platform::set_default_capture(&cable.id, true)?;
        kotone_core::log::log("voice chat: Windows default mic -> CABLE Output");
        Ok(json!({"defaultMic": cable.name, "restorable": true}))
    } else {
        let config = config(host);
        let prev = config
            .previous_default_mic
            .ok_or("没有记下原来的默认麦克风（Kotone 没有改过它）")?;
        let prev_comm = config.previous_communications_mic;
        platform::set_default_capture(&prev, prev_comm.is_none())?;
        if let Some(comm) = &prev_comm {
            platform::set_default_capture(comm, true)?;
            // 控制台角色再设回原来的（上一句把三个角色都设成了通信设备）。
            platform::set_default_capture(&prev, false)?;
        }
        host.update_settings(json!({
            "previousDefaultMic": null,
            "previousCommunicationsMic": null,
        }))?;
        let (console, _) = platform::default_capture()?;
        kotone_core::log::log("voice chat: Windows default mic restored");
        Ok(json!({"defaultMic": console.map(|e| e.name), "restored": true}))
    }
}

fn running_router(state: &VoiceChat, host: &dyn Host) -> Result<Arc<VoiceRouter>, String> {
    state.router().ok_or_else(|| {
        if !platform::detect().installed() {
            "还没有安装 VB-CABLE（voice_chat.install）".to_string()
        } else if !enabled(host) {
            "游戏语音插件没有打开（插件 → 游戏语音）".to_string()
        } else {
            state
                .inner
                .lock()
                .unwrap()
                .error
                .clone()
                .unwrap_or_else(|| "游戏语音还没准备好，稍后再试".to_string())
        }
    })
}

/// 把一句话说给队友听：合成一次送进虚拟麦克风，玩家自己也照常从耳机里听到（[`echo_to_player`]）。
pub async fn say(state: &Arc<VoiceChat>, host: Arc<dyn Host>, text: &str) -> Result<Value, String> {
    let router = {
        let (state, host) = (state.clone(), host.clone());
        blocking(move || running_router(&state, host.as_ref())).await?
    };
    let (samples, rate) = host.synthesize(text).await?;
    let duration_ms = router.say(&samples, rate);
    echo_to_player(state, &host, text);
    // 以玩家的名义对队友说的话，聊天里都留一条（和语音输入“你向「队伍」发了”对称）
    state.said.report(host.clone(), text);
    kotone_core::log::log(&format!(
        "voice chat: said {} chars to teammates",
        text.chars().count()
    ));
    Ok(json!({"sent": true, "durationMs": duration_ms}))
}

/// 玩家这边也听一遍：交给宿主，和她自己的话轮流放（她正在说就排在她后面，放的时候音乐压低）。
/// 同一句已经在缓存里，轮到了马上出声。工具不等它放完：她常在同一步里先 `voice_chat.say` 再 `speak`
/// “说了”，边生成边念的那句在模型写参数时就开口、拿着播报，要等 `speak` 执行才收尾，而工具是一个一个
/// 执行的——这里等它，两边就互相等着。宿主放不出来时会改用弹幕显示这句，这里记一笔原因；队友那边不受影响。
fn echo_to_player(state: &VoiceChat, host: &Arc<dyn Host>, text: &str) {
    let stop = state.echoes.lock().unwrap().clone();
    let speak = host.speak(text);
    host.spawn(Box::pin(async move {
        tokio::select! {
            _ = stop.cancelled() => {}
            result = speak => {
                if let Err(e) = result {
                    kotone_core::log::log(&format!("voice chat: local playback failed: {e}"));
                }
            }
        }
    }));
}

/// 自检：往虚拟麦克风放 1 秒测试音，同时从 CABLE Output 录回来（约 1.5 秒）。
pub async fn test(state: &Arc<VoiceChat>, host: &dyn Host) -> Result<Value, String> {
    let router = running_router(state, host)?;
    blocking(move || {
        let output = platform::detect().output.ok_or("没有找到 CABLE Output")?;
        let (tone, rate) = router::test_tone(1.0);
        let probe = std::thread::spawn(move || router::measure_peak(&output, 1_500));
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
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use kotone_plugin_api::{BoxFuture, Observation};

    /// 宿主那边“她正在说”：`speak` 一直轮不到（排在她的话后面），被丢掉时记一笔。
    #[derive(Default)]
    struct BusyHost {
        asked: AtomicUsize,
        dropped: Arc<AtomicUsize>,
    }

    struct Dropped(Arc<AtomicUsize>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Host for BusyHost {
        fn plugin(&self) -> &str {
            "game-voice"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _: &str) -> bool {
            true
        }
        fn settings(&self) -> Value {
            Value::Null
        }
        fn data_dir(&self) -> PathBuf {
            PathBuf::new()
        }
        fn spawn(&self, task: BoxFuture<'static, ()>) {
            tokio::spawn(task);
        }
        fn process_ids(&self, _: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _: &str, _: Observation) {}
        fn game_claim(&self, _: &str, _: bool) {}
        fn game_event(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: serde_json::Map<String, Value>,
            _: Option<f64>,
        ) {
        }
        fn speak(&self, _: &str) -> BoxFuture<'static, Result<(), String>> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let dropped = Dropped(self.dropped.clone());
            Box::pin(async move {
                let _dropped = dropped;
                std::future::pending::<Result<(), String>>().await
            })
        }
    }

    #[tokio::test]
    async fn the_player_side_echo_does_not_hold_up_the_tool() {
        let state = VoiceChat::default();
        let busy = Arc::new(BusyHost::default());
        let host: Arc<dyn Host> = busy.clone();
        // 她这一步里接着要 `speak`，那句拿着播报：这里要是等，就和她互相等着
        echo_to_player(&state, &host, "我去拿龙了");
        tokio::task::yield_now().await;
        assert_eq!(busy.asked.load(Ordering::SeqCst), 1, "交给宿主排队了");
        assert_eq!(busy.dropped.load(Ordering::SeqCst), 0, "还在排队，没丢");
        // 玩家让她停下：还在排队的那一遍也不放了；之后再说照常排队
        state.hush();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while busy.dropped.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("停下她时丢掉还在排队的那一遍");
        echo_to_player(&state, &host, "撤了");
        tokio::task::yield_now().await;
        assert_eq!(busy.asked.load(Ordering::SeqCst), 2);
        assert_eq!(busy.dropped.load(Ordering::SeqCst), 1);
    }

    fn cable() -> CableDevices {
        CableDevices {
            input: Some("CABLE Input (VB-Audio Virtual Cable)".into()),
            output: Some("CABLE Output (VB-Audio Virtual Cable)".into()),
        }
    }

    #[test]
    fn routes_only_when_enabled_and_installed() {
        let mut config = VoiceChatConfig::default();
        assert_eq!(desired(false, &config, &cable()), None, "off by default");
        let key = desired(true, &config, &cable()).unwrap();
        assert_eq!(key.output, "CABLE Input (VB-Audio Virtual Cable)");
        assert_eq!(key.mic.as_deref(), Some("default"));
        config.include_mic = false;
        assert_eq!(desired(true, &config, &cable()).unwrap().mic, None);
        assert_eq!(
            desired(true, &config, &CableDevices::default()),
            None,
            "no cable"
        );
    }
}
