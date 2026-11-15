//! `kotone-cli tools` 集成测试：真实 CLI 二进制 → 描述文件发现（真实 Windows 私有 ACL）
//! → loopback HTTP → 工具服务（假窗口/捕获后端）→ PNG 写出到临时目录。
//! 不启动桌面 exe，不读取用户配置，不触碰真实窗口。

#![cfg(windows)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::{json, Value};

use kotone_core::tools::ToolHost;
use kotone_platform_windows::tool_host::WindowsToolHost;
use kotone_tools::artifacts::SystemClock;
use kotone_tools::discovery::{self, Descriptor};
use kotone_tools::testing::{decode_png, pattern_pixel, FakeDesktop};
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

fn cli(dir: &Path, args: &[&str], stdin: Option<&str>) -> Run {
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

async fn cli_async(dir: PathBuf, args: Vec<String>, stdin: Option<String>) -> Run {
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli(&dir, &refs, stdin.as_deref())
    })
    .await
    .unwrap()
}

macro_rules! run {
    ($dir:expr, [$($arg:expr),* $(,)?]) => {
        cli_async($dir.to_path_buf(), vec![$($arg.to_string()),*], None).await
    };
    ($dir:expr, [$($arg:expr),* $(,)?], stdin = $input:expr) => {
        cli_async($dir.to_path_buf(), vec![$($arg.to_string()),*], Some($input.to_string())).await
    };
}

