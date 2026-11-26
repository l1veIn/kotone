//! 用 `scripts/manual-e2e/lol_probe.py` 录下来的真实数据，离线跑一遍三个工具会给她的文字（ADR 035 实测用）。
//!
//! ```text
//! cargo run -p kotone-plugin-lol --example render_probe -- target/lol-probe/<时间> target/lol-probe/gamedata [页面样例目录]
//! ```
//! 第一个目录是一次探测（局内），第二个是客户端的游戏资料（`/lol-game-data/assets/v1/*.json`）。
//! 输出里有玩家名，只在本机看，不要提交。
//!
//! 写了第三个目录时，再把三个页面的数据（`structuredContent`：这局、战绩、单局详情）存成
//! `match-sample.json`、`player-sample.json`、`game-sample.json`，给做页面的人对照。这三份去掉了玩家名和 puuid
//! （换成“玩家 1”“puuid-1”……）。

use std::collections::HashMap;
use std::path::Path;

use kotone_plugin_lol::client::history::{self, PlayerHistory};
use kotone_plugin_lol::client::live;
use kotone_plugin_lol::client::spells::{self, SpellTracker, SpellUse};
use kotone_plugin_lol::client::text::{self, ScoutPlayer};
use kotone_plugin_lol::client::GameData;
use kotone_plugin_lol::pages;
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
    let samples = args.get(3).map(Path::new);
    let mut data = GameData::from_json(
        &read(&assets.join("champion-summary.json")),
        &read(&assets.join("summoner-spells.json")),
        &read(&assets.join("items.json")),
        &read(&assets.join("cherry-augments.json")),
        &read(&assets.join("queues.json")),
    );
    data.add_perks(
        &read(&assets.join("perks.json")),
        &read(&assets.join("perkstyles.json")),
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
            profile_icon_id: v["summoner"]["profileIconId"].as_i64(),
            summoner_level: v["summoner"]["summonerLevel"].as_i64().unwrap_or(0),
            ..PlayerHistory::default()
        };
        h.riot_id = history::riot_id_from_match_list(&v["history"]).unwrap_or_default();
        histories.insert(puuid, h);
    }
    let gd = &session["gameData"];
    let queue = gd["queue"]["id"].as_i64().unwrap_or(0);
    let selections: Vec<&Value> = gd["playerChampionSelections"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
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
            let pick = selections
                .iter()
                .find(|x| x["puuid"].as_str() == Some(puuid));
            let spell = |key: &str| pick.and_then(|x| x[key].as_i64()).unwrap_or(0);
            let position = m["selectedPosition"].as_str().unwrap_or("");
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
                profile_icon_id: m["profileIconId"].as_i64(),
                spells: [spell("spell1Id"), spell("spell2Id")],
                position: if position == "NONE" {
                    String::new()
                } else {
                    position.to_string()
                },
                // 没有 puuid 的真人是隐藏了身份（ADR 043 D9）
                identity: if puuid.is_empty() {
                    kotone_plugin_lol::client::text::Identity::Hidden
                } else {
                    kotone_plugin_lol::client::text::Identity::Known
                },
            });
        }
    }
    let mode = data.queue_name(queue).unwrap_or("").to_string();
    let random = kotone_plugin_lol::client::gamedata::random_pick(Some(&data), queue);
    println!(
        "== lol.scout ==\n{}",
        text::render_scout(&players, queue, &mode, random, None)
    );

    // lol.player：每个人给她的几行字
    println!("\n== lol.player ==");
    for p in &players {
        if let Some(h) = &p.history {
            println!("{}\n", pages::player_text(h, Some(&data)));
        }
    }

    if let Some(out) = samples {
        let sample = Sample {
            session: &session,
            probe,
            data: &data,
            queue,
            mode: &mode,
            random,
        };
        sample.write(out, &mut players);
    }
}

/// 三个页面的数据（去掉玩家名和 puuid）要用的东西。
struct Sample<'a> {
    session: &'a Value,
    probe: &'a Path,
    data: &'a GameData,
    queue: i64,
    mode: &'a str,
    random: bool,
}

impl Sample<'_> {
    fn write(&self, out: &Path, players: &mut [ScoutPlayer]) {
        std::fs::create_dir_all(out).expect("页面样例目录");
        let mut puuids = Vec::new();
        for (n, p) in players.iter_mut().enumerate() {
            let n = n + 1;
            p.name = format!("玩家 {n}");
            if let Some(h) = p.history.as_mut() {
                h.riot_id = format!("玩家 {n}#0000");
            }
            puuids.push(format!("puuid-{n}"));
        }
        let game_id = self.session["gameData"]["gameId"].as_i64().unwrap_or(0);
        let save = |name: &str, v: &Value| {
            let path = out.join(name);
            std::fs::write(&path, serde_json::to_string_pretty(v).unwrap()).unwrap();
            println!("写了 {}", path.display());
        };
        let data = Some(self.data);
        save(
            "match-sample.json",
            &pages::match_page(
                players,
                &puuids,
                game_id,
                self.queue,
                self.mode,
                self.random,
                data,
            ),
        );
        if let Some(i) = players.iter().position(|p| p.me) {
            if let Some(h) = &players[i].history {
                save(
                    "player-sample.json",
                    &pages::player_page(&puuids[i], h, self.queue, data),
                );
            }
        }
        let mut detail = read(&self.probe.join("lcu-game-detail.json"));
        if let Some(ids) = detail["participantIdentities"].as_array_mut() {
            for (n, x) in ids.iter_mut().enumerate() {
                let n = n + 1;
                let player = &mut x["player"];
                player["gameName"] = format!("玩家 {n}").into();
                player["tagLine"] = "0000".into();
                player["summonerName"] = "".into();
                player["puuid"] = format!("puuid-{n}").into();
                for key in [
                    "accountId",
                    "currentAccountId",
                    "summonerId",
                    "matchHistoryUri",
                ] {
                    player[key] = Value::Null;
                }
            }
        }
        if let Some(page) = pages::game_page(&detail, data) {
            save("game-sample.json", &page);
        }
    }
}
