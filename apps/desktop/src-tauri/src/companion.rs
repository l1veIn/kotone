//! 她看游戏聊天、她用的连接（`companion` 就是她：设置里的 `assistant.companion`、发到游戏的 `Speaker::Companion`）。
//!
//! - 就绪检查：她用的 API 连接（[`connection_ready`]）、文字识别模型（[`ocr_backend`]），设置页和工具
//!   `companion.status` 看 [`status`]；
//! - 读聊天（工具 `companion.read_chat`、语音回合开口时的截图 [`crate::speech_screen`]）：截前台窗口（GDI 前台
//!   可见客户区；profile 记有聊天框区域时只识别那块）→ 选定的 OCR 模型识别、每行带截图坐标 → 写进聊天跟踪，
//!   标出这次新出现的消息。只读，不调模型；
//! - 快捷指令里的“一句话”（[`say`]）交给她，失败的提示（[`tell`]）用默认方式播出；
//! - 测试连接（[`self_check`] / [`check_connection`]）。
//!
//! 以前这里还有一条旧的单次陪玩流程（截图 + 识别 + 一次大模型调用，从提醒 / 帮忙打字 / 回答里选一种，
//! 工具 `companion.ask` / `companion.type`）；呼叫早就都交给她了，ADR 053 把它退掉。

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kotone_core::assistant::CompanionCheck;
use kotone_core::cancel::CancellationToken;
use kotone_core::ocr::{OcrBackend, OcrBackendKind, OcrCancel, OcrInput, OcrParams};
use kotone_core::profile::{self, ChatRegion, GameProfile};
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

/// 读聊天的跟踪（每个窗口一个会话，记住已经见过的消息）。
pub struct CompanionRuntime {
    tracker: Tracker,
}

impl Default for CompanionRuntime {
    fn default() -> Self {
        Self {
            tracker: Tracker::new(TrackLimits::default()),
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
        return Err("还没有选 API 连接：请在聊天页输入框旁的「她用的模型」里选一个（还没有连接就先在「设置 → API 连接」里添加）".into());
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
        // 快捷指令在热键表里（ADR 040 第 3 期）：全部“说一句 / 运行自动化”，形状和以前一样
        "quickMessages": kotone_core::hotkey_custom::quick_messages(&s.hotkeys),
        "profile": {"id": active_profile(&s).id, "notes": active_profile(&s).companion_notes, "chatRegion": active_profile(&s).chat_region},
    })
}

// ---------- 读聊天 ----------

/// OCR 识别出的一行（截图像素坐标；没有几何时为 None）。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub bbox: Option<[f64; 4]>,
    pub confidence: Option<f32>,
}

/// 聊天跟踪里的一条消息。
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedMessage {
    pub id: String,
    pub text: String,
    /// 这次读才第一次看到。
    pub new: bool,
    /// 第一次看到距今多少毫秒。
    pub first_seen_ago_ms: u64,
}

