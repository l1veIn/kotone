//! M2 端到端：真实 loopback HTTP 服务 + 真实客户端，覆盖图像预处理、OCR 后端（假后端与
//! 对接本地 mock 的真实 DeepSeek 后端）、聊天跟踪，以及期限/名额/关闭语义。
//! 不触网、不读取用户配置或凭据、不加载真实模型。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use kotone_core::connection::{ConnectionKind, ResolvedConnection};
use kotone_core::ocr::{
    OcrBackend, OcrBackendKind, OcrError, OcrGranularity, OcrObservation, OcrRect,
};

use crate::protocol::{Envelope, ToolCallRequest};
use crate::testing::{
    decode_png, ocr_line, ocr_output, pattern_pixel, validate_schema, FakeDesktop, FakeHost,
    FakeOcr, ManualClock,
};
use crate::{RunningToolService, ServiceDeps, ServiceLimits, ToolClient};

const GAME: u64 = 0x1001;
const OTHER: u64 = 0x2002;

struct H {
    running: RunningToolService,
    desktop: Arc<FakeDesktop>,
    clock: Arc<ManualClock>,
    client: ToolClient,
    _dir: tempfile::TempDir,
}

impl H {
    async fn new(backends: Vec<Arc<dyn OcrBackend>>, limits: ServiceLimits) -> Self {
        let desktop = FakeDesktop::new();
        // 客户区位于屏幕 (100, 50)，64x48 物理像素
        desktop.add_window(
            GAME,
            4242,
            "League of Legends (TM) Client",
            "League of Legends.exe",
            100,
            50,
            64,
            48,
        );
        desktop.add_window(OTHER, 777, "Notes", "notepad.exe", 0, 0, 64, 48);
        desktop.set_foreground(Some(GAME));
        let clock = ManualClock::at(1_700_000_000_000);
        let host = FakeHost::new(Some(9));
        let dir = tempfile::tempdir().unwrap();
        let deps = ServiceDeps {
            instance_id: crate::ids::new_id("inst"),
            app_version: "0.0.0-m2".into(),
            observer: desktop.clone(),
            capturer: desktop.clone(),
            asr_phase: Arc::new(|| "stopped".into()),
            limits,
            clock: clock.clone(),
            ocr_backends: backends,
        };
        let running = RunningToolService::start(deps, host.clone(), dir.path())
            .await
            .unwrap();
        let client = ToolClient::discover(dir.path(), host.as_ref()).unwrap();
        Self {
            running,
            desktop,
            clock,
            client,
            _dir: dir,
        }
    }

    async fn call(&self, tool: &str, arguments: Value, timeout_ms: Option<u64>) -> Envelope {
        self.client
            .call(&ToolCallRequest {
                request_id: None,
                tool: tool.into(),
                arguments,
                timeout_ms,
            })
            .await
            .unwrap()
    }

    async fn ok(&self, tool: &str, arguments: Value) -> Value {
        let env = self.call(tool, arguments, None).await;
        assert!(env.ok, "{tool} failed: {:?}", env.error);
        let result = env.result.unwrap();
        validate_schema(&self.output_schema(tool), &result, tool)
            .unwrap_or_else(|e| panic!("{tool} schema: {e}\n{result}"));
        result
    }

    async fn err(&self, tool: &str, arguments: Value) -> crate::protocol::ToolError {
        let env = self.call(tool, arguments, None).await;
        assert!(!env.ok, "{tool} unexpectedly succeeded: {:?}", env.result);
        env.error.unwrap()
    }

