//! M2 CLI 集成：真实 `kotone-cli` 二进制 → 描述文件发现 → loopback HTTP → 工具服务，
//! 覆盖 image.preprocess / ocr.backends / ocr.recognize / chat.track.* 与 `ocr status|install`。
//! 普通用例使用假窗口与假 OCR 后端、临时目录，不读取用户配置、不触网、不加载真实模型。
//!
//! 另含一个默认忽略的真实本地韩文 OCR 冒烟（显式隔离的模型目录与输入图像，见该用例注释）。

#![cfg(windows)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use kotone_core::ocr::{OcrBackend, OcrBackendKind};
use kotone_platform_windows::tool_host::WindowsToolHost;
use kotone_tools::artifacts::SystemClock;
use kotone_tools::discovery;
use kotone_tools::testing::{ocr_line, ocr_output, FakeDesktop, FakeOcr};
use kotone_tools::{RunningToolService, ServiceDeps, ServiceLimits};

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout 不是 JSON ({e}): {}\n{}", self.stdout, self.stderr))
    }
}

fn cli(dir: &Path, args: &[String], stdin: Option<&str>, envs: &[(&str, &str)]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kotone-cli"));
    command
        .args(args)
        .env(discovery::DIR_ENV, dir)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        command.env(k, v);
    }
    let mut child = command.spawn().expect("启动 kotone-cli");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

async fn run(dir: &Path, args: &[&str], stdin: Option<Value>) -> Run {
    let dir = dir.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let stdin = stdin.map(|v| v.to_string());
    tokio::task::spawn_blocking(move || cli(&dir, &args, stdin.as_deref(), &[]))
        .await
        .unwrap()
}

/// `tools call <tool> --args-file -`，断言退出码并返回信封。
async fn call(dir: &Path, tool: &str, args: Value, expect: i32) -> Value {
    let r = run(
        dir,
        &[
            "tools",
            "call",
            tool,
            "--args-file",
            "-",
            "--timeout-ms",
            "20000",
        ],
        Some(args),
    )
    .await;
    assert_eq!(r.code, expect, "{tool}: {}{}", r.stdout, r.stderr);
    r.json()
}

async fn start(
    run_dir: &Path,
    desktop: Arc<FakeDesktop>,
    backends: Vec<Arc<dyn OcrBackend>>,
) -> RunningToolService {
    let deps = ServiceDeps {
        instance_id: kotone_tools::ids::new_id("inst"),
        app_version: "0.0.0-cli-m2".into(),
        observer: desktop.clone(),
        capturer: desktop,
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: Arc::new(SystemClock),
        ocr_backends: backends,
        host: std::sync::Arc::new(kotone_tools::host::NoAppHost),
    };
    RunningToolService::start(deps, Arc::new(WindowsToolHost), run_dir)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_drives_preprocess_ocr_and_chat_tracking() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let desktop = FakeDesktop::new();
    desktop.add_window(0x1001, 4242, "Fake Game", "game.exe", 200, 100, 40, 30);
    desktop.set_foreground(Some(0x1001));
    let fake = FakeOcr::new("fake-local", OcrBackendKind::Local);
    let running = start(&run_dir, desktop, vec![fake.clone()]).await;

    let list = run(&run_dir, &["tools", "list", "--json"], None).await;
    assert_eq!(list.code, 0);
    let names: Vec<String> = list.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, kotone_tools::catalog::IMPLEMENTED_TOOLS);
    for tool in [
        "image.preprocess",
        "ocr.backends",
        "ocr.recognize",
        "chat.track.ingest",
        "chat.track.read",
        "chat.track.reset",
    ] {
        assert!(names.iter().any(|n| n == tool));
    }
    let plain = run(&run_dir, &["tools", "list"], None).await;
    assert!(plain.stdout.contains("chat.track.read"));

    let backends = call(&run_dir, "ocr.backends", json!({}), 0).await;
    assert_eq!(backends["result"]["backends"][0]["id"], "fake-local");
    assert_eq!(backends["result"]["fallback"], "none");

    let active = call(&run_dir, "window.active", json!({}), 0).await;
    let window = active["result"]["window"]["windowId"].clone();
    let capture = call(&run_dir, "screen.capture", json!({"windowId": window}), 0).await;
    let up = call(
        &run_dir,
        "image.preprocess",
        json!({"artifactId": capture["result"]["artifactId"], "preset": "upscale-2x-cubic"}),
        0,
    )
    .await;
    assert_eq!(up["result"]["width"], 80);
    assert_eq!(up["result"]["rootTransform"]["scaleX"], 0.5);
    let crop = call(&run_dir, "image.crop", json!({"artifactId": up["result"]["artifactId"], "rect": {"x": 20, "y": 10, "width": 40, "height": 30}}), 0).await;
    assert_eq!(
        crop["result"]["screenRect"],
        json!({"x": 210, "y": 105, "width": 20, "height": 15})
    );

    fake.push(Ok(ocr_output(vec![
        ocr_line("[팀] 가: 미드 갈게", 2.0, 4.0, 30.0, 8.0, None),
        ocr_line("[팀] 나: ㅇㅋ", 2.0, 16.0, 20.0, 8.0, Some(0.8)),
    ])));
    let ocr = call(
        &run_dir,
        "ocr.recognize",
        json!({"artifactId": crop["result"]["artifactId"], "backend": "fake-local"}),
        0,
    )
    .await;
    let result = &ocr["result"];
    assert_eq!(
        result["observations"][0]["rootBox"],
        json!({"x": 11, "y": 7, "width": 15, "height": 4})
    );
    assert!(result["observations"][0]["confidence"].is_null());
    assert!(ocr["timing"]["ocrMs"].is_u64());
    let unknown = call(
        &run_dir,
        "ocr.recognize",
        json!({"artifactId": crop["result"]["artifactId"], "backend": "nope"}),
        1,
    )
    .await;
    assert_eq!(unknown["error"]["code"], "OCR_BACKEND_NOT_FOUND");

    let ingest = call(&run_dir, "chat.track.ingest", json!({"session": "team", "observation": {"type": "ocrResult", "ocrResultId": result["ocrResultId"]}}), 0).await;
    assert_eq!(ingest["result"]["visible"].as_array().unwrap().len(), 2);
    let replay = call(&run_dir, "chat.track.ingest", json!({"session": "team", "observation": {"type": "ocrResult", "ocrResultId": result["ocrResultId"]}}), 0).await;
    assert_eq!(replay["result"]["replay"], true);
    let visible = call(
        &run_dir,
        "chat.track.read",
        json!({"session": "team", "mode": "visible"}),
        0,
    )
    .await;
    assert_eq!(visible["result"]["messages"][1]["text"], "[팀] 나: ㅇㅋ");
    let since = call(
        &run_dir,
        "chat.track.read",
        json!({"session": "team", "mode": "newSince", "cursor": ingest["result"]["cursor"]}),
        0,
    )
    .await;
    assert!(since["result"]["changes"].as_array().unwrap().is_empty());
    let reset = call(
        &run_dir,
        "chat.track.reset",
        json!({"session": "team", "reason": "reconnect"}),
        0,
    )
    .await;
    assert_eq!(reset["result"]["generation"], 2);
    let stale = call(
        &run_dir,
        "chat.track.read",
        json!({"session": "team", "mode": "newSince", "cursor": ingest["result"]["cursor"]}),
        1,
    )
    .await;
    assert_eq!(stale["error"]["code"], "CHAT_TRACK_CURSOR_STALE");
    drop(running);
}

