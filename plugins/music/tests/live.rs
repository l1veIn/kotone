//! 真网络冒烟（不登录）：默认不跑，`cargo test -p kotone-plugin-music --test live -- --ignored --nocapture`。

use std::sync::Arc;

use kotone_plugin_music::{MemorySessionStore, MusicProvider, NeteaseProvider, Playable, Quality};

#[tokio::test]
#[ignore = "talks to music.163.com"]
async fn anonymous_search_stream_and_qr() {
    let provider = NeteaseProvider::new(Arc::new(MemorySessionStore::default()));
    let (tracks, more) = provider.search("孤勇者", 0, 10).await.expect("search");
    println!("search: {} tracks, more={more}", tracks.len());
    for t in &tracks {
        println!("  {} {} {:?} {:?}", t.id, t.title(), t.playable, t.reason);
    }
    assert!(!tracks.is_empty());
    for t in tracks.iter().take(5) {
        let stream = provider
            .stream(&t.id, Quality::Exhigh)
            .await
            .expect("stream");
        println!(
            "  stream {}: {:?}",
            t.id,
            stream
                .as_ref()
                .map(|s| (s.bitrate, s.trial, &s.url[..s.url.len().min(40)]))
        );
    }
    assert!(
        tracks.iter().any(|t| t.playable == Playable::Vip),
        "陈奕迅的原唱要 VIP"
    );
    let qr = provider.login_qr_start().await.expect("qr");
    println!("qr: {} ({} bytes svg)", qr.content, qr.svg.len());
    assert!(qr.content.contains("codekey="));
}

/// 真的出声（很小声，几秒）：搜歌直接放、跳转、暂停、停。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to music.163.com and plays sound"]
async fn plays_a_free_cover_quietly() {
    use kotone_plugin_music::{Player, PlayerState, Status};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    let provider = Arc::new(NeteaseProvider::new(
        Arc::new(MemorySessionStore::default()),
    ));
    let states = Arc::new(Mutex::new(Vec::<PlayerState>::new()));
    let sink = states.clone();
    let player = Player::new(provider, Arc::new(move |s| sink.lock().unwrap().push(s)));
    player.set_volume(20);
    let started = Instant::now();
    let outcome = player.play_query("孤勇者").await.expect("play");
    let track = outcome.started.expect("a free version");
    println!(
        "started {} after skipping {:?}",
        track.title(),
        outcome
            .skipped
            .iter()
            .map(|(t, r)| (t.title(), r.clone()))
            .collect::<Vec<_>>()
    );
    let wait = |want: Status, min_pos: u64, secs: u64| {
        let player = &player;
        async move {
            let deadline = Instant::now() + Duration::from_secs(secs);
            loop {
                let s = player.state();
                if s.status == want && s.position_ms >= min_pos {
                    return s;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {want:?}/{min_pos}: {s:?}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };
    let s = wait(Status::Playing, 1, 15).await;
    println!(
        "sound after {} ms (duration {} ms)",
        started.elapsed().as_millis(),
        s.duration_ms
    );
    let s = wait(Status::Playing, 2500, 10).await;
    println!("at {} ms", s.position_ms);
    player.seek(60_000);
    let s = wait(Status::Playing, 61_000, 10).await;
    println!("after seek: {} ms", s.position_ms);
    player.pause();
    let paused = player.state().position_ms;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let s = player.state();
    assert_eq!(s.status, Status::Paused);
    assert!(s.position_ms <= paused + 100, "no progress while paused");
    player.resume();
    wait(Status::Playing, paused + 500, 5).await;
    player.stop();
    assert_eq!(player.state().status, Status::Idle);
    let emitted = states.lock().unwrap().len();
    println!("{emitted} state updates");
    assert!(emitted >= 5);
}
