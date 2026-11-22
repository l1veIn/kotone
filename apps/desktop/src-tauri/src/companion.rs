//! Kotone 陪玩（docs/product-evolution-game-tools.md §10）。
//!
//! 陪玩热键（按住说话，经编排器识别后进入 [`handle_assistant`]）与快捷消息热键（[`quick`]，
//! 不经过语音识别）共用同一条流程，工具 `companion.*` 也调用这里：
//!
//! 1. 截前台窗口（GDI 前台可见客户区）；profile 记有聊天框区域时只识别那块；
//! 2. 用统一模型清单里选定的 OCR 模型识别，每行带截图坐标；
//! 3. 写入本流程自己的聊天跟踪，附上最近的跟踪记录与 profile 说明；
//! 4. 一次大模型调用（`kotone_ai::companion`），从提醒 / 帮忙打字 / 回答中选一种；
//! 5. 执行：提醒交给调度器，打字经 profile 的注入设置打进聊天框（默认按发送键，`autoSend` 关掉时不按），
//!    回答走与提醒相同的语音 / 弹幕交付路径。模型回报的聊天框位置记进 profile。
//!
//! 必须选好 AI 连接并保存密钥；没有时不降级，只提示用户先去配置。失败都如实告诉用户
//! （热键入口用语音 / 弹幕播出，工具入口返回错误），不把失败说成“没看清”。

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kotone_ai::companion::{self as planner, CompanionAction, CompanionInput, OcrLine};
use kotone_core::assistant::{AssistantPlan, CompanionCheck};
use kotone_core::cancel::CancellationToken;
use kotone_core::inject::CancelToken;
use kotone_core::ocr::{OcrBackend, OcrBackendKind, OcrCancel, OcrInput, OcrParams};
use kotone_core::profile::{self, ChatRegion, GameProfile};
use kotone_core::reminders::ReminderIntent;
use kotone_core::settings::Settings;
use kotone_core::tools::WindowObserver;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_tools::tracking::{
    IngestInput, IngestLine, OnSourceChange, Region, SourceTag, TrackLimits, Tracker,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 截图像素上限（4K 客户区以内）。
const MAX_CAPTURE_PIXELS: u64 = 3840 * 2160;
const OCR_DEADLINE: Duration = Duration::from_secs(20);
const TYPE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct CompanionRuntime {
    tracker: Tracker,
    /// 同一时间只处理一条陪玩请求。
    busy: Arc<tokio::sync::Mutex<()>>,
}

impl Default for CompanionRuntime {
    fn default() -> Self {
        Self {
            tracker: Tracker::new(TrackLimits::default()),
            busy: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// 请求来源（写进提示词与回执）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Voice,
    QuickMessage,
    Text,
    Tool,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Self::Voice => "voice",
            Self::QuickMessage => "quick-message",
            Self::Text => "text",
            Self::Tool => "tool",
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn settings(app: &AppHandle) -> Settings {
    app.state::<SharedState>().settings.read().unwrap().clone()
}

pub(crate) fn active_profile(settings: &Settings) -> GameProfile {
    settings
        .active_profile_id
        .as_deref()
        .and_then(profile::get)
        .unwrap_or_else(GameProfile::builtin_generic)
}

// ---------- 就绪检查 ----------

/// AI 连接是否可用于陪玩：已选、有密钥就行（不再要求能力自检；“测试连接”只是给用户手动确认）。
pub fn connection_ready(
    app: &AppHandle,
    settings: &Settings,
) -> Result<kotone_core::connection::ResolvedConnection, String> {
    let id = settings.companion_connection_id().trim();
    if id.is_empty() {
        return Err("还没有选 API 连接：请在「设置 → 她怎么回答 → 她用的模型」里选一条（还没有连接就先在「设置 → API 连接」里添加）".into());
    }
    let connection = app.state::<SharedState>().connections.resolve(id)?;
    if connection
        .api_key
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err("这条 API 连接还没有保存 Key，请在「设置 → API 连接」里填写".into());
    }
    Ok(connection)
}

/// 当前 OCR 模型及其后端；没有可用模型时给出可操作的提示。
pub(crate) fn ocr_backend(
    app: &AppHandle,
    settings: &Settings,
) -> Result<(String, Arc<dyn OcrBackend>), String> {
    let root = kotone_stt::model::models_dir_from(settings);
    let model =
        kotone_ocr::catalog::resolve_active(settings, &root, crate::model_catalog::verifier())
            .ok_or("看游戏聊天需要文字识别模型：请在「设置 → 模型 → 文字识别」下载并选择一个")?;
    let backend = app
        .state::<crate::ocr_engines::OcrEngines>()
        .by_model(settings, &model)
        .ok_or_else(|| format!("所选文字识别模型 {model} 不在清单里，请重新选择"))?;
    Ok((model, backend))
}

/// 陪玩整体是否可用（工具 `companion.status` 与设置页展示）。
pub fn status(app: &AppHandle) -> Value {
    let s = settings(app);
    let connection = connection_ready(app, &s);
    let (ocr_model, ocr) = match ocr_backend(app, &s) {
        Ok((model, backend)) => {
            let info = backend.info();
            (
                Some(model),
                json!({"ready": info.ready, "reason": info.reason, "backend": info.id, "dataEgress": info.data_egress}),
            )
        }
        Err(reason) => (None, json!({"ready": false, "reason": reason})),
    };
    let ocr_ready = ocr["ready"].as_bool().unwrap_or(false);
    let reason = connection.as_ref().err().cloned().or_else(|| {
        (!ocr_ready).then(|| {
            ocr["reason"]
                .as_str()
                .unwrap_or("文字识别模型不可用")
                .to_string()
        })
    });
    json!({
        "ready": reason.is_none(),
        "reason": reason,
        "route": s.assistant.route,
        "connectionId": s.companion_connection_id(),
        "check": s.assistant.companion.check,
        "ocr": {"modelId": ocr_model, "state": ocr},
        "autoSend": s.assistant.companion.auto_send,
        "quickMessages": s.assistant.companion.quick_messages,
        "profile": {"id": active_profile(&s).id, "notes": active_profile(&s).companion_notes, "chatRegion": active_profile(&s).chat_region},
    })
}

// ---------- 读聊天（流程前半段） ----------

/// 截图 + OCR + 跟踪的结果（工具 `companion.read_chat` 的返回，也是规划的输入）。
pub struct ChatSnapshot {
    pub input: CompanionInput,
    pub window_size: (u32, u32),
    pub phases: Vec<(&'static str, u64)>,
    pub ocr_backend: String,
    pub data_egress: String,
    pub warnings: Vec<String>,
}

impl ChatSnapshot {
    pub fn to_json(&self) -> Value {
        json!({
            "window": {"title": self.input.window_title, "process": self.input.process_name, "width": self.window_size.0, "height": self.window_size.1},
            "ocr": {
                "model": self.input.ocr_model,
                "backend": self.ocr_backend,
                "dataEgress": self.data_egress,
                "region": self.input.ocr_region,
                "lines": self.input.ocr_lines.iter().map(|l| json!({"text": l.text, "box": l.bbox, "confidence": l.confidence})).collect::<Vec<_>>(),
            },
            "chatTracking": self.input.tracked.iter().map(|m| json!({"messageId": m.id, "text": m.text, "newThisRead": m.new, "firstSeenMsAgo": m.first_seen_ago_ms})).collect::<Vec<_>>(),
            "profile": {"name": self.input.profile_name, "notes": self.input.profile_notes},
            "warnings": self.warnings,
            "timing": self.phases.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
        })
    }
}

fn region_px(
    region: &ChatRegion,
    width: u32,
    height: u32,
) -> Option<kotone_tools::image::CropRect> {
    region.validate().ok()?;
    let x = (region.x * f64::from(width)).floor().max(0.0) as u32;
    let y = (region.y * f64::from(height)).floor().max(0.0) as u32;
    let w = ((region.width * f64::from(width)).ceil() as u32).min(width.saturating_sub(x));
    let h = ((region.height * f64::from(height)).ceil() as u32).min(height.saturating_sub(y));
    (w >= 16 && h >= 16).then_some(kotone_tools::image::CropRect {
        x,
        y,
        width: w,
        height: h,
    })
}

/// 截下来的前台窗口（还没识别）。语音回合在玩家开口时先截一张（[`crate::speech_screen`]），
/// 用到时再识别。
pub struct Frame {
    pub window: kotone_core::tools::WindowInfo,
    pub image: kotone_core::tools::CapturedImage,
    /// 截图的时间（毫秒）与耗时。
    pub captured_at_ms: u64,
    pub capture_ms: u64,
}

/// 截前台窗口（阻塞，放在阻塞线程里调用）。
pub fn capture_frame() -> Result<Frame, String> {
    let started = Instant::now();
    let window = WindowsWindowObserver
        .foreground_window()
        .map_err(|e| format!("找不到前台窗口：{e}"))?
        .ok_or("当前没有前台窗口")?;
    // 跨屏只截主要那块屏上的部分、超过上限等比缩小（玩家两块 4K 屏、窗口横跨两屏时
    // 以前直接报“图像像素数超过上限”）；都写进 image.warnings。只用来识别文字，不换算屏幕坐标。
    let image = GdiForegroundCapturer
        .capture_foreground_view(&window.identity, MAX_CAPTURE_PIXELS)
        .map_err(|e| format!("截图失败：{e}"))?
        .image;
    Ok(Frame {
        window,
        image,
        captured_at_ms: now_ms(),
        capture_ms: started.elapsed().as_millis() as u64,
    })
}

/// 当前的文字识别后端能不能用；不能用时给出原因。
pub(crate) fn ready_ocr_backend(
    app: &AppHandle,
    settings: &Settings,
) -> Result<(String, Arc<dyn OcrBackend>), String> {
    let (ocr_model, backend) = ocr_backend(app, settings)?;
    let info = backend.info();
    if !info.ready {
        return Err(format!(
            "文字识别模型 {ocr_model} 还不能用：{}",
            info.reason.unwrap_or_else(|| info.state.clone())
        ));
    }
    Ok((ocr_model, backend))
}

/// 截前台窗口、识别、写入跟踪。`request` 只是带进结果的玩家原话。
pub async fn read_chat(
    app: &AppHandle,
    request: &str,
    source: Source,
) -> Result<ChatSnapshot, String> {
    let s = settings(app);
    ready_ocr_backend(app, &s)?;
    let frame = tauri::async_runtime::spawn_blocking(capture_frame)
        .await
        .map_err(|e| format!("截图线程异常：{e}"))??;
    read_frame(app, request, source, frame).await
}

/// 识别一张已经截好的前台窗口、写入聊天跟踪。
pub async fn read_frame(
    app: &AppHandle,
    request: &str,
    source: Source,
    frame: Frame,
) -> Result<ChatSnapshot, String> {
    let s = settings(app);
    let (ocr_model, backend) = ready_ocr_backend(app, &s)?;
    let info = backend.info();
    let profile = active_profile(&s);
    let Frame {
        window,
        image,
        captured_at_ms,
        capture_ms,
    } = frame;
    let mut phases = vec![("captureMs", capture_ms)];
    let mut warnings: Vec<String> = Vec::new();
    if image
        .warnings
        .iter()
        .any(|w| w.code == kotone_platform_windows::window_tools::WARNING_UNIFORM_PIXELS)
    {
        return Err(
            "截到的是纯色画面，游戏可能是独占全屏：请在游戏里把显示模式改成无边框窗口后再试".into(),
        );
    }
    // 跨屏只截了一部分、缩小过：告诉模型（识别结果可能不全、小字可能读不准）。
    warnings.extend(
        image
            .warnings
            .iter()
            .filter(|w| {
                matches!(
                    w.code.as_str(),
                    kotone_platform_windows::window_tools::WARNING_PARTIAL_VIEW
                        | kotone_platform_windows::window_tools::WARNING_DOWNSCALED
                )
            })
            .map(|w| w.message.clone()),
    );
    let full = kotone_tools::image::RgbaImage::new(image.width, image.height, image.rgba)
        .map_err(|e| e.to_string())?;
    let crop = profile
        .chat_region
        .as_ref()
        .and_then(|r| region_px(r, full.width(), full.height()));
    let target = match crop {
        Some(rect) => full.crop(rect),
        None => full.clone(),
    };
    let offset = crop
        .map(|r| (f64::from(r.x), f64::from(r.y)))
        .unwrap_or((0.0, 0.0));
    let online = backend.kind() == OcrBackendKind::Online;
    let ocr_started = Instant::now();
    let language = info.default_language.clone();
    let ocr_backend = backend.clone();
    let output = tauri::async_runtime::spawn_blocking(move || {
        let png = if online {
            kotone_tools::image::encode_png(&target).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        ocr_backend
            .recognize(
                &OcrInput {
                    width: target.width(),
                    height: target.height(),
                    rgba: target.pixels(),
                    png: &png,
                },
                &OcrParams {
                    language,
                    options: Value::Null,
                },
                &OcrCancel::new(),
                Instant::now() + OCR_DEADLINE,
            )
            .map_err(|e| format!("文字识别失败：{e}"))
    })
    .await
    .map_err(|e| format!("文字识别线程异常：{e}"))??;
    phases.push(("ocrMs", ocr_started.elapsed().as_millis() as u64));
    warnings.extend(output.warnings.iter().map(|w| w.message.clone()));
    let lines: Vec<OcrLine> = output
        .observations
        .iter()
        .map(|o| OcrLine {
            text: o.text.clone(),
            bbox: o
                .bbox
                .map(|b| [b.x + offset.0, b.y + offset.1, b.width, b.height]),
            confidence: o.confidence,
        })
        .collect();
    let ocr_region = crop.map(|r| {
        [
            f64::from(r.x),
            f64::from(r.y),
            f64::from(r.width),
            f64::from(r.height),
        ]
    });
    // 聊天跟踪：每个窗口一个会话，记住已经见过的消息，标出这次新出现的（按截图的时间算）。
    let now = captured_at_ms;
    let frame_id = kotone_tools::ids::new_id("cfrm");
    let session = format!(
        "companion:{}",
        window
            .process_name
            .clone()
            .unwrap_or_else(|| window.title.clone())
    );
    let roi = ocr_region
        .map(|r| Region {
            x: r[0],
            y: r[1],
            width: r[2],
            height: r[3],
        })
        .unwrap_or(Region {
            x: 0.0,
            y: 0.0,
            width: f64::from(full.width()),
            height: f64::from(full.height()),
        });
    let ingest = IngestInput {
        session,
        expected_generation: None,
        frame_id: frame_id.clone(),
        window_id: format!("{}:{}", window.identity.handle, window.identity.pid),
        captured_at_unix_ms: now,
        roi,
        frame_size: (full.width(), full.height()),
        source: SourceTag {
            kind: "ocr".into(),
            producer: backend.id().to_string(),
            ocr_result_id: None,
            artifact_id: frame_id.clone(),
        },
        lines: lines
            .iter()
            .take(TrackLimits::default().max_lines)
            .map(|l| IngestLine {
                text: l
                    .text
                    .chars()
                    .take(TrackLimits::default().max_text_chars)
                    .collect(),
                root_box: l.bbox.map(|b| Region {
                    x: b[0],
                    y: b[1],
                    width: b[2],
                    height: b[3],
                }),
                confidence: l.confidence,
                block_box: None,
            })
            .collect(),
        on_source_change: OnSourceChange::NewGeneration,
    };
    let content_hash = kotone_tools::image::sha256_hex(
        lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
    );
    let tracked =
        match app
            .state::<CompanionRuntime>()
            .tracker
            .ingest(ingest, content_hash, now, || Ok(()))
        {
            Ok(result) => result["visible"]
                .as_array()
                .map(|visible| {
                    visible
                        .iter()
                        .map(|m| planner::TrackedMessage {
                            id: m["messageId"].as_str().unwrap_or("").to_string(),
                            text: m["text"].as_str().unwrap_or("").to_string(),
                            new: m["firstSeenFrameId"].as_str() == Some(frame_id.as_str()),
                            first_seen_ago_ms: now
                                .saturating_sub(m["firstSeenAtUnixMs"].as_u64().unwrap_or(now)),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(error) => {
                warnings.push(format!("聊天跟踪未更新：{error:?}"));
                Vec::new()
            }
        };
    Ok(ChatSnapshot {
        input: CompanionInput {
            request: request.to_string(),
            source: source.as_str().into(),
            window_title: window.title.clone(),
            process_name: window.process_name.clone(),
            image_size: (full.width(), full.height()),
            ocr_region,
            ocr_model,
            ocr_lines: lines,
            tracked,
            profile_name: profile.display_name.clone(),
            profile_notes: profile.companion_notes.clone(),
            default_delivery: s.assistant.default_delivery.clone(),
            auto_send: s.assistant.companion.auto_send,
        },
        window_size: (full.width(), full.height()),
        phases,
        ocr_backend: backend.id().to_string(),
        data_egress: info.data_egress,
        warnings,
    })
}

// ---------- 执行 ----------

/// 把文字打进前台游戏的聊天框：沿用当前 profile 的开聊天框键、延迟与注入方式；
/// `send = false` 时不按发送键。
pub async fn type_text(app: &AppHandle, text: &str, send: bool) -> Result<Value, String> {
    let text = text.trim().to_string();
    if text.is_empty() || text.chars().count() > 200 || text.contains(['\r', '\n']) {
        return Err("要打的字为空、超过 200 字或含有换行".into());
    }
    let s = settings(app);
    let mut profile = active_profile(&s);
    if !send {
        profile.send_key.clear();
    }
    let injector = app.state::<SharedState>().injector.clone();
    let profile_id = profile.id.clone();
    let typed = text.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        injector.send(&typed, &profile, CancelToken::default())
    });
    match tokio::time::timeout(TYPE_TIMEOUT, task).await {
        Ok(Ok(Ok(()))) => Ok(json!({"typed": text, "sent": send, "profile": profile_id,
            "receipt": if send { "已按 profile 的发送键发送；只证明本机按键已执行，不证明服务器已收到" } else { "已打进聊天框，未按发送键" }})),
        Ok(Ok(Err(error))) => Err(format!("打字失败：{}", error.message)),
        Ok(Err(_)) => Err("打字线程异常".into()),
        Err(_) => Err("打字超时（10 秒）：模拟输入可能被安全软件或游戏拦截".into()),
    }
}

/// 把模型回报的聊天框位置换算成比例记进当前 profile（与已记录的差别很小时不写）。
fn learn_chat_region(
    settings: &Settings,
    region: [f64; 4],
    size: (u32, u32),
) -> Option<ChatRegion> {
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    if w < 1.0 || h < 1.0 {
        return None;
    }
    // 留一点边距，避免下一条消息的行刚好落在边界外。
    let pad_x = region[2] * 0.05;
    let pad_y = region[3] * 0.1;
    let x0 = ((region[0] - pad_x) / w).clamp(0.0, 1.0);
    let y0 = ((region[1] - pad_y) / h).clamp(0.0, 1.0);
    let x1 = ((region[0] + region[2] + pad_x) / w).clamp(0.0, 1.0);
    let y1 = ((region[1] + region[3] + pad_y) / h).clamp(0.0, 1.0);
    let next = ChatRegion {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    };
    next.validate().ok()?;
    let mut profile = active_profile(settings);
    if let Some(old) = profile.chat_region {
        let close = (old.x - next.x).abs() < 0.02
            && (old.y - next.y).abs() < 0.02
            && (old.width - next.width).abs() < 0.02
            && (old.height - next.height).abs() < 0.02;
        if close {
            return None;
        }
    }
    profile.chat_region = Some(next);
    match profile::save(&profile) {
        Ok(()) => Some(next),
        Err(error) => {
            kotone_core::log::log(&format!("companion: 记录聊天框位置失败：{error}"));
            None
        }
    }
}

/// 一次完整陪玩的结果。
pub struct Outcome {
    pub action: CompanionAction,
    pub message: String,
    pub snapshot: ChatSnapshot,
    pub learned_region: Option<ChatRegion>,
    pub plan_ms: u64,
}

impl Outcome {
    fn summary(&self) -> String {
        match &self.action {
            CompanionAction::Reminders(r) => format!("已设置 {} 个提醒", r.len()),
            CompanionAction::Type { text } => format!("已打进聊天框：{text}"),
            CompanionAction::Answer { text, .. } => text.clone(),
            CompanionAction::Clarify { message } => message.clone(),
        }
    }
}

/// 截图、识别、规划（不执行动作）。
async fn plan(
    app: &AppHandle,
    text: &str,
    source: Source,
    cancel: &CancellationToken,
) -> Result<Outcome, String> {
    let s = settings(app);
    let connection = connection_ready(app, &s)?;
    let snapshot = read_chat(app, text, source).await?;
    let started = Instant::now();
    let plan = planner::plan(&connection, &snapshot.input, cancel).await?;
    let plan_ms = started.elapsed().as_millis() as u64;
    let learned_region = plan
        .chat_region
        .and_then(|region| learn_chat_region(&s, region, snapshot.window_size));
    let mut outcome = Outcome {
        action: plan.action,
        message: String::new(),
        snapshot,
        learned_region,
        plan_ms,
    };
    outcome.message = outcome.summary();
    Ok(outcome)
}

/// 告诉用户一段话（失败提示、追问）：用助手的默认方式，后台进行，不阻塞调用方。
fn tell(app: &AppHandle, text: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::assistant::deliver_text(&app, text, Vec::new()).await {
            kotone_core::log::log(&format!("companion: 提示未能播出：{error}"));
        }
    });
}

/// 是否有一条陪玩请求正在处理（工具入口用来提前给出“忙”的答复）。
pub fn is_busy(app: &AppHandle) -> bool {
    app.state::<CompanionRuntime>().busy.try_lock().is_err()
}

fn acquire(app: &AppHandle) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    app.state::<CompanionRuntime>()
        .busy
        .clone()
        .try_lock_owned()
        .map_err(|_| "上一条陪玩请求还在处理，请稍等".to_string())
}

/// 陪玩热键（编排器识别完语音后）：规划并执行打字 / 回答；提醒交回编排器提交。
pub async fn handle_assistant(
    app: &AppHandle,
    text: String,
    source: Source,
    cancel: CancellationToken,
) -> Result<AssistantPlan, String> {
    let result = async {
        let _busy = acquire(app)?;
        let outcome = plan(app, &text, source, &cancel).await?;
        if cancel.is_cancelled() {
            return Err("陪玩请求已取消".to_string());
        }
        log_outcome(&outcome);
        match &outcome.action {
            CompanionAction::Reminders(reminders) => Ok(AssistantPlan {
                message: outcome.message.clone(),
                reminders: reminders.clone(),
            }),
            CompanionAction::Type { text } => {
                let auto_send = settings(app).assistant.companion.auto_send;
                type_text(app, text, auto_send).await?;
                Ok(AssistantPlan {
                    message: outcome.message.clone(),
                    reminders: vec![],
                })
            }
            CompanionAction::Answer { text, delivery } => {
                let (app2, text2, delivery2) = (app.clone(), text.clone(), delivery.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        crate::assistant::deliver_text(&app2, text2, delivery2).await
                    {
                        kotone_core::log::log(&format!("companion: 回答未能播出：{error}"));
                    }
                });
                Ok(AssistantPlan {
                    message: outcome.message.clone(),
                    reminders: vec![],
                })
            }
            CompanionAction::Clarify { message } => {
                tell(app, message.clone());
                Ok(AssistantPlan {
                    message: message.clone(),
                    reminders: vec![],
                })
            }
        }
    }
    .await;
    if let Err(error) = &result {
        if !cancel.is_cancelled() {
            tell(app, error.clone());
        }
    }
    result
}

fn log_outcome(outcome: &Outcome) {
    // 只记录动作类型、行数与耗时，不记录聊天内容或回答原文。
    kotone_core::log::log(&format!(
        "companion: action={} lines={} ocr={} timing={:?} plan={}ms region_learned={}",
        outcome.action.kind(),
        outcome.snapshot.input.ocr_lines.len(),
        outcome.snapshot.input.ocr_model,
        outcome.snapshot.phases,
        outcome.plan_ms,
        outcome.learned_region.is_some()
    ));
}

/// 完整执行一次（快捷消息热键与工具 `companion.ask`）：提醒自己提交、打字、回答都等到完成，
/// 返回如实的回执。
pub async fn run(app: &AppHandle, text: String, source: Source) -> Result<Value, String> {
    let _busy = acquire(app)?;
    let cancel = CancellationToken::default();
    let outcome = plan(app, &text, source, &cancel).await?;
    log_outcome(&outcome);
    let delivery = match &outcome.action {
        CompanionAction::Reminders(reminders) => {
            let scheduler = app
                .state::<crate::assistant::AssistantRuntime>()
                .scheduler
                .as_ref()
                .map_err(Clone::clone)?
                .clone();
            let session = format!(
                "companion-{}-{}",
                source.as_str(),
                kotone_core::eval::new_session_id()
            );
            let scheduled = scheduler.schedule_batch(&session, now_ms(), reminders)?;
            json!({"scheduled": scheduled})
        }
        CompanionAction::Type { text } => {
            let auto_send = settings(app).assistant.companion.auto_send;
            type_text(app, text, auto_send).await?
        }
        CompanionAction::Answer { text, delivery } => {
            let used = crate::assistant::deliver_text(app, text.clone(), delivery.clone()).await?;
            json!({"delivered": used})
        }
        CompanionAction::Clarify { message } => {
            let used = crate::assistant::deliver_text(app, message.clone(), Vec::new()).await?;
            json!({"delivered": used})
        }
    };
    let (text_out, reminders): (String, Vec<ReminderIntent>) = match &outcome.action {
        CompanionAction::Reminders(r) => (outcome.message.clone(), r.clone()),
        CompanionAction::Type { text } | CompanionAction::Answer { text, .. } => {
            (text.clone(), vec![])
        }
        CompanionAction::Clarify { message } => (message.clone(), vec![]),
    };
    let mut timing: serde_json::Map<String, Value> = outcome
        .snapshot
        .phases
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    timing.insert("planMs".into(), json!(outcome.plan_ms));
    Ok(json!({
        "action": outcome.action.kind(),
        "text": text_out,
        "reminders": reminders.iter().map(|r| json!({"delaySeconds": r.delay_seconds, "text": r.text, "delivery": r.delivery})).collect::<Vec<_>>(),
        "result": delivery,
        "ocr": {"model": outcome.snapshot.input.ocr_model, "backend": outcome.snapshot.ocr_backend, "dataEgress": outcome.snapshot.data_egress, "lines": outcome.snapshot.input.ocr_lines.len(), "region": outcome.snapshot.input.ocr_region},
        "chatRegionLearned": outcome.learned_region,
        "warnings": outcome.snapshot.warnings,
        "timing": timing,
    }))
}

/// 快捷指令热键：交给 AI 的一句话（等于替用户说了这句话，不经过语音识别），或直接运行一个
/// 已保存的自动化（目标是按下热键时的前台窗口，不调用 AI）。失败如实播出。
pub fn quick(app: &AppHandle, slot: usize) {
    let s = settings(app);
    let Some(message) = s.assistant.companion.quick_messages.get(slot).cloned() else {
        return;
    };
    if message.runs_automation() {
        let result = crate::automation::run_saved(
            app,
            "quick-command",
            message.automation_id.trim(),
            &message.args,
            kotone_core::automation::now_ms(),
        );
        if let Err(error) = result {
            kotone_core::log::log(&format!(
                "companion: quick command {} failed: {error}",
                slot + 1
            ));
            tell(app, format!("快捷指令没能运行：{error}"));
        }
        return;
    }
    if s.agent.in_game && s.assistant.route != kotone_core::assistant::AssistantRoute::Agent {
        // “一句话”交给琴音 Agent（ADR 014 §4）；结果在后台播出。
        if let Err(error) =
            crate::agent::submit_from_assistant(app, message.text.clone(), "quick-command")
        {
            tell(app, format!("快捷指令没能交出去：{error}"));
        }
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run(&app, message.text.clone(), Source::QuickMessage).await {
            kotone_core::log::log(&format!(
                "companion: quick message {} failed: {error}",
                slot + 1
            ));
            tell(&app, error);
        }
    });
}

/// 按 ID 找快捷消息的内容（工具 `companion.ask {quickMessageId}`）。
pub fn quick_message_text(app: &AppHandle, id: &str) -> Option<String> {
    settings(app)
        .assistant
        .companion
        .quick_messages
        .into_iter()
        .find(|m| m.id == id && !m.runs_automation())
        .map(|m| m.text)
}

// ---------- 测试连接 ----------

/// 测试琴音当前用的连接能不能连通（一次付费请求），并把结果写进设置（只用于显示）。
pub async fn self_check(app: &AppHandle) -> Result<CompanionCheck, String> {
    let id = settings(app).companion_connection_id().trim().to_string();
    if id.is_empty() {
        return Err("还没有选 API 连接：请在「设置 → 她怎么回答 → 她用的模型」里选一条".into());
    }
    check_connection(app, &id).await
}

/// 测试任意一条连接能不能连通（一次付费请求）。只有它正是琴音在用的连接时才写进设置；
/// 其他连接的结果只返回给界面，不改配置。
pub async fn check_connection(app: &AppHandle, id: &str) -> Result<CompanionCheck, String> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("缺少连接 ID".into());
    }
    let connection = app.state::<SharedState>().connections.resolve(&id)?;
    if connection
        .api_key
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err("这条连接还没有保存 Key".into());
    }
    let result = planner::self_check(&connection, &CancellationToken::default()).await?;
    let check = CompanionCheck {
        connection_id: id,
        base_url: connection.base_url.clone(),
        model: connection.model.clone(),
        passed: result.passed,
        checked_at_ms: now_ms(),
        message: result.message,
    };
    if settings(app).companion_connection_id().trim() != check.connection_id {
        return Ok(check);
    }
    let saved = check.clone();
    app.state::<SharedState>()
        .settings_repository
        .update(move |next| {
            // 自检期间用户可能换了琴音的连接：只在仍是同一条时写入。
            if next.companion_connection_id().trim() == saved.connection_id {
                next.assistant.companion.check = Some(saved);
            }
            Ok(())
        })?;
    crate::tool_config::notify_settings_changed(app);
    Ok(check)
}

