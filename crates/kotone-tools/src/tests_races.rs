//! 期限、关闭、空闲回收、采集时刻与连接寿命的确定性回归测试。
//!
//! 这些测试针对评审确认的竞态：执行器停顿导致迟到结果被接受、排队超时的释放在之后
//! 生效、关闭与同步提交之间的重新插入、空闲时到期产物不回收、采集时刻晚于编码、
//! 整连接超时截断在途请求。观测驻留产物一律用不触发清理的接口。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::artifacts::Clock;
use crate::protocol::{Envelope, ToolCallRequest};
use crate::testing::{FakeDesktop, FakeHost, GatedClock, ManualClock};
use crate::{RunningToolService, ServiceDeps, ServiceLimits, ToolService};

const GAME: u64 = 0x1001;

fn desktop() -> Arc<FakeDesktop> {
    let desktop = FakeDesktop::new();
    desktop.add_window(GAME, 4242, "Game", "game.exe", 100, 50, 64, 48);
    desktop.set_foreground(Some(GAME));
    desktop
}

fn deps(desktop: &Arc<FakeDesktop>, clock: Arc<dyn Clock>, limits: ServiceLimits) -> ServiceDeps {
    ServiceDeps {
        instance_id: crate::ids::new_id("inst"),
        app_version: "test".into(),
        observer: desktop.clone(),
        capturer: desktop.clone(),
        asr_phase: Arc::new(|| "stopped".into()),
        limits,
        clock,
    }
}

fn request(tool: &str, arguments: Value, timeout_ms: u64) -> ToolCallRequest {
    ToolCallRequest {
        request_id: None,
        tool: tool.into(),
        arguments,
        timeout_ms: Some(timeout_ms),
    }
}

async fn window_id(service: &Arc<ToolService>) -> String {
    let env = service.call(request("window.list", json!({}), 5_000)).await;
    env.result.unwrap()["windows"][0]["windowId"]
        .as_str()
        .unwrap()
        .to_string()
}

fn code(env: &Envelope) -> String {
    env.error.as_ref().expect("expected failure").code.clone()
}

/// 执行器停顿：期限 30ms、捕获 60ms，运行时唯一线程被阻塞 150ms，使工作线程结果与
/// 计时器同时就绪。tokio 的 timeout_at 先轮询内部 future，会返回 Ok(迟到结果)；
/// 提交闸门必须仍以 TIMEOUT 拒绝，且不发布产物。
#[test]
fn late_worker_result_after_executor_stall_is_not_published() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let desktop = desktop();
        let service = ToolService::new(deps(
            &desktop,
            ManualClock::at(1_000),
            ServiceLimits::default(),
        ));
        let id = window_id(&service).await;
        desktop.set_capture_delay(Duration::from_millis(60));
        let svc = service.clone();
        let task = tokio::spawn(async move {
            svc.call(request("screen.capture", json!({"windowId": id}), 30))
                .await
        });
        // 让调用任务运行到等待工作线程的位置，并确认工作线程已开始捕获
        for _ in 0..500 {
            tokio::task::yield_now().await;
            if desktop.capture_starts() > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(desktop.capture_starts(), 1, "场景要求工作线程在期限前开始");
        // 阻塞唯一的运行时线程：期间期限到达、工作线程完成
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(desktop.capture_count(), 1, "工作线程已产出迟到结果");
        let env = task.await.unwrap();
        let error = env.error.expect("迟到结果不得被接受");
        assert_eq!(error.code, "TIMEOUT");
        assert_eq!(
            error.details.unwrap()["stage"],
            "late-result",
            "必须由提交闸门拒绝（证明内部 future 确实先就绪）"
        );
        assert_eq!(service.resident_artifacts(), (0, 0), "迟到产物不得发布");
    });
}

