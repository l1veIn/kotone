//! 真网络冒烟（不登录）：默认不跑，`cargo test -p kotone-music --test live -- --ignored --nocapture`。

use std::sync::Arc;

use kotone_music::{MemorySessionStore, MusicProvider, NeteaseProvider, Playable, Quality};

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
