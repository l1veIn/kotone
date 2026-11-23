//! 把网易云接口返回的 JSON 变成 [`Track`] / [`Playlist`] / [`Account`] / [`StreamInfo`]。
//!
//! 能不能完整地放照 YesPlayMusic `src/utils/common.js` 的 `isTrackPlayable`
//! （https://github.com/qier222/YesPlayMusic，MIT，Copyright (c) 2020-2023 qier222）：
//! `privilege.pl > 0` 能放；登录了且在自己的云盘里能放；`fee == 1` 要 VIP（VIP 账号能放）；
//! `fee == 4` 付费专辑；有 `noCopyrightRcmd` 没版权；登录了且 `privilege.st < 0` 已下架。

use serde_json::Value;

use crate::{Account, Playable, Playlist, StreamInfo, Track};

/// 判断能不能放要知道的账号情况。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Ctx {
    pub logged_in: bool,
    pub vip: bool,
}

pub(crate) const PREFIX: &str = "ne";

pub(crate) fn id_of(raw: u64) -> String {
    format!("{PREFIX}:{raw}")
}

fn u64_of(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn i64_of(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn str_of(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string).filter(|s| !s.is_empty())
}

/// 一首歌；`privilege` 是另外给的（歌单、歌曲详情里在 `privileges` 数组里），没给就用歌里自带的。
pub(crate) fn track(song: &Value, privilege: Option<&Value>, ctx: Ctx) -> Option<Track> {
    let raw = u64_of(&song["id"])?;
    let privilege = privilege
        .or_else(|| song.get("privilege").filter(|p| p.is_object()))
        .cloned()
        .unwrap_or(Value::Null);
    let artists_value = if song["ar"].is_array() {
        &song["ar"]
    } else {
        &song["artists"]
    };
    let artists = artists_value
        .as_array()
        .map(|a| a.iter().filter_map(|x| str_of(&x["name"])).collect())
        .unwrap_or_default();
    let album = if song["al"].is_object() {
        &song["al"]
    } else {
        &song["album"]
    };
    let duration = u64_of(&song["dt"])
        .or_else(|| u64_of(&song["duration"]))
        .unwrap_or(0);
    let (playable, reason) = playability(song, &privilege, ctx);
    Some(Track {
        id: id_of(raw),
        name: str_of(&song["name"]).unwrap_or_default(),
        artists,
        album: str_of(&album["name"]).unwrap_or_default(),
        cover_url: str_of(&album["picUrl"]),
        duration_ms: duration,
        playable,
        reason,
    })
}

pub(crate) fn playability(song: &Value, privilege: &Value, ctx: Ctx) -> (Playable, Option<String>) {
    let fee_of = |v: &Value| i64_of(&v["fee"]);
    if i64_of(&privilege["pl"]).is_some_and(|pl| pl > 0) {
        return (Playable::Full, None);
    }
    if ctx.logged_in && privilege["cs"].as_bool() == Some(true) {
        return (Playable::Full, None);
    }
    let fee = fee_of(song).or_else(|| fee_of(privilege));
    let privilege_fee = fee_of(privilege);
    if fee == Some(1) || privilege_fee == Some(1) {
        return if ctx.logged_in && ctx.vip {
            (Playable::Full, None)
        } else {
            (Playable::Vip, Some("要网易云音乐 VIP".to_string()))
        };
    }
    if fee == Some(4) || privilege_fee == Some(4) {
        return (Playable::Paid, Some("要单独购买的付费专辑".to_string()));
    }
    if !song["noCopyrightRcmd"].is_null() && song.get("noCopyrightRcmd").is_some() {
        return (
            Playable::Unavailable,
            Some("网易云音乐没有版权".to_string()),
        );
    }
    if ctx.logged_in && i64_of(&privilege["st"]).is_some_and(|st| st < 0) {
        return (Playable::Unavailable, Some("已下架".to_string()));
    }
    (Playable::Full, None)
}

/// `songs` 数组配上同一份响应里的 `privileges` 数组（按 id 对上）。
pub(crate) fn tracks(songs: &Value, privileges: Option<&Value>, ctx: Ctx) -> Vec<Track> {
    let privileges: Vec<&Value> = privileges
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    songs
        .as_array()
        .map(|songs| {
            songs
                .iter()
                .filter_map(|song| {
                    let id = u64_of(&song["id"]);
                    let privilege = privileges
                        .iter()
                        .find(|p| id.is_some() && u64_of(&p["id"]) == id)
                        .copied();
                    track(song, privilege, ctx)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 歌单（`/api/user/playlist` 的一项或歌单详情的 `playlist`）。`uid` = 登录的账号，用来判断是不是自己建的。
pub(crate) fn playlist(v: &Value, uid: Option<u64>) -> Option<Playlist> {
    let raw = u64_of(&v["id"])?;
    let creator_id = u64_of(&v["creator"]["userId"]).or_else(|| u64_of(&v["userId"]));
    Some(Playlist {
        id: id_of(raw),
        name: str_of(&v["name"]).unwrap_or_default(),
        cover_url: str_of(&v["coverImgUrl"]),
        track_count: u64_of(&v["trackCount"]).unwrap_or(0) as u32,
        creator: str_of(&v["creator"]["nickname"]).unwrap_or_default(),
        liked: i64_of(&v["specialType"]) == Some(5),
        mine: uid.is_some() && creator_id == uid,
    })
}

/// `/api/nuser/account/get`：`profile` 为空就是没登录（或登录过期）。
pub(crate) fn account(v: &Value) -> Option<Account> {
    let profile = v.get("profile").filter(|p| p.is_object())?;
    let id = u64_of(&profile["userId"]).or_else(|| u64_of(&v["account"]["id"]))?;
    let vip_type = i64_of(&profile["vipType"])
        .filter(|t| *t != 0)
        .or_else(|| i64_of(&v["account"]["vipType"]))
        .unwrap_or(0);
    Some(Account {
        id: id.to_string(),
        nickname: str_of(&profile["nickname"]).unwrap_or_default(),
        avatar_url: str_of(&profile["avatarUrl"]),
        vip: vip_type != 0,
    })
}

/// 播放链接接口的 `data[0]`：没有 `url` 就是放不了；带 `freeTrialInfo` 是试听片段。
pub(crate) fn stream(item: &Value) -> Option<StreamInfo> {
    let url = str_of(&item["url"])?;
    Some(StreamInfo {
        url,
        bitrate: u64_of(&item["br"]).unwrap_or(0) as u32,
        trial: item.get("freeTrialInfo").is_some_and(|t| !t.is_null()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GUEST: Ctx = Ctx {
        logged_in: false,
        vip: false,
    };
    const MEMBER: Ctx = Ctx {
        logged_in: true,
        vip: false,
    };
    const VIP: Ctx = Ctx {
        logged_in: true,
        vip: true,
    };

    fn song(fee: i64, privilege: Value) -> Value {
        json!({"id": 1901371647u64, "name": "孤勇者", "ar": [{"name": "陈奕迅"}],
               "al": {"name": "孤勇者", "picUrl": "http://p1.music.126.net/x.jpg"}, "dt": 256_000,
               "fee": fee, "privilege": privilege})
    }

    #[test]
    fn a_search_result_becomes_a_track() {
        let t = track(
            &song(8, json!({"pl": 128000, "fee": 8, "st": 0})),
            None,
            GUEST,
        )
        .unwrap();
        assert_eq!(t.id, "ne:1901371647");
        assert_eq!(t.title(), "孤勇者 - 陈奕迅");
        assert_eq!(
            t.cover_url.as_deref(),
            Some("http://p1.music.126.net/x.jpg")
        );
        assert_eq!(t.duration_ms, 256_000);
        assert_eq!(t.playable, Playable::Full);
        // 老格式（artists / album / duration）
        let old = json!({"id": 2, "name": "a", "artists": [{"name": "b"}], "album": {"name": "c"}, "duration": 5});
        let t = track(&old, None, GUEST).unwrap();
        assert_eq!(
            (t.artists[0].as_str(), t.album.as_str(), t.duration_ms),
            ("b", "c", 5)
        );
        assert!(track(&json!({"name": "no id"}), None, GUEST).is_none());
    }

    #[test]
    fn playability_follows_yes_play_music() {
        let vip_song = song(1, json!({"pl": 0, "fee": 1, "st": 0}));
        assert_eq!(
            playability(&vip_song, &vip_song["privilege"], GUEST).0,
            Playable::Vip
        );
        assert_eq!(
            playability(&vip_song, &vip_song["privilege"], MEMBER).0,
            Playable::Vip
        );
        assert_eq!(
            playability(&vip_song, &vip_song["privilege"], VIP).0,
            Playable::Full
        );
        // pl > 0：这个账号能放（哪怕 fee 是 1）
        let owned = song(1, json!({"pl": 320000, "fee": 1}));
        assert_eq!(
            playability(&owned, &owned["privilege"], MEMBER).0,
            Playable::Full
        );
        let paid = song(4, json!({"pl": 0, "fee": 4}));
        let (p, reason) = playability(&paid, &paid["privilege"], VIP);
        assert_eq!(p, Playable::Paid);
        assert!(reason.unwrap().contains("付费专辑"));
        let mut gone = song(0, json!({"pl": 0, "fee": 0, "st": -200}));
        assert_eq!(
            playability(&gone, &gone["privilege"], GUEST).0,
            Playable::Full,
            "st only counts when logged in"
        );
        assert_eq!(
            playability(&gone, &gone["privilege"], MEMBER).0,
            Playable::Unavailable
        );
        gone["noCopyrightRcmd"] = json!({"type": 1});
        let (p, reason) = playability(&gone, &gone["privilege"], GUEST);
        assert_eq!(p, Playable::Unavailable);
        assert!(reason.unwrap().contains("版权"));
        let cloud = song(1, json!({"pl": 0, "fee": 1, "cs": true}));
        assert_eq!(
            playability(&cloud, &cloud["privilege"], MEMBER).0,
            Playable::Full
        );
        let free = song(0, json!({"pl": 0, "fee": 0, "st": 0}));
        assert_eq!(
            playability(&free, &free["privilege"], GUEST),
            (Playable::Full, None)
        );
    }

    #[test]
    fn privileges_are_matched_by_id() {
        let songs = json!([{"id": 1, "name": "a", "fee": 1}, {"id": 2, "name": "b", "fee": 1}]);
        let privileges = json!([{"id": 2, "pl": 320000}, {"id": 1, "pl": 0, "fee": 1}]);
        let t = tracks(&songs, Some(&privileges), GUEST);
        assert_eq!(
            (t[0].playable, t[1].playable),
            (Playable::Vip, Playable::Full)
        );
        assert!(tracks(&json!(null), None, GUEST).is_empty());
    }

    #[test]
    fn playlists_accounts_and_streams() {
        let p = playlist(
            &json!({"id": 7, "name": "我喜欢的音乐", "coverImgUrl": "c", "trackCount": 321,
                                 "specialType": 5, "creator": {"userId": 42, "nickname": "我"}}),
            Some(42),
        )
        .unwrap();
        assert_eq!(
            (p.id.as_str(), p.track_count, p.liked, p.mine),
            ("ne:7", 321, true, true)
        );
        let other = playlist(
            &json!({"id": 8, "name": "x", "specialType": 0, "creator": {"userId": 9}}),
            Some(42),
        )
        .unwrap();
        assert!(!other.liked && !other.mine);
        let a = account(&json!({"account": {"id": 42, "vipType": 0},
                                "profile": {"userId": 42, "nickname": "玩家", "avatarUrl": "a", "vipType": 11}})).unwrap();
        assert_eq!(
            (a.id.as_str(), a.nickname.as_str(), a.vip),
            ("42", "玩家", true)
        );
        assert!(account(&json!({"account": null, "profile": null})).is_none());
        let not_vip =
            account(&json!({"profile": {"userId": 1, "nickname": "n", "vipType": 0}})).unwrap();
        assert!(!not_vip.vip);
        let full = stream(&json!({"id": 1, "url": "http://m8.music.126.net/a.mp3", "br": 320000, "freeTrialInfo": null})).unwrap();
        assert_eq!((full.bitrate, full.trial), (320_000, false));
        let trial = stream(&json!({"url": "http://m8/a.mp3", "br": 128000, "freeTrialInfo": {"start": 0, "end": 30}})).unwrap();
        assert!(trial.trial);
        assert!(stream(&json!({"url": null, "code": 404})).is_none());
    }
}