/// 阻塞线程池饱和：排队的 artifact.release / screen.capture 在开始前超时。
/// 之后线程池恢复，排队任务开始执行时必须在工作线程入口被期限闸门拦下，
/// 释放不得删除产物，捕获不得调用后端。
#[test]
fn queued_work_that_times_out_before_starting_has_no_side_effects() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let desktop = desktop();
        let service = ToolService::new(deps(
            &desktop,
            ManualClock::at(1_000),
            ServiceLimits::default(),
        ));
        let id = window_id(&service).await;
        let capture = service
            .call(request("screen.capture", json!({"windowId": id}), 5_000))
            .await;
        let artifact = capture.result.unwrap()["artifactId"].clone();
        assert_eq!(desktop.capture_starts(), 1);

        // 占满唯一的阻塞线程
        let (unblock, wait) = std::sync::mpsc::channel::<()>();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = wait.recv();
        });
        let release = service
            .call(request(
                "artifact.release",
                json!({"artifactId": artifact}),
                20,
            ))
            .await;
        assert_eq!(code(&release), "TIMEOUT");
        let queued_capture = service
            .call(request("screen.capture", json!({"windowId": id}), 20))
            .await;
        assert_eq!(code(&queued_capture), "TIMEOUT");

        // 恢复线程池并等待排队任务全部执行完（单线程 FIFO）
        unblock.send(()).unwrap();
        blocker.await.unwrap();
        tokio::task::spawn_blocking(|| ()).await.unwrap();

        assert_eq!(
            service.resident_artifacts().0,
            1,
            "超时的释放不得在之后删除产物"
        );
        let got = service
            .call(request(
                "artifact.get",
                json!({"artifactId": artifact, "includeContent": false}),
                5_000,
            ))
            .await;
        assert!(got.ok, "{:?}", got.error);
        assert_eq!(desktop.capture_starts(), 1, "过期的排队捕获不得调用后端");
    });
}

/// 关闭与提交线性化：捕获工作线程已完成，提交在插入前被时钟闸门暂停；此时在另一
/// OS 线程完成 shutdown_now。放行后旧请求不得重新填充存储，新请求被拒绝。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_linearizes_against_a_paused_commit() {
    let desktop = desktop();
    let clock = GatedClock::at(5_000);
    let host = FakeHost::new(Some(3));
    let dir = tempfile::tempdir().unwrap();
    let limits = ServiceLimits {
        maintenance_interval: Duration::from_secs(3600),
        ..Default::default()
    };
    let running =
        RunningToolService::start(deps(&desktop, clock.clone(), limits), host, dir.path())
            .await
            .unwrap();
    let service = running.service().clone();
    let id = window_id(&service).await;

    // 只暂停非捕获线程上的时钟读取，即提交路径中插入前的那一次
    let watcher = desktop.clone();
    let (entered, release) = clock.arm(move || !watcher.is_capture_thread());
    let svc = service.clone();
    let call = tokio::spawn(async move {
        svc.call(request("screen.capture", json!({"windowId": id}), 10_000))
            .await
    });
    tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(5)))
        .await
        .unwrap()
        .expect("提交应停在插入之前");
    assert_eq!(desktop.capture_count(), 1, "工作线程已完成");

    let descriptor = running.descriptor_path().unwrap();
    let closer = std::thread::spawn(move || {
        let mut running = running;
        running.shutdown_now();
        drop(running);
    });
    tokio::task::spawn_blocking(move || closer.join().unwrap())
        .await
        .unwrap();
    assert!(service.is_closed());
    assert!(!descriptor.exists());
    assert_eq!(service.resident_artifacts(), (0, 0));

    release.send(()).unwrap();
    let env = call.await.unwrap();
    assert_eq!(code(&env), "SERVICE_CLOSED", "关闭后旧请求不得发布");
    assert_eq!(service.resident_artifacts(), (0, 0), "存储不得被重新填充");
    let after = service
        .call(request("system.status", json!({}), 1_000))
        .await;
    assert_eq!(code(&after), "SERVICE_CLOSED");
    // 已结束的维护任务不再运行
    for _ in 0..100 {
        if !service.maintenance_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!service.maintenance_running());
}

