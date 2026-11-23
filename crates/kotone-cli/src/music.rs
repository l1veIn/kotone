//! `kotone-cli music`：网易云音乐的真网络冒烟测试（ADR 034）。不连接桌面实例。
//!
//! 登录状态存在一个文件里（`--session`，缺省 `<KOTONE_HOME>/music/cli-session.json`），和桌面端
//! 存在 Windows 凭据库里的那份分开。
//!
//! 退出码：0 成功；1 失败（网络、要登录、没找到）；2 用法错误。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Subcommand;
use kotone_music::{
    FileSessionStore, MusicProvider, NeteaseProvider, Player, QrPoll, Quality, Status, Track,
};

#[derive(Subcommand, Debug)]
pub enum MusicCommand {
    /// 搜歌（不用登录）
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 一首歌的播放链接（看是不是试听片段）
    Url {
        /// 歌曲 ID（`ne:123` 或 `123`）
        id: String,
        /// standard / exhigh / lossless
        #[arg(long, default_value = "exhigh")]
        quality: String,
        #[arg(long)]
        session: Option<PathBuf>,
    },
    /// 扫码登录：终端里显示二维码，用网易云音乐 App 扫，登录状态存进 --session 文件
    Login {
        #[arg(long)]
        session: Option<PathBuf>,
        /// 最多等多少秒
        #[arg(long, default_value_t = 180)]
        timeout: u64,
    },
    /// 登录的账号
    Whoami {
        #[arg(long)]
        session: Option<PathBuf>,
    },
    /// 每日推荐（要登录）
    Daily {
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 我的歌单（要登录）
    Playlists {
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 搜歌直接放几秒（本机默认扬声器）
    Play {
        query: String,
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// 音量 0–100
        #[arg(long, default_value_t = 20)]
        volume: u8,
        #[arg(long)]
        session: Option<PathBuf>,
    },
    /// 退出登录（删掉 --session 文件里的 Cookie）
    Logout {
        #[arg(long)]
        session: Option<PathBuf>,
    },
}

fn provider(session: Option<PathBuf>) -> NeteaseProvider {
    let path = session.unwrap_or_else(|| {
        kotone_core::settings::kotone_dir()
            .join("music")
            .join("cli-session.json")
    });
    NeteaseProvider::new(Arc::new(FileSessionStore(path)))
}

fn print_tracks(tracks: &[Track], json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(tracks).unwrap_or_default()
        );
        return;
    }
    for (i, t) in tracks.iter().enumerate() {
        let reason = t
            .reason
            .as_deref()
            .map(|r| format!("  [{r}]"))
            .unwrap_or_default();
        println!(
            "{:>3}. {:<16} {} ({}:{:02}){reason}",
            i + 1,
            t.id,
            t.title(),
            t.duration_ms / 60_000,
            t.duration_ms / 1000 % 60
        );
    }
}

fn fail(error: impl std::fmt::Display) -> i32 {
    eprintln!("{error}");
    1
}

