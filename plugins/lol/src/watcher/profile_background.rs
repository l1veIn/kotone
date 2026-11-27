//! 生涯主页的原画。按正在显示的 PUUID 读取，切换账号时资料与背景始终属于同一人。

use serde_json::{json, Value};

use crate::client::{abilities, assets, Lcu, LolError};

pub(super) async fn resolve(lcu: &Lcu, puuid: &str) -> Option<String> {
    let puuid = super::encode(puuid);
    let profile = match lcu
        .get(&format!("/lol-summoner/v1/summoner-profile?puuid={puuid}"))
        .await
    {
        Ok(profile) => profile,
        Err(LolError::Status(404, _)) => Value::Null,
        Err(_) => return None,
    };
    let selected = profile.get("backgroundSkinId").and_then(|id| {
        id.as_i64()
            .or_else(|| id.as_str()?.parse().ok())
            .filter(|id| *id > 0)
    });
    let champion_id = match selected {
        Some(skin_id) => skin_id / 1000,
        None => {
            let mastery = lcu
                .post(
                    &format!("/lol-champion-mastery/v1/{puuid}/champion-mastery/top?count=1"),
                    &json!({"skipCache": false}),
                )
                .await
                .ok()?;
            mastery["masteries"].get(0)?["championId"].as_i64()?
        }
    };
    if champion_id <= 0 {
        return None;
    }
    let champion = lcu.get(&abilities::client_path(champion_id)).await.ok()?;
    splash_path(&champion, champion_id, selected)
}

fn splash_path(champion: &Value, champion_id: i64, selected: Option<i64>) -> Option<String> {
    let skins = champion["skins"].as_array()?;
    let skin = match selected {
        Some(id) => skins.iter().find_map(|skin| {
            if skin["id"].as_i64() == Some(id) {
                return Some(skin);
            }
            skin.pointer("/questSkinInfo/tiers")?
                .as_array()?
                .iter()
                .find(|tier| tier["id"].as_i64() == Some(id))
        }),
        None => {
            let base_id = champion_id.checked_mul(1000)?;
            skins.iter().find(|skin| {
                skin["id"].as_i64() == Some(base_id) || skin["isBase"].as_bool() == Some(true)
            })
        }
    }?;
    // 横幅优先使用保留完整构图的原画。部分皮肤（包括进阶形态）只提供居中版本。
    ["uncenteredSplashPath", "splashPath"]
        .iter()
        .find_map(|key| assets::normalize(skin.get(*key)?.as_str()?).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_selected_skin_and_quest_tier_and_defaults_to_base_skin() {
        let base = "/lol-game-data/assets/ASSETS/test/base.jpg";
        let selected = "/lol-game-data/assets/ASSETS/test/selected.jpg";
        let tier = "/lol-game-data/assets/ASSETS/test/tier.jpg";
        let centered = "/lol-game-data/assets/ASSETS/test/centered.jpg";
        let champion = json!({"skins": [
            {"id": 39001, "splashPath": centered, "uncenteredSplashPath": selected, "questSkinInfo": {
                "tiers": [
                    {"id": 39003, "uncenteredSplashPath": tier, "splashPath": centered},
                    {"id": 39004, "splashPath": centered}
                ]
            }},
            {"id": 39005, "uncenteredSplashPath": "../bad.jpg", "splashPath": centered},
            {"id": 39000, "uncenteredSplashPath": base, "splashPath": centered}
        ]});
        assert_eq!(
            splash_path(&champion, 39, Some(39001)).as_deref(),
            Some(selected)
        );
        assert_eq!(
            splash_path(&champion, 39, Some(39003)).as_deref(),
            Some(tier)
        );
        assert_eq!(splash_path(&champion, 39, None).as_deref(), Some(base));
        assert_eq!(splash_path(&champion, 39, Some(39999)), None);
        for id in [39004, 39005] {
            assert_eq!(
                splash_path(&champion, 39, Some(id)).as_deref(),
                Some(centered)
            );
        }
    }
}
