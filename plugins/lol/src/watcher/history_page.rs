//! 主页、战绩页的分页战绩（`lol.history`）。客户端按人缓存第一次被请求的那一段战绩，之后不管要哪一段都给那一份
//! （[`paths::matches`]），所以每一页都要同一段（最近 [`paths::MATCH_WINDOW`] 局），插件自己切页：`offset` 是第几局
//! （只数列出来的，重开、自定义不算）。客户端给的比要的少（别的工具先要过小的一段）时照给的用，最后一页写清客户端给了几局
//! （`clientGames`）。

use super::{encode, status, str_arg};
use crate::client::history::{parse_match_list, GameRecord};
use crate::client::http::paths;
use crate::client::Lcu;
use crate::{pages, LolPlugin};
use kotone_plugin_api::{Host, ToolResult};
use serde_json::{json, Value};

impl LolPlugin {
    pub(super) async fn history_page_tool(&self, host: &dyn Host, arguments: &Value) -> ToolResult {
        let Some(puuid) = str_arg(arguments, "puuid").filter(|id| id.len() <= 100) else {
            return ToolResult::content(status("error", "请提供要查看战绩的玩家 PUUID"));
        };
        let Some(offset) = arguments["offset"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
        else {
            return ToolResult::content(status("error", "战绩起始位置无效"));
        };
        let Some(page_size @ (10 | 20)) = arguments["pageSize"].as_u64() else {
            return ToolResult::content(status("error", "每页可选 10 或 20 局"));
        };
        let lcu = match self.lcu_or_closed(host) {
            Ok(lcu) => lcu,
            Err(result) => return ToolResult::content(result),
        };
        let page = match read_page(&lcu, puuid, offset, page_size as usize).await {
            Ok(page) => page,
            Err(message) => return ToolResult::content(status("error", message)),
        };
        let data = self.ensure_data().await;
        ToolResult {
            content: json!({"status": "ok"}),
            structured_content: Some(json!({
                "puuid": puuid, "offset": offset, "pageSize": page_size,
                "games": page.games.iter().map(|g| pages::game_json(g, data.as_deref())).collect::<Vec<_>>(),
                "hasMore": page.next.is_some(), "nextOffset": page.next,
                "clientGames": page.client_games,
            })),
            ui: None,
        }
    }
}

/// 一页：这几局、下一页从第几局开始（没有了是 None）、客户端一共给了几局记录（重开、自定义也算）。
#[derive(Debug, PartialEq)]
struct Page {
    games: Vec<GameRecord>,
    next: Option<usize>,
    client_games: usize,
}

async fn read_page(
    lcu: &Lcu,
    puuid: &str,
    offset: usize,
    page_size: usize,
) -> Result<Page, String> {
    let response = lcu
        .get(&paths::matches(&encode(puuid)))
        .await
        .map_err(|error| format!("读取近期战绩失败：{error}"))?;
    page_of(&response, puuid, offset, page_size)
}

/// 从客户端给的那一份切出一页。范围和要的不一样（`gameIndexBegin`、条数）不算错，照给的用。
fn page_of(response: &Value, puuid: &str, offset: usize, page_size: usize) -> Result<Page, String> {
    let rows = response["games"]["games"]
        .as_array()
        .ok_or("客户端返回的战绩列表格式有误")?;
    if rows.iter().any(|raw| {
        raw.pointer("/participantIdentities/0/player/puuid")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && id != puuid)
    }) {
        return Err("客户端返回了其他玩家的战绩".into());
    }
    let mut games: Vec<GameRecord> = parse_match_list(response)
        .into_iter()
        .filter(|g| g.game_id > 0)
        .collect();
    let end = offset.saturating_add(page_size).min(games.len());
    let next = (end < games.len()).then_some(end);
    let games = if offset < end {
        games.drain(offset..end).collect()
    } else {
        Vec::new()
    };
    Ok(Page {
        games,
        next,
        client_games: rows.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::tests::game;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 假客户端：每个人有 `total` 局战绩（第 3 局是重开的），**按人缓存第一次被请求的那一段**，之后不管要哪一段都给那一份
    /// （国服客户端 2026-11-27 实测的样子）。`seen` 记下每次要的路径。
    #[derive(Clone, Default)]
    struct Fake {
        total: usize,
        cached: Arc<Mutex<HashMap<String, (usize, usize)>>>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Fake {
        fn reply(&self, path: &str) -> (u16, Value) {
            self.seen.lock().unwrap().push(path.to_string());
            let Some(rest) = path.strip_prefix("/lol-match-history/v1/products/lol/") else {
                return (404, json!({"message": "no such path"}));
            };
            let (puuid, query) = rest.split_once("/matches?").unwrap();
            let arg = |key: &str| -> usize {
                query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
                    .unwrap()
                    .parse()
                    .unwrap()
            };
            let (begin, end) = *self
                .cached
                .lock()
                .unwrap()
                .entry(puuid.to_string())
                .or_insert((arg("begIndex"), arg("endIndex")));
            let rows: Vec<Value> = (begin..=end.min(self.total.saturating_sub(1)))
                .filter(|_| self.total > 0)
                .map(|i| {
                    let mut g = game(i as i64, 2400, 39, i % 2 == 0, (1, 2, 3), 1000);
                    if i == 3 {
                        g["gameDuration"] = json!(120);
                    }
                    g["participantIdentities"][0]["player"]["puuid"] = json!(puuid);
                    g
                })
                .collect();
            (
                200,
                json!({"accountId": 1, "platformId": "HN10", "games": {
                    "gameBeginDate": "", "gameEndDate": "", "gameCount": rows.len(),
                    "gameIndexBegin": begin, "gameIndexEnd": end, "games": rows,
                }}),
            )
        }

        /// 别的地方（比如别的工具）先要过 `begin..=end`。
        fn primed(self, puuid: &str, begin: usize, end: usize) -> Self {
            self.cached
                .lock()
                .unwrap()
                .insert(puuid.to_string(), (begin, end));
            self
        }
    }

    async fn serve(fake: Fake) -> Lcu {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let fake = fake.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&buf).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let (code, value) = fake.reply(&path);
                    let data = value.to_string();
                    let head = format!(
                        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(data.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Lcu::with_base(&format!("http://127.0.0.1:{port}"), "tok")
    }

    fn ids(page: &Page) -> Vec<i64> {
        page.games.iter().map(|g| g.game_id).collect()
    }

    /// 翻完所有页：每页的局 id、客户端给了几局。
    async fn all_pages(lcu: &Lcu, puuid: &str, size: usize) -> (Vec<Vec<i64>>, usize) {
        let (mut pages, mut offset) = (Vec::new(), Some(0));
        let mut client = 0;
        while let Some(at) = offset {
            let page = read_page(lcu, puuid, at, size).await.unwrap();
            pages.push(ids(&page));
            client = page.client_games;
            offset = page.next;
            assert!(pages.len() < 20, "翻不完");
        }
        (pages, client)
    }

    #[tokio::test]
    async fn my_own_history_pages_through_the_whole_window_at_10_and_20_a_page() {
        let fake = Fake {
            total: 120,
            ..Fake::default()
        };
        let lcu = serve(fake.clone()).await;
        // 每页 10 局：第 1、2、3 页接着往下，不重复；第 3 局（id 997）是重开的，不列
        let first = read_page(&lcu, "me", 0, 10).await.unwrap();
        assert_eq!(first.games.len(), 10);
        assert!(!ids(&first).contains(&997));
        let second = read_page(&lcu, "me", first.next.unwrap(), 10)
            .await
            .unwrap();
        let third = read_page(&lcu, "me", second.next.unwrap(), 10)
            .await
            .unwrap();
        assert_eq!(ids(&second)[0], ids(&first)[9] - 1);
        assert_eq!(ids(&third)[0], ids(&second)[9] - 1);
        // 一共 50 局记录（一局重开）：每页 10 局 5 页（最后一页 9 局），每页 20 局 3 页
        let (pages, client) = all_pages(&lcu, "me", 10).await;
        assert_eq!(
            pages.iter().map(Vec::len).collect::<Vec<_>>(),
            [10, 10, 10, 10, 9]
        );
        assert_eq!(client, 50);
        let (pages, _) = all_pages(&lcu, "me", 20).await;
        assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [20, 20, 9]);
        // 每次都要同一段
        let seen = fake.seen.lock().unwrap();
        assert!(seen
            .iter()
            .all(|p| p == "/lol-match-history/v1/products/lol/me/matches?begIndex=0&endIndex=49"));
    }

    #[tokio::test]
    async fn a_player_the_scout_looked_up_first_still_pages() {
        let fake = Fake {
            total: 120,
            ..Fake::default()
        };
        let lcu = serve(fake.clone()).await;
        // 进加载界面时查过这个人（lol.scout 那一份）：要的是同一段，主页、战绩页照样翻得到 50 局
        let scouted = super::super::fetch_history(&lcu, "enemy").await;
        assert_eq!(scouted.games.len(), 20, "她看的还是最近 20 局");
        let (pages, client) = all_pages(&lcu, "enemy", 10).await;
        assert_eq!(pages.len(), 5);
        assert_eq!(client, 50);
    }

    #[tokio::test]
    async fn when_the_client_cached_a_smaller_range_first_it_uses_what_it_got() {
        // 别的工具先要过 0..19（或者 Kotone 上一版的 lol.scout）：客户端只给这 20 局，不报错，翻到头写清给了 20 局
        let fake = Fake {
            total: 120,
            ..Fake::default()
        }
        .primed("other", 0, 19);
        let lcu = serve(fake).await;
        let (pages, client) = all_pages(&lcu, "other", 10).await;
        assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [10, 9]);
        assert_eq!(client, 20);
        // 先要过的是后面的一段（10..20，起点不是 0）：也照给的用
        let fake = Fake {
            total: 120,
            ..Fake::default()
        }
        .primed("late", 10, 20);
        let lcu = serve(fake).await;
        let page = read_page(&lcu, "late", 0, 20).await.unwrap();
        assert_eq!(
            (page.games.len(), page.next, page.client_games),
            (11, None, 11)
        );
        assert_eq!(ids(&page)[0], 990);
    }

    #[test]
    fn pages_past_the_end_are_empty_and_other_players_games_are_refused() {
        let rows: Vec<Value> = (0..5)
            .map(|i| game(i, 420, 39, true, (1, 2, 3), 100))
            .collect();
        let list = json!({"games": {"gameIndexBegin": 0, "games": rows}});
        assert_eq!(
            page_of(&list, "p", 10, 10),
            Ok(Page {
                games: vec![],
                next: None,
                client_games: 5
            })
        );
        assert_eq!(page_of(&list, "p", 0, 10).unwrap().games.len(), 5);
        assert!(page_of(&list, "another player", 0, 10).is_err());
        assert!(page_of(&json!({"httpStatus": 404}), "p", 0, 10).is_err());
    }
}
