//! 用 `scripts/manual-e2e/lol_probe.py` 录下来的真实数据，离线跑一遍三个工具会给她的文字（ADR 035 实测用）。
//!
//! ```text
//! cargo run -p kotone-lol --example render_probe -- target/lol-probe/<时间> target/lol-probe/gamedata
//! ```
//! 第一个目录是一次探测（局内），第二个是客户端的游戏资料（`/lol-game-data/assets/v1/*.json`）。
//! 输出里有玩家名，只在本机看，不要提交。

use std::collections::HashMap;
use std::path::Path;

use kotone_lol::history::{self, PlayerHistory};
use kotone_lol::live;
use kotone_lol::spells::{self, SpellTracker, SpellUse};
use kotone_lol::text::{self, ScoutPlayer};
use kotone_lol::GameData;
use serde_json::Value;

fn read(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let probe = Path::new(args.get(1).expect("探测目录"));
    let assets = Path::new(args.get(2).expect("游戏资料目录"));
    let data = GameData::from_json(
        &read(&assets.join("champion-summary.json")),
        &read(&assets.join("summoner-spells.json")),
        &read(&assets.join("items.json")),
        &read(&assets.join("cherry-augments.json")),
        &read(&assets.join("queues.json")),
    );

    // lol.match：局内接口
    let game = live::parse_all_game_data(&read(&probe.join("live-allgamedata.json")));
    let mut tracker = SpellTracker::default();
    if let Some(game) = &game {
        // 假装玩家说了“对面第一个人闪了”
        if let Some(p) = game
            .enemies()
            .find(|p| p.spells.iter().any(|s| s == "闪现"))
        {
            let base = data
                .spell("闪现", &game.mode)
                .map(|s| s.cooldown)
                .unwrap_or(300.0);
            let (ready, notes) = spells::ready_time(base, game.time, &game.mode, &p.items);
            let entry = SpellUse {
                alias: p.alias.clone(),
                champion: text::champion_names(p, Some(&data)).1,
                spell: "闪现".into(),
                used_at: game.time,
                ready_at: ready,
                notes,
            };
            println!(
                "== lol.spell（假装 {} 刚交了闪）==\n{}  {:?}\n",
                entry.champion,
                spells::chat_line(&entry),
                entry.notes
            );
            tracker.record(entry);
        }
        println!(
            "== lol.match ==\n{}\n",
            text::render_match(game, Some(&data), &tracker)
        );
    } else {
        println!("（这次探测没有局内数据）\n");
    }

    // lol.scout：客户端接口
    let session = read(&probe.join("lcu-gameflow-session.json"));
    let me = read(&probe.join("lcu-current-summoner.json"))["puuid"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let mut histories: HashMap<String, PlayerHistory> = HashMap::new();
    for entry in std::fs::read_dir(probe).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("scout-") {
            continue;
        }
        let v = read(&entry.path());
        let puuid = v["summoner"]["puuid"].as_str().unwrap_or("").to_string();
        let mut h = PlayerHistory {
            privacy: v["summoner"]["privacy"].as_str().unwrap_or("").to_string(),
            games: history::parse_match_list(&v["history"]),
            ..PlayerHistory::default()
        };
        h.riot_id = history::riot_id_from_match_list(&v["history"]).unwrap_or_default();
        histories.insert(puuid, h);
    }
    let gd = &session["gameData"];
    let queue = gd["queue"]["id"].as_i64().unwrap_or(0);
    let mut players = Vec::new();
    let mut my_team = 1;
    for (team, key) in [(1, "teamOne"), (2, "teamTwo")] {
        for m in gd[key].as_array().into_iter().flatten() {
            if m["puuid"].as_str() == Some(me.as_str()) {
                my_team = team;
            }
        }
    }
    for (team, key) in [(1, "teamOne"), (2, "teamTwo")] {
        for m in gd[key].as_array().into_iter().flatten() {
            let puuid = m["puuid"].as_str().unwrap_or("");
            let id = m["championId"].as_i64().unwrap_or(0);
            let c = data.champion(id);
            let history = histories.get(puuid).cloned();
            players.push(ScoutPlayer {
                champion: c.map(|c| c.label()).unwrap_or_default(),
                alias: c.map(|c| c.alias.clone()).unwrap_or_default(),
                short: c.map(|c| c.short().to_string()).unwrap_or_default(),
                champion_id: id,
                name: history
                    .as_ref()
                    .map(|h| h.riot_id.split('#').next().unwrap_or("").to_string())
                    .unwrap_or_default(),
                ally: team == my_team,
                me: puuid == me,
                party: m["teamParticipantId"].as_i64(),
                history,
            });
        }
    }
    let mode = data.queue_name(queue).unwrap_or("").to_string();
    let random = kotone_lol::gamedata::random_pick(Some(&data), queue);
    println!(
        "== lol.scout ==\n{}",
        text::render_scout(&players, queue, &mode, random, None)
    );
}