fn write_args(dir: &Path, name: &str, value: Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

async fn start_service(run_dir: &Path) -> (RunningToolService, Arc<FakeDesktop>) {
    let desktop = FakeDesktop::new();
    desktop.add_window(0x1001, 4242, "Fake Game", "game.exe", 200, 100, 40, 30);
    desktop.set_foreground(Some(0x1001));
    let deps = ServiceDeps {
        instance_id: kotone_tools::ids::new_id("inst"),
        app_version: "0.0.0-cli-test".into(),
        observer: desktop.clone(),
        capturer: desktop.clone(),
        asr_phase: Arc::new(|| "stopped".into()),
        limits: ServiceLimits::default(),
        clock: Arc::new(SystemClock),
        ocr_backends: Vec::new(),
        host: std::sync::Arc::new(kotone_tools::host::NoAppHost),
    };
    let running = RunningToolService::start(deps, Arc::new(WindowsToolHost), run_dir)
        .await
        .unwrap();
    (running, desktop)
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_drives_a_running_service_end_to_end() {
    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let work = temp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let (running, _desktop) = start_service(&run_dir).await;
    let descriptor: Descriptor =
        serde_json::from_slice(&std::fs::read(running.descriptor_path().unwrap()).unwrap())
            .unwrap();
    let token = descriptor.token.clone();
    let mut outputs = Vec::new();

    // status：ASR 停止时可用
    let status = run!(run_dir, ["tools", "status", "--json"]);
    assert_eq!(status.code, 0, "{}{}", status.stdout, status.stderr);
    let env = status.json();
    assert_eq!(env["ok"], true);
    assert_eq!(env["instanceId"], running.instance_id());
    assert_eq!(env["result"]["asr"]["runtimePhase"], "stopped");
    outputs.push(status);
    let human = run!(run_dir, ["tools", "status"]);
    assert_eq!(human.code, 0);
    assert!(human.stdout.contains("运行中") && human.stdout.contains("stopped"));
    outputs.push(human);

    // list：实际能力
    let list = run!(run_dir, ["tools", "list", "--json"]);
    assert_eq!(list.code, 0);
    let names: Vec<String> = list.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, kotone_tools::catalog::IMPLEMENTED_TOOLS);
    outputs.push(list);
    let plain = run!(run_dir, ["tools", "list"]);
    assert!(plain.stdout.contains("screen.capture") && plain.stdout.contains("ready"));
    outputs.push(plain);

    // window.active → screen.capture → image.crop
    let active = run!(run_dir, ["tools", "call", "window.active"]);
    assert_eq!(active.code, 0, "{}", active.stdout);
    let window_id = active.json()["result"]["window"]["windowId"]
        .as_str()
        .unwrap()
        .to_string();
    outputs.push(active);
    let capture_args = write_args(&work, "capture.json", json!({"windowId": window_id}));
    let capture = run!(
        run_dir,
        [
            "tools",
            "call",
            "screen.capture",
            "--args-file",
            capture_args.display(),
            "--request-id",
            "agent-1"
        ]
    );
    assert_eq!(capture.code, 0, "{}", capture.stdout);
    let capture_env = capture.json();
    assert_eq!(capture_env["requestId"], "agent-1");
    let source_id = capture_env["result"]["artifactId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(capture_env["result"]["width"], 40);
    outputs.push(capture);
    let crop = run!(
        run_dir,
        ["tools", "call", "image.crop", "--args-file", "-"],
        stdin = json!({"artifactId": source_id, "rect": {"x": 7, "y": 3, "width": 9, "height": 4}})
    );
    assert_eq!(crop.code, 0, "{}", crop.stdout);
    let crop_env = crop.json();
    let crop_id = crop_env["result"]["artifactId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(crop_env["result"]["parentArtifactId"], source_id.as_str());
    assert_eq!(
        crop_env["result"]["screenRect"],
        json!({"x":207,"y":103,"width":9,"height":4})
    );
    outputs.push(crop);

    // artifact get：客户端写文件，内容逐像素核对
    let png_path = work.join("chat.png");
    let got = run!(
        run_dir,
        [
            "tools",
            "artifact",
            "get",
            crop_id,
            "--output",
            png_path.display()
        ]
    );
    assert_eq!(got.code, 0, "{}{}", got.stdout, got.stderr);
    let got_env = got.json();
    assert!(
        got_env["result"].get("content").is_none(),
        "CLI 不把 base64 打到终端"
    );
    assert_eq!(
        got_env["result"]["output"]["bytesWritten"].as_u64(),
        Some(std::fs::metadata(&png_path).unwrap().len())
    );
    let bytes = std::fs::read(&png_path).unwrap();
    assert_eq!(
        kotone_tools::image::sha256_hex(&bytes),
        crop_env["result"]["sha256"].as_str().unwrap()
    );
    let (w, h, px) = decode_png(&bytes);
    assert_eq!((w, h), (9, 4));
    for y in 0..4u32 {
        for x in 0..9u32 {
            let i = ((y * 9 + x) * 4) as usize;
            assert_eq!(&px[i..i + 4], &pattern_pixel(x + 7, y + 3));
        }
    }
    outputs.push(got);
    let again = run!(
        run_dir,
        [
            "tools",
            "artifact",
            "get",
            crop_id,
            "--output",
            png_path.display()
        ]
    );
    assert_eq!(again.code, 2, "不覆盖已有文件");
    let forced = run!(
        run_dir,
        [
            "tools",
            "artifact",
            "get",
            crop_id,
            "--output",
            png_path.display(),
            "--force"
        ]
    );
    assert_eq!(forced.code, 0);
    outputs.push(forced);

    // 工具错误：退出码 1，信封中带错误码
    let planned = run!(run_dir, ["tools", "call", "tts.speak"]);
    assert_eq!(planned.code, 1);
    assert_eq!(planned.json()["error"]["code"], "TOOL_NOT_FOUND");
    outputs.push(planned);
    let missing = run!(
        run_dir,
        [
            "tools",
            "artifact",
            "get",
            "art_unknown",
            "--output",
            work.join("x.png").display()
        ]
    );
    assert_eq!(missing.code, 1);
    assert_eq!(missing.json()["error"]["code"], "ARTIFACT_NOT_FOUND");
    assert!(!work.join("x.png").exists());
    let release_args = write_args(&work, "release.json", json!({"artifactId": source_id}));
    let released = run!(
        run_dir,
        [
            "tools",
            "call",
            "artifact.release",
            "--args-file",
            release_args.display()
        ]
    );
    assert_eq!(released.json()["result"]["status"], "released");
    let released_again = run!(
        run_dir,
        [
            "tools",
            "call",
            "artifact.release",
            "--args-file",
            release_args.display()
        ]
    );
    assert_eq!(released_again.code, 0);
    assert_eq!(
        released_again.json()["result"]["status"],
        "already-released"
    );
    // 本地参数错误：退出码 2，不连接服务
    let bad = write_args(&work, "bad.json", json!([1, 2]));
    let bad_run = run!(
        run_dir,
        [
            "tools",
            "call",
            "screen.capture",
            "--args-file",
            bad.display()
        ]
    );
    assert_eq!(bad_run.code, 2);

    for output in &outputs {
        assert!(!output.stdout.contains(&token), "stdout 泄露凭证");
        assert!(!output.stderr.contains(&token), "stderr 泄露凭证");
    }

    // 退出后：描述文件已删除 → SERVICE_NOT_RUNNING
    drop(running);
    let after = run!(run_dir, ["tools", "status"]);
    assert_eq!(after.code, 3);
    assert_eq!(after.json()["error"]["code"], "SERVICE_NOT_RUNNING");
    assert_eq!(after.json()["instanceId"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_reports_stale_and_missing_descriptors() {
    let temp = tempfile::tempdir().unwrap();
    let empty = temp.path().join("empty");
    let none = run!(empty, ["tools", "list", "--json"]);
    assert_eq!(none.code, 3);
    assert_eq!(none.json()["error"]["code"], "SERVICE_NOT_RUNNING");
    assert!(!empty.exists(), "客户端不创建运行目录");

    // 已退出进程留下的描述文件
    let run_dir = temp.path().join("run");
    let host = WindowsToolHost;
    host.prepare_private_dir(&run_dir).unwrap();
    let mut child = Command::new("cmd")
        .args(["/C", "exit"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let dead_pid = child.id();
    child.wait().unwrap();
    let descriptor = Descriptor {
        schema: discovery::DESCRIPTOR_SCHEMA.into(),
        protocol: kotone_tools::protocol::PROTOCOL.into(),
        protocol_version: kotone_tools::protocol::PROTOCOL_VERSION,
        instance_id: "inst_dead".into(),
        app_version: "0".into(),
        pid: dead_pid,
        process_started_at: Some(1),
        session_id: host.session_id(),
        address: "127.0.0.1:9".into(),
        token: "a".repeat(64),
        created_at_unix_ms: 0,
    };
    let path = discovery::descriptor_path(&run_dir, host.session_id());
    host.write_private_file(&path, &serde_json::to_vec(&descriptor).unwrap())
        .unwrap();
    let stale = run!(run_dir, ["tools", "status"]);
    assert_eq!(stale.code, 3, "{}", stale.stdout);
    assert_eq!(stale.json()["error"]["code"], "STALE_DESCRIPTOR");
    assert!(!stale.stdout.contains(&"a".repeat(64)));

    // 权限被放宽的描述文件
    let out = Command::new("icacls")
        .arg(&path)
        .args(["/grant", "*S-1-1-0:R"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let insecure = run!(run_dir, ["tools", "status"]);
    assert_eq!(insecure.code, 3);
    assert_eq!(insecure.json()["error"]["code"], "DESCRIPTOR_INSECURE");
}