#[tokio::test(flavor = "multi_thread")]
async fn ocr_status_and_install_never_publish_unverified_models() {
    let temp = tempfile::tempdir().unwrap();
    let models = temp.path().join("models");
    let source = temp.path().join("source");
    std::fs::create_dir_all(&source).unwrap();
    let models_s = models.display().to_string();
    let status = run(
        temp.path(),
        &["ocr", "status", "--model-dir", &models_s, "--json"],
        None,
    )
    .await;
    assert_eq!(status.code, 1, "{}", status.stderr);
    let v = status.json();
    assert_eq!(v["state"], "not-installed");
    assert_eq!(v["details"]["runtime"]["name"], "onnxruntime");
    assert!(
        v["details"]["runtime"]["version"]
            .as_str()
            .unwrap()
            .starts_with("1."),
        "CLI 默认构建链接 ONNX Runtime"
    );
    assert_eq!(v["details"]["model"]["license"], "Apache-2.0");
    assert_eq!(
        v["details"]["model"]["files"][1]["sha256"],
        "cd6e2ea50f6943ca7271eb8c56a877a5a90720b7047fe9c41a2e541a25773c9b"
    );
    // 离线安装源里的文件被篡改（大小/哈希不符）：失败且模型目录中不发布任何文件
    std::fs::write(source.join("ch_PP-OCRv5_det_mobile.onnx"), b"not the model").unwrap();
    std::fs::write(
        source.join("korean_PP-OCRv5_rec_mobile.onnx"),
        b"not the model",
    )
    .unwrap();
    let source_s = source.display().to_string();
    let install = run(
        temp.path(),
        &[
            "ocr",
            "install",
            "--model-dir",
            &models_s,
            "--from-dir",
            &source_s,
            "--json",
        ],
        None,
    )
    .await;
    assert_eq!(install.code, 1, "{}{}", install.stdout, install.stderr);
    assert_eq!(install.json()["ok"], false);
    let published: Vec<_> = std::fs::read_dir(&models)
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(published.is_empty(), "校验失败不得留下文件: {published:?}");
    let missing = run(
        temp.path(),
        &[
            "ocr",
            "install",
            "--model-dir",
            &models_s,
            "--from-dir",
            "Z:/definitely/missing",
        ],
        None,
    )
    .await;
    assert_eq!(missing.code, 2);
}