    fn output_schema(&self, tool: &str) -> Value {
        let caps = self.running.service().capabilities();
        caps["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == tool)
            .unwrap()["outputSchema"]
            .clone()
    }

    async fn capture(&self, handle: u64) -> Value {
        let list = self
            .ok("window.list", json!({"includeUntitled": true}))
            .await;
        let id = list["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["pid"] == if handle == GAME { 4242 } else { 777 })
            .unwrap()["windowId"]
            .as_str()
            .unwrap()
            .to_string();
        self.ok("screen.capture", json!({"windowId": id})).await
    }

    async fn png(&self, id: &str) -> (u32, u32, Vec<u8>, Vec<u8>) {
        let got = self.ok("artifact.get", json!({"artifactId": id})).await;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(got["content"]["data"].as_str().unwrap())
            .unwrap();
        let (w, h, px) = decode_png(&bytes);
        (w, h, px, bytes)
    }
}

fn px(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

#[tokio::test(flavor = "multi_thread")]
async fn preprocess_scale_then_crop_maps_exactly_back_to_the_original() {
    let h = H::new(vec![], ServiceLimits::default()).await;
    let cap = h.capture(GAME).await;
    let root = cap["artifactId"].as_str().unwrap();
    let root_hash = cap["sha256"].clone();
    // 放大 3 倍（最近邻，便于逐像素核对）
    let up = h
        .ok("image.preprocess", json!({"artifactId": root, "operations": [{"op": "scale", "factor": 3, "filter": "nearest"}]}))
        .await;
    assert_eq!(up["created"], true);
    assert_eq!(up["kind"], "preprocess");
    assert_eq!(
        (up["width"].as_u64(), up["height"].as_u64()),
        (Some(192), Some(144))
    );
    assert_eq!(up["parentArtifactId"], root);
    assert_eq!(up["rootArtifactId"], root);
    assert_eq!(up["derivation"]["type"], "preprocess");
    assert_eq!(up["preprocess"]["scaleX"], 3.0);
    assert_eq!(
        up["rootTransform"],
        json!({"offsetX": 0, "offsetY": 0, "scaleX": 1.0 / 3.0, "scaleY": 1.0 / 3.0})
    );
    assert_eq!(
        up["rootRect"],
        json!({"x": 0, "y": 0, "width": 64, "height": 48})
    );
    assert_eq!(
        up["source"]["frameId"], cap["source"]["frameId"],
        "整条链共享采集来源"
    );
    // 在放大图上裁剪 (30, 15, 12, 9) → 原图 (10, 5, 4, 3)
    let crop = h
        .ok("image.crop", json!({"artifactId": up["artifactId"], "rect": {"x": 30, "y": 15, "width": 12, "height": 9}}))
        .await;
    assert_eq!(crop["rootTransform"]["offsetX"], 10);
    assert_eq!(crop["rootTransform"]["offsetY"], 5);
    assert_eq!(
        crop["rootRect"],
        json!({"x": 10, "y": 5, "width": 4, "height": 3})
    );
    assert_eq!(
        crop["screenRect"],
        json!({"x": 110, "y": 55, "width": 4, "height": 3})
    );
    assert_eq!(crop["derivation"]["type"], "crop");
    let (w, _, pixels, _) = h.png(crop["artifactId"].as_str().unwrap()).await;
    for y in 0..9 {
        for x in 0..12 {
            assert_eq!(
                px(&pixels, w, x, y),
                pattern_pixel(10 + x / 3, 5 + y / 3),
                "({x},{y})"
            );
        }
    }
    // 再放大 2 倍：多级变换的比例相乘
    let up2 = h
        .ok("image.preprocess", json!({"artifactId": crop["artifactId"], "operations": [{"op": "scale", "factor": 2, "filter": "nearest"}, {"op": "grayscale"}]}))
        .await;
    let t = &up2["rootTransform"];
    assert!((t["scaleX"].as_f64().unwrap() - 1.0 / 6.0).abs() < 1e-12);
    assert_eq!(t["offsetX"], 10);
    assert_eq!(
        up2["rootRect"],
        json!({"x": 10, "y": 5, "width": 4, "height": 3})
    );
    // 非整数比例：1.5 倍线性放大后在 (1,1) 裁剪 → 偏移为小数
    let up15 = h
        .ok("image.preprocess", json!({"artifactId": root, "operations": [{"op": "scale", "factor": 1.5, "filter": "linear"}]}))
        .await;
    assert_eq!(up15["width"], 96);
    let c15 = h
        .ok("image.crop", json!({"artifactId": up15["artifactId"], "rect": {"x": 1, "y": 1, "width": 3, "height": 3}}))
        .await;
    assert_eq!(
        c15["rootRect"],
        json!({"x": 0.667, "y": 0.667, "width": 2, "height": 2})
    );
    // 原图不变
    let again = h
        .ok(
            "artifact.get",
            json!({"artifactId": root, "includeContent": false}),
        )
        .await;
    assert_eq!(again["sha256"], root_hash);
    // preset=none：允许不处理，返回输入产物本身
    let none = h
        .ok(
            "image.preprocess",
            json!({"artifactId": root, "preset": "none"}),
        )
        .await;
    assert_eq!(none["created"], false);
    assert_eq!(none["artifactId"], root);
    let preset = h
        .ok(
            "image.preprocess",
            json!({"artifactId": root, "preset": "upscale-3x-cubic"}),
        )
        .await;
    assert_eq!(
        preset["preprocess"]["operations"],
        json!([{"op": "scale", "factor": 3.0, "filter": "cubic"}])
    );
    // 拒绝：二选一、空列表、越界比例、超像素上限、未知操作、路径字段、未知预设
    for bad in [
        json!({"artifactId": root, "preset": "none", "operations": [{"op": "invert"}]}),
        json!({"artifactId": root}),
        json!({"artifactId": root, "operations": []}),
        json!({"artifactId": root, "operations": [{"op": "scale", "factor": 8}]}),
        json!({"artifactId": root, "operations": [{"op": "blur"}]}),
        json!({"artifactId": root, "operations": [{"op": "invert"}], "path": "C:/x.png"}),
        json!({"artifactId": root, "preset": "magic"}),
    ] {
        assert_eq!(
            h.err("image.preprocess", bad).await.code,
            "INVALID_ARGUMENTS"
        );
    }
    let tiny = ServiceLimits {
        max_image_pixels: 64 * 48 * 2,
        ..Default::default()
    };
    let small = H::new(vec![], tiny).await;
    let cap = small.capture(GAME).await;
    let e = small
        .err(
            "image.preprocess",
            json!({"artifactId": cap["artifactId"], "operations": [{"op": "scale", "factor": 2}]}),
        )
        .await;
    assert_eq!(e.code, "INVALID_ARGUMENTS");
    assert_eq!(
        small
            .err(
                "image.preprocess",
                json!({"artifactId": "art_missing", "preset": "none"})
            )
            .await
            .code,
        "ARTIFACT_NOT_FOUND"
    );
}

fn local(id: &str) -> Arc<FakeOcr> {
    FakeOcr::new(id, OcrBackendKind::Local)
}

#[tokio::test(flavor = "multi_thread")]
async fn ocr_recognize_maps_boxes_through_preprocess_and_keeps_null_confidence() {
    let fake = local("fake-local");
    let online = FakeOcr::new("fake-online", OcrBackendKind::Online);
    online.set_languages(&["auto"]);
    let h = H::new(vec![fake.clone(), online.clone()], ServiceLimits::default()).await;
    let cap = h.capture(GAME).await;
    let up = h
        .ok(
            "image.preprocess",
            json!({"artifactId": cap["artifactId"], "preset": "upscale-3x-cubic"}),
        )
        .await;
    let crop = h
        .ok("image.crop", json!({"artifactId": up["artifactId"], "rect": {"x": 30, "y": 60, "width": 150, "height": 60}}))
        .await;
    // 后端在输入（裁剪后）图像中返回一行带框、无置信度
    fake.push(Ok(ocr_output(vec![
        ocr_line("[팀] 가: 미드 갈게", 6.0, 3.0, 90.0, 12.0, None),
        ocr_line("나: ㅋㅋ", 6.0, 30.0, 45.0, 12.0, Some(0.91)),
    ])));
    let r = h
        .ok(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-local"}),
        )
        .await;
    assert_eq!(r["language"], "ko");
    assert_eq!(r["backend"]["dataEgress"], "none");
    assert_eq!(r["frameId"], cap["source"]["frameId"]);
    assert_eq!(r["text"], "[팀] 가: 미드 갈게\n나: ㅋㅋ");
    let o = &r["observations"][0];
    assert_eq!(o["granularity"], "line");
    assert_eq!(o["geometry"], "box");
    assert!(o["confidence"].is_null(), "没有置信度必须是 null");
    assert_eq!(o["box"], json!({"x": 6, "y": 3, "width": 90, "height": 12}));
    // 根坐标：裁剪偏移 (30,60)/3 = (10,20)，再加 (6,3)/3 = (2,1) → (12, 21)，尺寸 /3
    assert_eq!(
        o["rootBox"],
        json!({"x": 12, "y": 21, "width": 30, "height": 4})
    );
    assert_eq!(
        r["observations"][1]["confidence"]
            .as_f64()
            .map(|c| (c * 100.0).round()),
        Some(91.0)
    );
    assert!(r["ocrResultId"].as_str().unwrap().starts_with("ocr_"));
    assert!(r["confidenceAvailable"].as_bool().unwrap());
    // block（整块框 + 行，无逐行几何）与 page（无几何）
    fake.push(Ok(ocr_output(vec![
        OcrObservation {
            text: "줄 하나\n줄 둘".into(),
            granularity: OcrGranularity::Block,
            bbox: Some(OcrRect {
                x: 0.0,
                y: 0.0,
                width: 150.0,
                height: 60.0,
            }),
            polygon: None,
            confidence: None,
            label: Some("text".into()),
            lines: vec!["줄 하나".into(), "줄 둘".into()],
        },
        OcrObservation {
            text: "기하 없음".into(),
            granularity: OcrGranularity::Line,
            bbox: None,
            polygon: None,
            confidence: None,
            label: None,
            lines: vec![],
        },
    ])));
    let r = h
        .ok(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-local"}),
        )
        .await;
    assert_eq!(r["observations"][0]["granularity"], "block");
    assert_eq!(r["observations"][0]["lines"], json!(["줄 하나", "줄 둘"]));
    assert_eq!(
        r["observations"][0]["rootBox"],
        json!({"x": 10, "y": 20, "width": 50, "height": 20})
    );
    assert_eq!(r["observations"][1]["geometry"], "none");
    assert!(
        r["observations"][1]["rootBox"].is_null(),
        "没有框就没有几何，不伪造坐标"
    );
    // 空结果：成功但 empty=true + 警告
    let r = h
        .ok(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-local"}),
        )
        .await;
    assert_eq!(r["empty"], true);
    assert!(r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "OCR_EMPTY_RESULT"));
    // 错误分类；失败不降级到另一个后端
    let before_online = online.calls();
    for (args, code) in [
        (
            json!({"artifactId": crop["artifactId"], "backend": "nope"}),
            "OCR_BACKEND_NOT_FOUND",
        ),
        (
            json!({"artifactId": crop["artifactId"], "backend": "fake-local", "language": "ja"}),
            "OCR_LANGUAGE_UNSUPPORTED",
        ),
        (
            json!({"artifactId": crop["artifactId"], "backend": "fake-local", "options": {"turbo": true}}),
            "OCR_OPTION_UNSUPPORTED",
        ),
        (
            json!({"artifactId": crop["artifactId"], "backend": "fake-local", "path": "C:/a.png"}),
            "INVALID_ARGUMENTS",
        ),
        (
            json!({"artifactId": "C:/Windows/win.ini", "backend": "fake-local"}),
            "ARTIFACT_NOT_FOUND",
        ),
    ] {
        assert_eq!(h.err("ocr.recognize", args).await.code, code);
    }
    fake.push(Err(OcrError::Engine("onnx exploded".into())));
    let e = h
        .err(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-local"}),
        )
        .await;
    assert_eq!(e.code, "OCR_FAILED");
    fake.push(Err(OcrError::Transport {
        message: "reset".into(),
        retryable: true,
    }));
    let e = h
        .err(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-local"}),
        )
        .await;
    assert_eq!(
        (e.code.as_str(), e.retryable),
        ("OCR_TRANSPORT_ERROR", true)
    );
    fake.set_ready(false);
    let e = h
        .err(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-local"}),
        )
        .await;
    assert_eq!(e.code, "OCR_BACKEND_NOT_READY");
    assert_eq!(online.calls(), before_online, "本地失败从不改走在线后端");
    // 在线后端只接受 auto
    assert_eq!(
        h.err(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-online", "language": "ko"})
        )
        .await
        .code,
        "OCR_LANGUAGE_UNSUPPORTED"
    );
    let r = h
        .ok(
            "ocr.recognize",
            json!({"artifactId": crop["artifactId"], "backend": "fake-online"}),
        )
        .await;
    assert_eq!(r["backend"]["dataEgress"], "selected-image");
    // ocr.backends：就绪、外发声明、无降级
    let b = h.ok("ocr.backends", json!({})).await;
    assert_eq!(b["fallback"], "none");
    let ids: Vec<&str> = b["backends"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["fake-local", "fake-online"]);
    assert_eq!(b["backends"][0]["ready"], false);
    assert_eq!(b["backends"][1]["dataEgress"], "selected-image");
    let caps = h.running.service().capabilities();
    let tool = caps["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "ocr.recognize")
        .unwrap()
        .clone();
    assert_eq!(tool["ready"], true, "在线假后端就绪");
}

fn ds_connection(base: String, key: Option<&str>) -> ResolvedConnection {
    ResolvedConnection {
        id: "ds".into(),
        display_name: "DeepSeek".into(),
        kind: ConnectionKind::Remote,
        provider: "custom".into(),
        base_url: base,
        model: "deepseek-chat".into(),
        api_key: key.map(str::to_string),
    }
}

async fn rich(h: &H, id: &Value) -> Envelope {
    h.call(
        "ocr.recognize",
        json!({"artifactId": id, "backend": "deepseek-ocr", "options": {"purpose": "rich_layout_recognition"}}),
        Some(10_000),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn deepseek_backend_uploads_only_the_selected_png_and_handles_200_errors() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let uri = server.uri();
    let deepseek = Arc::new(kotone_ocr::DeepSeekOcr::new(Arc::new(move || {
        Ok(Some(ds_connection(uri.clone(), Some("sk-mock"))))
    })));
    let h = H::new(vec![deepseek.clone()], ServiceLimits::default()).await;
    let cap = h.capture(GAME).await;
    let crop = h
        .ok("image.crop", json!({"artifactId": cap["artifactId"], "rect": {"x": 0, "y": 0, "width": 50, "height": 20}}))
        .await;
    let (_, _, _, crop_png) = h.png(crop["artifactId"].as_str().unwrap()).await;
    let replies = [
        ResponseTemplate::new(200).set_body_string(format!(
            "\n\n{}",
            json!({"object": "ocr.result", "page_count": 1, "pages": [{"index": 0, "text": "<｜ref｜>text<｜/ref｜><｜box｜>[[0,0,1000,500]]<｜/box｜>[팀] 가: 안녕\n나: 응", "finish_reason": "stop"}], "usage": {"total_tokens": 9}})
        )),
        ResponseTemplate::new(200).set_body_json(json!({"error": {"message": "OCR did not finish in time", "type": "gateway_timeout_error", "param": null, "code": "gateway_timeout_error"}})),
        ResponseTemplate::new(200).set_body_json(json!({"object": "ocr.result", "page_count": 1, "pages": [{"index": 0, "text": "잘린", "finish_reason": "length"}]})),
        ResponseTemplate::new(429).set_body_json(json!({"error": {"message": "too many", "type": "rate_limit_error"}})),
        ResponseTemplate::new(200).set_body_json(json!({"object": "ocr.result", "page_count": 1, "pages": [{"index": 0, "text": "", "finish_reason": "stop"}]})),
    ];
    for reply in replies {
        Mock::given(method("POST"))
            .and(path("/v1/tools/ocr"))
            .respond_with(reply)
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    let crop_id = crop["artifactId"].clone();
    // 1. block 粒度（0–1000 归一化 → 输入像素 → 根坐标）
    let env = rich(&h, &crop_id).await;
    let r = env.result.unwrap();
    validate_schema(&h.output_schema("ocr.recognize"), &r, "ocr.recognize").unwrap();
    assert_eq!(r["backend"]["dataEgress"], "selected-image");
    assert_eq!(r["observations"][0]["granularity"], "block");
    assert_eq!(
        r["observations"][0]["lines"],
        json!(["[팀] 가: 안녕", "나: 응"])
    );
    assert_eq!(
        r["observations"][0]["rootBox"],
        json!({"x": 0, "y": 0, "width": 50, "height": 10})
    );
    assert!(r["observations"][0]["confidence"].is_null());
    assert_eq!(r["backendExtra"]["usage"]["total_tokens"], 9);
    // 2. HTTP 200 内的 error 对象 → 错误，而不是空聊天
    let e = rich(&h, &crop_id).await.error.unwrap();
    assert_eq!((e.code.as_str(), e.retryable), ("OCR_REMOTE_ERROR", false));
    assert_eq!(e.details.as_ref().unwrap()["httpStatus"], 200);
    // 3. 截断
    let r = rich(&h, &crop_id).await.result.unwrap();
    assert_eq!(r["truncated"], true);
    assert!(r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "OCR_TRUNCATED"));
    // 4. 429 可重试
    let e = rich(&h, &crop_id).await.error.unwrap();
    assert_eq!((e.code.as_str(), e.retryable), ("OCR_REMOTE_ERROR", true));
    // 5. 空结果
    let r = rich(&h, &crop_id).await.result.unwrap();
    assert_eq!(r["empty"], true);
    // 上传的正是所选产物的 PNG 字节，附带 Bearer key，只有两个表单字段
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 5);
    for req in &requests {
        assert_eq!(
            req.headers.get("authorization").unwrap().to_str().unwrap(),
            "Bearer sk-mock"
        );
        assert!(req
            .body
            .windows(crop_png.len())
            .any(|w| w == crop_png.as_slice()));
        let text = String::from_utf8_lossy(&req.body);
        assert_eq!(text.matches("Content-Disposition").count(), 2);
        assert!(
            !text.contains("League of Legends"),
            "不上传窗口标题等元数据"
        );
    }
    // 未配置的在线后端：不就绪、不发请求
    let h2 = H::new(
        vec![Arc::new(kotone_ocr::DeepSeekOcr::new(Arc::new(|| {
            Ok(None)
        })))],
        ServiceLimits::default(),
    )
    .await;
    let b = h2.ok("ocr.backends", json!({})).await;
    assert_eq!(b["backends"][0]["state"], "not-configured");
    let cap2 = h2.capture(GAME).await;
    let e = h2
        .err(
            "ocr.recognize",
            json!({"artifactId": cap2["artifactId"], "backend": "deepseek-ocr"}),
        )
        .await;
    assert_eq!(e.code, "OCR_BACKEND_NOT_READY");
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn ocr_timeout_cancels_and_lane_is_held_until_the_worker_really_ends() {
    let coop = local("coop");
    let stubborn = FakeOcr::new("stubborn", OcrBackendKind::Online);
    stubborn.set_languages(&["auto"]);
    let h = H::new(
        vec![coop.clone(), stubborn.clone()],
        ServiceLimits {
            online_ocr_concurrency: 1,
            ..Default::default()
        },
    )
    .await;
    let cap = h.capture(GAME).await;
    let art = cap["artifactId"].clone();
    // 协作后端：期限到达后被取消，结果不发布
    coop.set_delay(Duration::from_millis(2_000), true);
    coop.push(Ok(ocr_output(vec![ocr_line(
        "늦은 결과",
        0.0,
        0.0,
        10.0,
        10.0,
        None,
    )])));
    let env = h
        .call(
            "ocr.recognize",
            json!({"artifactId": art, "backend": "coop"}),
            Some(100),
        )
        .await;
    assert_eq!(env.error.as_ref().unwrap().code, "TIMEOUT");
    for _ in 0..200 {
        if coop.finished() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(coop.cancels(), 1, "超时通过取消令牌通知后端");
    assert_eq!(
        h.running.service().resident_ocr_and_sessions().0,
        0,
        "迟到结果不发布"
    );
    // 不响应取消的后端：超时后工作线程仍在运行，名额不释放
    stubborn.set_delay(Duration::from_millis(600), false);
    let env = h
        .call(
            "ocr.recognize",
            json!({"artifactId": art, "backend": "stubborn"}),
            Some(80),
        )
        .await;
    assert_eq!(env.error.as_ref().unwrap().code, "TIMEOUT");
    let env = h
        .call(
            "ocr.recognize",
            json!({"artifactId": art, "backend": "stubborn"}),
            Some(80),
        )
        .await;
    let e = env.error.unwrap();
    assert_eq!(e.code, "BUSY", "名额直到工作线程真正结束才释放");
    assert_eq!(e.details.unwrap()["lane"], "online");
    assert_eq!(stubborn.calls(), 1, "排队超时的调用从未开始");
    for _ in 0..200 {
        if stubborn.finished() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    stubborn.set_delay(Duration::ZERO, true);
    let env = h
        .call(
            "ocr.recognize",
            json!({"artifactId": art, "backend": "stubborn"}),
            Some(2_000),
        )
        .await;
    assert!(env.ok, "{:?}", env.error);
    assert_eq!(stubborn.max_active(), 1);
    assert_eq!(h.running.service().resident_ocr_and_sessions().0, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_cancels_inflight_ocr_and_rejects_its_result() {
    let coop = local("coop");
    let h = H::new(vec![coop.clone()], ServiceLimits::default()).await;
    let cap = h.capture(GAME).await;
    coop.set_delay(Duration::from_millis(3_000), true);
    let service = h.running.service().clone();
    let art = cap["artifactId"].clone();
    let pending = tokio::spawn(async move {
        service
            .call(ToolCallRequest {
                request_id: None,
                tool: "ocr.recognize".into(),
                arguments: json!({"artifactId": art, "backend": "coop"}),
                timeout_ms: Some(10_000),
            })
            .await
    });
    for _ in 0..200 {
        if coop.calls() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let service = h.running.service().clone();
    let mut running = h.running;
    running.shutdown_now();
    let env = pending.await.unwrap();
    assert_eq!(env.error.unwrap().code, "SERVICE_CLOSED");
    assert_eq!(coop.cancels(), 1);
    assert_eq!(service.resident_ocr_and_sessions(), (0, 0));
}

/// 一次“截图 → OCR → ingest”，返回 ingest 结果。
async fn observe(h: &H, fake: &FakeOcr, lines: &[(&str, f64)]) -> Value {
    h.clock.advance(1_000);
    let cap = h.capture(GAME).await;
    fake.push(Ok(ocr_output(
        lines
            .iter()
            .map(|(t, y)| ocr_line(t, 2.0, *y, 50.0, 6.0, Some(0.9)))
            .collect(),
    )));
    let r = h
        .ok(
            "ocr.recognize",
            json!({"artifactId": cap["artifactId"], "backend": "fake-local"}),
        )
        .await;
    h.ok("chat.track.ingest", json!({"session": "lol-team", "observation": {"type": "ocrResult", "ocrResultId": r["ocrResultId"]}})).await
}

fn ids(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["messageId"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_tracking_end_to_end_over_http() {
    let fake = local("fake-local");
    let h = H::new(vec![fake.clone()], ServiceLimits::default()).await;
    // 帧 1：两条
    let a = observe(
        &h,
        &fake,
        &[("[팀] 가: 미드 갈개", 10.0), ("[전체] 나: 안녕", 20.0)],
    )
    .await;
    assert_eq!(a["generation"], 1);
    assert_eq!(a["visible"][0]["origin"], "first-observation");
    assert_eq!(
        a["visible"][0]["channel"],
        json!({"value": "unknown", "candidates": [{"value": "team", "basis": "text-prefix", "evidence": "[팀]"}]})
    );
    assert_eq!(a["visible"][1]["speaker"]["candidates"][0]["name"], "나");
    let cursor1 = a["cursor"].as_str().unwrap().to_string();
    // 同一 OCR 结果重复提交：幂等
    let cap_ocr = {
        h.clock.advance(1_000);
        let cap = h.capture(GAME).await;
        fake.push(Ok(ocr_output(vec![
            ocr_line("[팀] 가: 미드 갈게", 2.0, 10.0, 50.0, 6.0, None),
            ocr_line("[전체] 나: 안녕", 2.0, 20.0, 50.0, 6.0, None),
            ocr_line("[팀] 가: 미드 갈게", 2.0, 30.0, 50.0, 6.0, None),
        ])));
        h.ok(
            "ocr.recognize",
            json!({"artifactId": cap["artifactId"], "backend": "fake-local"}),
        )
        .await
    };
    let ingest = json!({"session": "lol-team", "observation": {"type": "ocrResult", "ocrResultId": cap_ocr["ocrResultId"]}});
    let b = h.ok("chat.track.ingest", ingest.clone()).await;
    let b2 = h.ok("chat.track.ingest", ingest).await;
    assert_eq!(b2["replay"], true);
    assert_eq!(ids(&b2, "visible"), ids(&b, "visible"));
    assert_eq!(b2["cursor"], b["cursor"]);
    // OCR 改字：同 ID、revision 2；同一人重复同一句：新消息
    assert_eq!(ids(&b, "visible")[0], ids(&a, "visible")[0]);
    assert_eq!(b["visible"][0]["revision"], 2);
    assert!(b["visible"][0]["confidence"].is_null());
    let kinds: Vec<(&str, &str)> = b["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["kind"].as_str().unwrap(), c["text"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("revised", "[팀] 가: 미드 갈게"),
            ("added", "[팀] 가: 미드 갈게")
        ]
    );
    assert_ne!(ids(&b, "visible")[0], ids(&b, "visible")[2]);
    // 滚动：上移一行 + 新消息
    let c = observe(
        &h,
        &fake,
        &[
            ("[전체] 나: 안녕", 10.0),
            ("[팀] 가: 미드 갈게", 20.0),
            ("[팀] 다: 탑 간다", 30.0),
        ],
    )
    .await;
    assert_eq!(&ids(&c, "visible")[..2], &ids(&b, "visible")[1..]);
    assert_eq!(c["changes"].as_array().unwrap().len(), 1);
    assert_eq!(c["changes"][0]["origin"], "new");
    // visible 仍返回已见旧消息；newSince 独立表达新增与修订
    let v = h
        .ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "visible"}),
        )
        .await;
    assert_eq!(v["messages"].as_array().unwrap().len(), 3);
    let since = h
        .ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "newSince", "cursor": cursor1}),
        )
        .await;
    let kinds: Vec<&str> = since["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["revised", "added", "added"]);
    let tail = h
        .ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "newSince", "cursor": since["cursor"]}),
        )
        .await;
    assert!(tail["changes"].as_array().unwrap().is_empty());
    // 空 OCR：不清空可见集、不推进游标
    let e = observe(&h, &fake, &[]).await;
    assert_eq!(e["stale"], true);
    assert_eq!(ids(&e, "visible"), ids(&c, "visible"));
    assert_eq!(e["cursor"], since["cursor"]);
    // 外部视觉观测：box 在产物像素中，映射到根坐标；来源标注
    h.clock.advance(1_000);
    let cap = h.capture(GAME).await;
    let crop = h.ok("image.crop", json!({"artifactId": cap["artifactId"], "rect": {"x": 0, "y": 8, "width": 64, "height": 40}})).await;
    let x = h
        .ok("chat.track.ingest", json!({"session": "lol-team", "observation": {"type": "external", "artifactId": crop["artifactId"], "producer": "agent:vision-model", "lines": [
            {"text": "[팀] 가: 미드 갈게", "box": {"x": 2, "y": 2, "width": 50, "height": 6}},
            {"text": "[팀] 다: 탑 간다", "box": {"x": 2, "y": 12, "width": 50, "height": 6}},
            {"text": "[팀] 라: 용 가자", "box": {"x": 2, "y": 22, "width": 50, "height": 6}}
        ]}}))
        .await;
    let last = x["visible"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        last["rootBox"],
        json!({"x": 2, "y": 30, "width": 50, "height": 6})
    );
    assert_eq!(last["sources"][0]["kind"], "external-vision");
    assert_eq!(last["sources"][0]["producer"], "agent:vision-model");
    assert!(x["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "ROI_CHANGED"));
    // 旧帧的完全相同重放：幂等，不改变状态
    let before = h
        .ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "visible"}),
        )
        .await;
    let replay = h
        .ok("chat.track.ingest", json!({"session": "lol-team", "observation": {"type": "ocrResult", "ocrResultId": cap_ocr["ocrResultId"]}}))
        .await;
    assert_eq!(replay["replay"], true);
    assert_eq!(
        h.ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "visible"})
        )
        .await,
        before
    );
    // 乱序：先采集 A、再采集 B，先提交 B，再提交更早的 A → 拒绝且不改变状态
    h.clock.advance(1_000);
    let frame_a = h.capture(GAME).await;
    h.clock.advance(1_000);
    let frame_b = h.capture(GAME).await;
    let ext = |art: &Value, text: &str| json!({"session": "lol-team", "observation": {"type": "external", "artifactId": art["artifactId"], "producer": "agent", "lines": [{"text": text, "box": {"x": 2, "y": 10, "width": 50, "height": 6}}]}});
    h.ok("chat.track.ingest", ext(&frame_b, "[팀] 마: 나중"))
        .await;
    let before = h
        .ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "newSince"}),
        )
        .await;
    let e = h
        .err("chat.track.ingest", ext(&frame_a, "[팀] 마: 먼저"))
        .await;
    assert_eq!(e.code, "CHAT_TRACK_OUT_OF_ORDER");
    assert_eq!(
        h.ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "newSince"})
        )
        .await,
        before
    );
    // 窗口变化 → 拒绝；new-generation → 新代际
    h.clock.advance(1_000);
    h.desktop.set_foreground(Some(OTHER));
    let other = h.capture(OTHER).await;
    let e = h
        .err("chat.track.ingest", json!({"session": "lol-team", "observation": {"type": "external", "artifactId": other["artifactId"], "producer": "agent", "lines": [{"text": "x"}]}}))
        .await;
    assert_eq!(e.code, "CHAT_TRACK_SOURCE_CHANGED");
    // reset：新代际，旧游标失效，清理计数
    let r = h
        .ok(
            "chat.track.reset",
            json!({"session": "lol-team", "reason": "new-match"}),
        )
        .await;
    assert_eq!(
        (r["previousGeneration"].as_u64(), r["generation"].as_u64()),
        (Some(1), Some(2))
    );
    assert!(r["cleared"]["messages"].as_u64().unwrap() >= 4);
    let e = h
        .err(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "newSince", "cursor": cursor1}),
        )
        .await;
    assert_eq!(e.code, "CHAT_TRACK_CURSOR_STALE");
    let v = h
        .ok(
            "chat.track.read",
            json!({"session": "lol-team", "mode": "visible"}),
        )
        .await;
    assert!(v["messages"].as_array().unwrap().is_empty());
    // 参数错误与未知会话
    for (tool, args, code) in [
        (
            "chat.track.read",
            json!({"session": "nope", "mode": "visible"}),
            "CHAT_TRACK_SESSION_NOT_FOUND",
        ),
        (
            "chat.track.read",
            json!({"session": "lol-team", "mode": "all"}),
            "INVALID_ARGUMENTS",
        ),
        (
            "chat.track.read",
            json!({"session": "lol-team", "mode": "newSince", "cursor": "bogus"}),
            "CHAT_TRACK_CURSOR_INVALID",
        ),
        (
            "chat.track.reset",
            json!({"session": "lol-team", "reason": "because"}),
            "INVALID_ARGUMENTS",
        ),
        (
            "chat.track.ingest",
            json!({"session": "bad name!", "observation": {"type": "ocrResult", "ocrResultId": "x"}}),
            "INVALID_ARGUMENTS",
        ),
        (
            "chat.track.ingest",
            json!({"session": "s", "observation": {"type": "ocrResult", "ocrResultId": "ocr_missing"}}),
            "OCR_RESULT_NOT_FOUND",
        ),
        (
            "chat.track.ingest",
            json!({"session": "s", "observation": {"type": "external", "artifactId": crop["artifactId"], "producer": "a", "lines": [{"text": "x", "box": {"x": 0, "y": 0, "width": 999, "height": 1}}]}}),
            "INVALID_ARGUMENTS",
        ),
        (
            "chat.track.ingest",
            json!({"session": "s", "generation": 7, "observation": {"type": "external", "artifactId": crop["artifactId"], "producer": "a", "lines": []}}),
            "CHAT_TRACK_GENERATION_MISMATCH",
        ),
    ] {
        assert_eq!(h.err(tool, args).await.code, code, "{tool}");
    }
    let status = h.ok("system.status", json!({})).await;
    assert!(status["chatTrackSessions"].as_u64().unwrap() >= 1);
}