/// 截图 + OCR + 跟踪的结果（工具 `companion.read_chat` 的返回，语音回合开口时的截图也用它）。
pub struct ChatSnapshot {
    pub window_title: String,
    pub process_name: Option<String>,
    /// 整张截图尺寸（客户区物理像素）。
    pub image_size: (u32, u32),
    /// 本次实际识别的区域（截图像素）；None = 整张。
    pub ocr_region: Option<[f64; 4]>,
    pub ocr_model: String,
    pub ocr_lines: Vec<OcrLine>,
    pub tracked: Vec<TrackedMessage>,
    pub profile_name: String,
    pub profile_notes: String,
    pub phases: Vec<(&'static str, u64)>,
    pub ocr_backend: String,
    pub data_egress: String,
    pub warnings: Vec<String>,
}

impl ChatSnapshot {
    pub fn to_json(&self) -> Value {
        json!({
            "window": {"title": self.window_title, "process": self.process_name, "width": self.image_size.0, "height": self.image_size.1},
            "ocr": {
                "model": self.ocr_model,
                "backend": self.ocr_backend,
                "dataEgress": self.data_egress,
                "region": self.ocr_region,
                "lines": self.ocr_lines.iter().map(|l| json!({"text": l.text, "box": l.bbox, "confidence": l.confidence})).collect::<Vec<_>>(),
            },
            "chatTracking": self.tracked.iter().map(|m| json!({"messageId": m.id, "text": m.text, "newThisRead": m.new, "firstSeenMsAgo": m.first_seen_ago_ms})).collect::<Vec<_>>(),
            "profile": {"name": self.profile_name, "notes": self.profile_notes},
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

/// 截前台窗口、识别、写入跟踪。
pub async fn read_chat(app: &AppHandle) -> Result<ChatSnapshot, String> {
    let s = settings(app);
    ready_ocr_backend(app, &s)?;
    let frame = tauri::async_runtime::spawn_blocking(capture_frame)
        .await
        .map_err(|e| format!("截图线程异常：{e}"))??;
    read_frame(app, frame).await
}

/// 识别一张已经截好的前台窗口、写入聊天跟踪。
pub async fn read_frame(app: &AppHandle, frame: Frame) -> Result<ChatSnapshot, String> {
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
                        .map(|m| TrackedMessage {
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
        window_title: window.title.clone(),
        process_name: window.process_name.clone(),
        image_size: (full.width(), full.height()),
        ocr_region,
        ocr_model,
        ocr_lines: lines,
        tracked,
        profile_name: profile.display_name.clone(),
        profile_notes: profile.companion_notes.clone(),
        phases,
        ocr_backend: backend.id().to_string(),
        data_egress: info.data_egress,
        warnings,
    })
}

// ---------- 快捷指令、提示 ----------

/// 告诉用户一段话（失败提示）：用默认方式，后台进行，不阻塞调用方。
pub(crate) fn tell(app: &AppHandle, text: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::assistant::deliver_text(&app, text, Vec::new()).await {
            kotone_core::log::log(&format!("companion: 提示未能播出：{error}"));
        }
    });
}

/// 快捷指令（自己加的热键“跟她说一句”，ADR 040 第 3 期）：等于替玩家说了这句话（不经过语音识别），交给她；
/// 按下热键的那一路在 [`crate::hotkey_run`]。`assistant.route = agent`（转交外部 Agent）时和呼叫一样原文转交。
/// 失败如实播出。
pub fn say(app: &AppHandle, text: String) {
    let route = settings(app).assistant.route;
    if route == kotone_core::assistant::AssistantRoute::Agent {
        if let Err(error) = crate::tool_service::submit_user_request(app, &text, "assistant-text") {
            tell(app, format!("快捷指令没能转交 Agent：{error}"));
        }
        return;
    }
    // “一句话”交给她（ADR 014 §4）；结果在后台播出。
    if let Err(error) = crate::agent::submit_from_assistant(app, text, "quick-command") {
        tell(app, format!("快捷指令没能交出去：{error}"));
        return;
    }
    // 快捷指令 = 替你说一句：选了通话就接通，她答完你可以直接接着说（ADR 033、术语表“通话”）；
    // 说一句就走、录音笔不接通，她答完照常安静（问了一句才再听）
    crate::call::start(app);
}

// ---------- 测试连接 ----------

/// 测试琴音当前用的连接能不能连通（一次付费请求），并把结果写进设置（只用于显示）。
pub async fn self_check(app: &AppHandle) -> Result<CompanionCheck, String> {
    let id = settings(app).companion_connection_id().trim().to_string();
    if id.is_empty() {
        return Err("还没有选 API 连接：请在聊天页输入框旁的「她用的模型」里选一个".into());
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
    let result =
        kotone_ai::connection_check::self_check(&connection, &CancellationToken::default()).await?;
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