pub async fn run(command: MusicCommand) -> i32 {
    match command {
        MusicCommand::Search {
            query,
            limit,
            session,
            json,
        } => match provider(session).search(&query, 0, limit).await {
            Ok((tracks, more)) => {
                print_tracks(&tracks, json);
                if !json && more {
                    println!("（还有更多）");
                }
                0
            }
            Err(e) => fail(e),
        },
        MusicCommand::Url {
            id,
            quality,
            session,
        } => {
            let quality = match quality.as_str() {
                "standard" => Quality::Standard,
                "exhigh" => Quality::Exhigh,
                "lossless" => Quality::Lossless,
                other => {
                    eprintln!("不认识的音质 {other}（standard / exhigh / lossless）");
                    return 2;
                }
            };
            match provider(session).stream(&id, quality).await {
                Ok(Some(s)) => {
                    println!(
                        "{} kbps{}\n{}",
                        s.bitrate / 1000,
                        if s.trial {
                            "（只是试听片段）"
                        } else {
                            ""
                        },
                        s.url
                    );
                    0
                }
                Ok(None) => fail("拿不到播放链接（可能没有版权）"),
                Err(e) => fail(e),
            }
        }
        MusicCommand::Login { session, timeout } => {
            let provider = provider(session);
            let start = match provider.login_qr_start().await {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            println!("用网易云音乐 App 扫一扫：\n");
            match kotone_music::qr::text(&start.content) {
                Ok(text) => println!("{text}"),
                Err(e) => println!("（画不出二维码：{e}）"),
            }
            println!("{}\n", start.content);
            let deadline = Instant::now() + Duration::from_secs(timeout);
            let mut last = String::new();
            while Instant::now() < deadline {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let poll = match provider.login_qr_poll(&start.key).await {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let now = match &poll {
                    QrPoll::Waiting => "等扫码…".to_string(),
                    QrPoll::Scanned => "扫了，在手机上确认…".to_string(),
                    QrPoll::Expired => return fail("二维码过期了，再运行一次"),
                    QrPoll::Failed(m) => return fail(m),
                    QrPoll::Ok(account) => {
                        println!(
                            "登录成功：{}（{}）",
                            account.nickname,
                            if account.vip { "VIP" } else { "非 VIP" }
                        );
                        return 0;
                    }
                };
                if now != last {
                    println!("{now}");
                    last = now;
                }
            }
            fail("等太久了，没扫码")
        }
        MusicCommand::Whoami { session } => match provider(session).account().await {
            Ok(Some(a)) => {
                println!(
                    "{}（id {}，{}）",
                    a.nickname,
                    a.id,
                    if a.vip { "VIP" } else { "非 VIP" }
                );
                0
            }
            Ok(None) => fail("没登录（先 kotone-cli music login）"),
            Err(e) => fail(e),
        },
        MusicCommand::Daily { session, json } => match provider(session).daily().await {
            Ok(tracks) => {
                print_tracks(&tracks, json);
                0
            }
            Err(e) => fail(e),
        },
        MusicCommand::Playlists { session, json } => match provider(session).playlists().await {
            Ok(lists) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&lists).unwrap_or_default()
                    );
                } else {
                    for p in &lists {
                        println!(
                            "{:<16} {}（{} 首）{}{}",
                            p.id,
                            p.name,
                            p.track_count,
                            if p.liked { " ❤" } else { "" },
                            if p.mine { "" } else { " 收藏" }
                        );
                    }
                }
                0
            }
            Err(e) => fail(e),
        },
        MusicCommand::Play {
            query,
            seconds,
            volume,
            session,
        } => {
            let provider: Arc<dyn MusicProvider> = Arc::new(provider(session));
            let player = Player::new(provider, Arc::new(|_| {}));
            player.set_volume(volume.min(100));
            let started = Instant::now();
            let outcome = match player.play_query(&query).await {
                Ok(o) => o,
                Err(e) => return fail(e),
            };
            for (t, reason) in &outcome.skipped {
                println!("跳过 {}：{reason}", t.title());
            }
            let Some(track) = outcome.started else {
                return fail("一首都放不了");
            };
            println!("放：{}（来源 {}）", track.title(), query);
            let deadline = Instant::now() + Duration::from_secs(seconds + 15);
            let mut sounded = None;
            loop {
                let s = player.state();
                if s.status == Status::Error {
                    return fail(s.error.unwrap_or_else(|| "放的时候出错了".into()));
                }
                if s.status == Status::Playing && s.position_ms > 0 && sounded.is_none() {
                    sounded = Some(started.elapsed());
                    println!("出声：{} ms", started.elapsed().as_millis());
                }
                if sounded.is_some_and(|at| started.elapsed() >= at + Duration::from_secs(seconds))
                {
                    println!("放到 {} ms，停", s.position_ms);
                    break;
                }
                if Instant::now() >= deadline {
                    return fail(format!("{seconds} 秒内没放起来：{s:?}"));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            player.stop();
            0
        }
        MusicCommand::Logout { session } => match provider(session).logout().await {
            Ok(()) => {
                println!("已退出");
                0
            }
            Err(e) => fail(e),
        },
    }
}