fn decode_rgba(path: &Path) -> (u32, u32, Vec<u8>) {
    let bytes = std::fs::read(path).expect("读取冒烟输入");
    let mut decoder = png::Decoder::new(bytes.as_slice());
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("不支持的 PNG 颜色类型 {other:?}"),
    };
    (info.width, info.height, rgba)
}

/// 真实本地韩文 OCR 冒烟（默认忽略）：真实 PP-OCRv5 模型 + 真实 HTTP 服务 + 真实 CLI。
///
/// ```powershell
/// kotone-cli ocr install --model-dir <隔离目录>
/// $env:KOTONE_OCR_SMOKE_MODEL_DIR = "<隔离目录>"; $env:KOTONE_OCR_SMOKE_IMAGE = "<png>"
/// cargo test -p kotone-cli --test tools_cli_m2 -- --ignored --nocapture
/// ```
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要显式隔离的已安装模型与输入图像"]
async fn real_local_korean_ocr_smoke_over_http_and_cli() {
    let model_dir = PathBuf::from(
        std::env::var("KOTONE_OCR_SMOKE_MODEL_DIR").expect("KOTONE_OCR_SMOKE_MODEL_DIR"),
    );
    let image =
        PathBuf::from(std::env::var("KOTONE_OCR_SMOKE_IMAGE").expect("KOTONE_OCR_SMOKE_IMAGE"));
    let (w, h, rgba) = decode_rgba(&image);
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let desktop = FakeDesktop::new();
    desktop.add_window(0x1001, 4242, "Smoke", "smoke.exe", 0, 0, w, h);
    desktop.set_image(0x1001, w, h, rgba);
    desktop.set_foreground(Some(0x1001));
    let local: Arc<dyn OcrBackend> = Arc::new(kotone_ocr::LocalKoreanOcr::new(model_dir.clone()));
    let running = start(&run_dir, desktop, vec![local]).await;

    let status = run(
        &run_dir,
        &[
            "ocr",
            "status",
            "--model-dir",
            &model_dir.display().to_string(),
            "--json",
        ],
        None,
    )
    .await;
    assert_eq!(status.code, 0, "{}", status.stdout);
    let backends = call(&run_dir, "ocr.backends", json!({}), 0).await;
    assert_eq!(backends["result"]["backends"][0]["state"], "ready");
    let active = call(&run_dir, "window.active", json!({}), 0).await;
    let capture = call(
        &run_dir,
        "screen.capture",
        json!({"windowId": active["result"]["window"]["windowId"]}),
        0,
    )
    .await;
    let art = capture["result"]["artifactId"].clone();
    println!(
        "input: {} {}x{} sha256(png artifact)={}",
        image.display(),
        w,
        h,
        capture["result"]["sha256"]
    );
    for round in ["cold", "warm", "warm"] {
        let started = Instant::now();
        let ocr = call(
            &run_dir,
            "ocr.recognize",
            json!({"artifactId": art, "backend": "local-ppocrv5-korean"}),
            0,
        )
        .await;
        let wall = started.elapsed().as_millis();
        let r = &ocr["result"];
        println!(
            "== {round}: coldStart={} cliWallMs={wall} timing={}",
            r["coldStart"], ocr["timing"]
        );
        println!("   model={}", r["backend"]["modelVersion"]);
        for o in r["observations"].as_array().unwrap() {
            println!(
                "   {} conf={} rootBox={}",
                o["text"], o["confidence"], o["rootBox"]
            );
        }
        assert!(!r["empty"].as_bool().unwrap());
        assert!(r["observations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["granularity"] == "line" && o["rootBox"].is_object()));
        if round == "cold" {
            let ingest = call(&run_dir, "chat.track.ingest", json!({"session": "smoke", "observation": {"type": "ocrResult", "ocrResultId": r["ocrResultId"]}}), 0).await;
            println!(
                "   tracked visible={}",
                ingest["result"]["visible"].as_array().unwrap().len()
            );
        }
    }
    // 预处理链：2 倍放大后识别，框仍映射回原图
    let up = call(
        &run_dir,
        "image.preprocess",
        json!({"artifactId": art, "preset": "upscale-2x-cubic"}),
        0,
    )
    .await;
    let ocr = call(
        &run_dir,
        "ocr.recognize",
        json!({"artifactId": up["result"]["artifactId"], "backend": "local-ppocrv5-korean"}),
        0,
    )
    .await;
    println!("== upscale-2x: timing={}", ocr["timing"]);
    for o in ocr["result"]["observations"].as_array().unwrap() {
        println!(
            "   {} conf={} rootBox={}",
            o["text"], o["confidence"], o["rootBox"]
        );
    }
    drop(running);
}