/// 跟踪提交与期限线性化：提交停在时钟读取处，期限过后放行 → TIMEOUT，状态不变。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_ingest_commit_does_not_mutate_tracking_state() {
    use crate::testing::GatedClock;
    let desktop = FakeDesktop::new();
    desktop.add_window(GAME, 4242, "Game", "game.exe", 0, 0, 32, 32);
    desktop.set_foreground(Some(GAME));
    let clock = GatedClock::at(10_000);
    let service = crate::ToolService::new(ServiceDeps {
        instance_id: "inst".into(),
        app_version: "t".into(),
        observer: desktop.clone(),
        capturer: desktop.clone(),
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: clock.clone(),
        ocr_backends: vec![],
    });
    let req = |tool: &str, arguments: Value, timeout_ms: u64| ToolCallRequest {
        request_id: None,
        tool: tool.into(),
        arguments,
        timeout_ms: Some(timeout_ms),
    };
    let list = service
        .call(req("window.list", json!({}), 5_000))
        .await
        .result
        .unwrap();
    let win = list["windows"][0]["windowId"].clone();
    let cap = service
        .call(req("screen.capture", json!({"windowId": win}), 5_000))
        .await
        .result
        .unwrap();
    let art = cap["artifactId"].clone();
    // 工作线程读两次时钟（产物查询、ingest 准备），第三次是提交闸门内 tracker 之前的读取
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = reads.clone();
    let (entered, release) =
        clock.arm(move || counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == 3);
    let svc = service.clone();
    let call = tokio::spawn(async move {
        svc.call(req("chat.track.ingest", json!({"session": "s", "observation": {"type": "external", "artifactId": art, "producer": "a", "lines": [{"text": "늦음"}]}}), 150))
            .await
    });
    tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(5)))
        .await
        .unwrap()
        .expect("暂停在提交前");
    tokio::time::sleep(Duration::from_millis(300)).await;
    release.send(()).unwrap();
    let env = call.await.unwrap();
    let e = env.error.unwrap();
    assert_eq!(e.code, "TIMEOUT");
    assert_eq!(e.details.unwrap()["stage"], "late-result", "停在提交闸门内");
    assert_eq!(
        service.resident_ocr_and_sessions().1,
        0,
        "迟到的 ingest 不得创建或修改会话"
    );
}