/// 空闲回收：没有任何读请求时，维护任务按周期回收到期产物并释放字节。
#[tokio::test(flavor = "multi_thread")]
async fn idle_maintenance_reclaims_expired_artifacts_without_reads() {
    let desktop = desktop();
    let clock = ManualClock::at(10_000);
    let host = FakeHost::new(Some(3));
    let dir = tempfile::tempdir().unwrap();
    let limits = ServiceLimits {
        maintenance_interval: Duration::from_millis(20),
        ..Default::default()
    };
    let running =
        RunningToolService::start(deps(&desktop, clock.clone(), limits), host, dir.path())
            .await
            .unwrap();
    let service = running.service().clone();
    assert!(service.maintenance_running());
    let id = window_id(&service).await;
    let capture = service
        .call(request(
            "screen.capture",
            json!({"windowId": id, "ttlMs": 1_000}),
            5_000,
        ))
        .await;
    let artifact = capture.result.unwrap()["artifactId"].clone();
    let (count, bytes) = service.resident_artifacts();
    assert_eq!(count, 1);
    assert!(bytes > 64 * 48 * 4);

    clock.advance(1_001);
    let mut reclaimed = false;
    for _ in 0..200 {
        if service.resident_artifacts() == (0, 0) {
            reclaimed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(reclaimed, "空闲时到期产物必须被回收（字节释放）");
    let env = service
        .call(request(
            "artifact.get",
            json!({"artifactId": artifact}),
            1_000,
        ))
        .await;
    let error = env.error.unwrap();
    assert_eq!(error.code, "ARTIFACT_EXPIRED");
    assert_eq!(error.details.unwrap()["reason"], "ttl");

    running.shutdown().await;
    assert!(!service.maintenance_running(), "关闭后维护任务已退出");
}

/// 对照：没有维护任务时，观测接口本身不会清理，到期产物仍驻留——证明上一测试观察到的
/// 是维护任务的回收，而不是读取接口的隐式清理。
#[tokio::test(flavor = "multi_thread")]
async fn without_maintenance_expired_artifacts_stay_resident_until_touched() {
    let desktop = desktop();
    let clock = ManualClock::at(10_000);
    let service = ToolService::new(deps(&desktop, clock.clone(), ServiceLimits::default()));
    let id = window_id(&service).await;
    service
        .call(request(
            "screen.capture",
            json!({"windowId": id, "ttlMs": 1_000}),
            5_000,
        ))
        .await;
    clock.advance(5_000);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(service.resident_artifacts().0, 1);
}

/// 采集时刻取自捕获返回边界：编码前人为推进时钟 5 秒，capturedAt 不得包含这段处理时间；
/// createdAt 是发布时刻。裁剪沿用根产物的采集时刻。
#[tokio::test(flavor = "multi_thread")]
async fn captured_at_is_the_capture_boundary_and_survives_crops() {
    let desktop = desktop();
    let clock = ManualClock::at(1_000_000);
    let service = ToolService::new(deps(&desktop, clock.clone(), ServiceLimits::default()));
    let id = window_id(&service).await;
    let hook_clock = clock.clone();
    *service.before_encode_hook.lock().unwrap() = Some(Arc::new(move || hook_clock.advance(5_000)));
    let capture = service
        .call(request("screen.capture", json!({"windowId": id}), 5_000))
        .await
        .result
        .unwrap();
    assert_eq!(capture["source"]["capturedAtUnixMs"], 1_000_000);
    assert_eq!(capture["createdAtUnixMs"], 1_005_000);
    assert_eq!(capture["expiresAtUnixMs"], 1_005_000 + 300_000);

    clock.advance(7_000);
    let crop = service
        .call(request(
            "image.crop",
            json!({"artifactId": capture["artifactId"], "rect": {"x":0,"y":0,"width":2,"height":2}}),
            5_000,
        ))
        .await
        .result
        .unwrap();
    assert_eq!(
        crop["source"]["capturedAtUnixMs"], 1_000_000,
        "裁剪保留原始采集时刻"
    );
    assert_eq!(crop["createdAtUnixMs"], 1_012_000);
}

// ---------- 连接寿命（真实 loopback HTTP，手写 HTTP/1.1） ----------

struct HttpHarness {
    running: RunningToolService,
    token: String,
    desktop: Arc<FakeDesktop>,
    _dir: tempfile::TempDir,
}

async fn http_harness(limits: ServiceLimits) -> HttpHarness {
    let desktop = desktop();
    let host = FakeHost::new(Some(3));
    let dir = tempfile::tempdir().unwrap();
    let running = RunningToolService::start(
        deps(&desktop, Arc::new(crate::artifacts::SystemClock), limits),
        host,
        dir.path(),
    )
    .await
    .unwrap();
    let path = crate::discovery::descriptor_path(dir.path(), Some(3));
    let descriptor: crate::discovery::Descriptor =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    HttpHarness {
        running,
        token: descriptor.token,
        desktop,
        _dir: dir,
    }
}

fn get(h: &HttpHarness, path: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: keep-alive\r\n\r\n",
        h.running.local_addr(),
        h.token
    )
}

fn post(h: &HttpHarness, body: &str) -> String {
    format!(
        "POST /v1/tools/call HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
        h.running.local_addr(),
        h.token,
        body.len()
    )
}

/// 读取一个完整响应：返回 (状态码, 响应体)；连接在读到完整响应前关闭则返回 None。
async fn read_response(stream: &mut tokio::net::TcpStream) -> Option<(u16, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_string();
            let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
            let len: usize = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(0);
            while buf.len() < end + 4 + len {
                let n = stream.read(&mut chunk).await.ok()?;
                if n == 0 {
                    return None;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = String::from_utf8_lossy(&buf[end + 4..end + 4 + len]).to_string();
            return Some((status, body));
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn is_closed_by_server(stream: &mut tokio::net::TcpStream, within: Duration) -> bool {
    let mut byte = [0u8; 1];
    matches!(
        tokio::time::timeout(within, stream.read(&mut byte)).await,
        Ok(Ok(0)) | Ok(Err(_))
    )
}

/// 同一 keep-alive 连接：寿命 300ms；第一个请求后空闲 200ms，在寿命到期前开始一个
/// 需要 500ms 的捕获（工具期限 3s）。整连接超时会在 300ms 截断它；优雅到期必须让它完成，
/// 随后服务端关闭连接。
#[tokio::test(flavor = "multi_thread")]
async fn connection_lifetime_does_not_cut_an_in_flight_request() {
    let limits = ServiceLimits {
        connection_lifetime: Duration::from_millis(300),
        connection_grace: Duration::from_secs(5),
        ..Default::default()
    };
    let h = http_harness(limits).await;
    let id = window_id(h.running.service()).await;
    h.desktop.set_capture_delay(Duration::from_millis(500));
    let mut stream = tokio::net::TcpStream::connect(h.running.local_addr())
        .await
        .unwrap();
    stream
        .write_all(get(&h, "/v1/status").as_bytes())
        .await
        .unwrap();
    let (status, _) = read_response(&mut stream).await.expect("first response");
    assert_eq!(status, 200);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = std::time::Instant::now();
    let body = json!({"tool": "screen.capture", "arguments": {"windowId": id}, "timeoutMs": 3000})
        .to_string();
    stream.write_all(post(&h, &body).as_bytes()).await.unwrap();
    let (status, body) = read_response(&mut stream)
        .await
        .expect("在途请求不得被连接寿命截断");
    assert_eq!(status, 200);
    let env: Envelope = serde_json::from_str(&body).unwrap();
    assert!(env.ok, "{:?}", env.error);
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "请求确实跨越了寿命边界"
    );
    assert!(
        is_closed_by_server(&mut stream, Duration::from_secs(2)).await,
        "寿命到期后在途请求完成即关闭连接"
    );
    assert_eq!(h.running.service().resident_artifacts().0, 1);
}

/// 寿命仍然有效：空闲的 keep-alive 连接在寿命到期后被关闭。
#[tokio::test(flavor = "multi_thread")]
async fn idle_keep_alive_connection_is_closed_at_lifetime() {
    let limits = ServiceLimits {
        connection_lifetime: Duration::from_millis(200),
        ..Default::default()
    };
    let h = http_harness(limits).await;
    let mut stream = tokio::net::TcpStream::connect(h.running.local_addr())
        .await
        .unwrap();
    stream
        .write_all(get(&h, "/v1/status").as_bytes())
        .await
        .unwrap();
    assert_eq!(read_response(&mut stream).await.unwrap().0, 200);
    assert!(is_closed_by_server(&mut stream, Duration::from_secs(2)).await);
}

/// 请求头与请求体读取期限仍然有效。
#[tokio::test(flavor = "multi_thread")]
async fn header_and_body_read_limits_still_apply() {
    let limits = ServiceLimits {
        header_read_timeout: Duration::from_millis(200),
        body_read_timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let h = http_harness(limits).await;
    // 不完整的请求头：连接被关闭（不会无限占用连接名额）
    let mut stream = tokio::net::TcpStream::connect(h.running.local_addr())
        .await
        .unwrap();
    stream
        .write_all(b"GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .await
        .unwrap();
    let mut rest = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut rest)).await;
    assert!(read.is_ok(), "请求头超时后连接应被关闭");
    // 请求体声明 100 字节只发 10 字节：408 REQUEST_TIMEOUT
    let mut stream = tokio::net::TcpStream::connect(h.running.local_addr())
        .await
        .unwrap();
    let head = format!(
        "POST /v1/tools/call HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{\"tool\":\"",
        h.running.local_addr(),
        h.token
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let (status, body) = tokio::time::timeout(Duration::from_secs(3), read_response(&mut stream))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, 408);
    let env: Envelope = serde_json::from_str(&body).unwrap();
    assert_eq!(env.error.unwrap().code, "REQUEST_TIMEOUT");
}