#[tauri::command]
pub async fn companion_self_check(app: AppHandle) -> Result<CompanionCheck, String> {
    self_check(&app).await
}

/// 设置页「API 连接」：单独测某一条连接，不必先设为琴音的连接。
#[tauri::command]
pub async fn connection_self_check(app: AppHandle, id: String) -> Result<CompanionCheck, String> {
    check_connection(&app, &id).await
}

/// 读这条连接在服务商那里的模型列表（不花 token；给模型菜单用）。
/// `supported: false` = 服务商没有列表接口，界面退回预置的模型名。
#[tauri::command]
pub async fn connection_models(app: AppHandle, id: String) -> Result<Value, String> {
    let connection = app.state::<SharedState>().connections.resolve(id.trim())?;
    match kotone_ai::model_listing::list_models(&connection, &CancellationToken::default()).await? {
        kotone_ai::model_listing::Listing::Models(models) => {
            Ok(json!({"supported": true, "models": models}))
        }
        kotone_ai::model_listing::Listing::Unsupported => {
            Ok(json!({"supported": false, "models": []}))
        }
    }
}

#[tauri::command]
pub fn companion_status(app: AppHandle) -> Value {
    status(&app)
}

#[tauri::command]
pub async fn companion_clear_chat_region(app: AppHandle) -> Result<(), String> {
    let s = settings(&app);
    let mut profile = active_profile(&s);
    profile.chat_region = None;
    profile::save(&profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_region_maps_to_pixels_within_the_image() {
        let rect = region_px(
            &ChatRegion {
                x: 0.0,
                y: 0.7,
                width: 0.3,
                height: 0.3,
            },
            1920,
            1080,
        )
        .unwrap();
        assert_eq!((rect.x, rect.y), (0, 756));
        assert!(rect.x + rect.width <= 1920 && rect.y + rect.height <= 1080);
        assert!(region_px(
            &ChatRegion {
                x: 0.9,
                y: 0.9,
                width: 0.5,
                height: 0.5
            },
            1920,
            1080
        )
        .is_none());
    }
}
