//! 主页的历史列表沿用战绩筛选。游标记录 LCU 的原始位置，跳过的重开或自定义对局也占一个位置。

use std::ops::ControlFlow;

use super::{encode, status, str_arg};
use crate::client::history::{parse_match, GameRecord};
use crate::client::http::paths;
use crate::client::{Lcu, LolError};
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
            .and_then(|n| u32::try_from(n).ok())
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
        let (games, next_offset) = match read_page(&lcu, puuid, offset, page_size as u32).await {
            Ok(page) => page,
            Err(message) => return ToolResult::content(status("error", message)),
        };
        let data = self.ensure_data().await;
        ToolResult {
            content: json!({"status": "ok"}),
            structured_content: Some(json!({
                "puuid": puuid, "offset": offset, "pageSize": page_size,
                "games": games.iter().map(|g| pages::game_json(g, data.as_deref())).collect::<Vec<_>>(),
                "hasMore": next_offset.is_some(), "nextOffset": next_offset,
            })),
            ui: None,
        }
    }
}

async fn read_page(
    lcu: &Lcu,
    puuid: &str,
    offset: u32,
    page_size: u32,
) -> Result<(Vec<GameRecord>, Option<u32>), String> {
    let encoded = encode(puuid);
    let mut cursor = offset;
    let mut games = Vec::with_capacity(page_size as usize);
    loop {
        // 最多请求 20 条，另看一条实际可显示的对局再启用「下一页」。遇到连续重开时分批往前找。
        let count = (page_size + 1 - games.len() as u32).clamp(5, 20);
        let end = cursor
            .checked_add(count - 1)
            .ok_or("战绩起始位置超出范围")?;
        let response = match lcu.get(&paths::match_range(&encoded, cursor, end)).await {
            Ok(response) => response,
            Err(LolError::Status(404, _)) if cursor > 0 => return Ok((games, None)),
            Err(error) => return Err(format!("读取近期战绩失败：{error}")),
        };
        match collect_batch(&response, puuid, cursor, count, page_size, &mut games)? {
            ControlFlow::Break(next) => return Ok((games, next)),
            ControlFlow::Continue(next) => cursor = next,
        }
    }
}

fn collect_batch(
    response: &Value,
    puuid: &str,
    start: u32,
    count: u32,
    page_size: u32,
    games: &mut Vec<GameRecord>,
) -> Result<ControlFlow<Option<u32>, u32>, String> {
    let batch = &response["games"];
    let rows = batch["games"]
        .as_array()
        .ok_or("客户端返回的战绩列表格式有误")?;
    if rows.is_empty() {
        return Ok(ControlFlow::Break(None));
    }
    if rows.len() > count as usize
        || batch["gameIndexBegin"]
            .as_u64()
            .is_some_and(|n| n != u64::from(start))
    {
        return Err("客户端返回的战绩范围与请求不符".into());
    }
    for (index, raw) in rows.iter().enumerate() {
        let Some(game) = parse_match(raw).filter(|g| g.game_id > 0) else {
            continue;
        };
        if raw
            .pointer("/participantIdentities/0/player/puuid")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && id != puuid)
        {
            return Err("客户端返回了其他玩家的战绩".into());
        }
        if games.len() == page_size as usize {
            return Ok(ControlFlow::Break(Some(start + index as u32)));
        }
        games.push(game);
    }
    if rows.len() < count as usize {
        return Ok(ControlFlow::Break(None));
    }
    Ok(ControlFlow::Continue(
        start.checked_add(count).ok_or("战绩起始位置超出范围")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::tests::game;

    #[test]
    fn a_full_twenty_needs_another_match_and_skipped_games_advance_the_cursor() {
        let mut games = Vec::new();
        let rows = (0..20)
            .map(|i| game(i, 420, 39, true, (1, 2, 3), 100))
            .collect::<Vec<_>>();
        let batch = json!({"games": {"gameCount": 20, "gameIndexBegin": 0, "games": rows}});
        assert_eq!(
            collect_batch(&batch, "p", 0, 20, 20, &mut games),
            Ok(ControlFlow::Continue(20))
        );
        let mut remake = game(20, 420, 39, false, (0, 0, 0), 0);
        remake["gameDuration"] = json!(90);
        let more = json!({"games": {"gameIndexBegin": 20, "games": [remake, game(21, 420, 99, true, (1,2,3), 100)]}});
        assert_eq!(
            collect_batch(&more, "p", 20, 5, 20, &mut games),
            Ok(ControlFlow::Break(Some(21)))
        );
        assert_eq!(games.len(), 20);
        assert_eq!(
            collect_batch(&json!({"games":{"games":[]}}), "p", 20, 5, 20, &mut games),
            Ok(ControlFlow::Break(None))
        );
        assert!(collect_batch(&batch, "p", 20, 20, 20, &mut Vec::new()).is_err());
        assert!(collect_batch(&batch, "another player", 0, 20, 20, &mut Vec::new()).is_err());
    }
}
