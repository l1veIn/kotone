//! 海斗（海克斯大乱斗）英雄 × 海克斯的胜率：[czgreat/aramhex-data](https://github.com/czgreat/aramhex-data)
//! （**CC BY-NC 4.0**：页面上署名、不能商用；运行时下载到本机，不打包进 Kotone 的仓库）。
//!
//! 照它的 AGENTS.md：先读 `latest.json`（`path` 是最新常规数据的目录，`files` 是 SHA-256），再读
//! `<path>/heroes.json`、`hero_augments.json`、`augments.json`；下载后用 SHA-256 校验；样本少的按
//! `(wins + 英雄胜率 × k) / (games + k)` 平滑，k = 300。
//!
//! 打分（照 aramhex：三选一时给三张卡片各一个分）：平滑后的胜率减去这个英雄的平均胜率，换成“比平均高 / 低几个百分点”。

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const REPO: &str = "github.com/czgreat/aramhex-data";
pub const LICENSE: &str = "CC BY-NC 4.0";
pub const RAW: &str = "https://raw.githubusercontent.com/czgreat/aramhex-data/main";
/// 平滑用的 k（AGENTS.md 建议 300 左右）。
pub const SMOOTH_K: f64 = 300.0;

/// `latest.json`。
#[derive(Debug, Clone, PartialEq)]
pub struct Latest {
    pub path: String,
    pub patch: String,
    pub seq: i64,
    pub snapshot_at: String,
    pub matches: i64,
    /// 文件名 → SHA-256。
    pub files: HashMap<String, String>,
}

pub fn parse_latest(v: &Value) -> Option<Latest> {
    let path = v.get("path")?.as_str()?.to_string();
    // 只认 patches/<版本>/<规则> 这样的相对目录，免得被带到别处
    if path.contains("..")
        || path.starts_with('/')
        || !path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_'))
    {
        return None;
    }
    Some(Latest {
        patch: v.get("patch").and_then(Value::as_str).unwrap_or("").into(),
        seq: v.get("seq").and_then(Value::as_i64).unwrap_or(0),
        snapshot_at: v
            .get("snapshot_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        matches: v.get("matches").and_then(Value::as_i64).unwrap_or(0),
        files: v
            .get("files")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, f)| Some((k.clone(), f.get("sha256")?.as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        path,
    })
}

/// 下载的文件和 `latest.json` 里的 SHA-256 对得上吗（没写 SHA-256 的不收）。
pub fn verify(latest: &Latest, file: &str, bytes: &[u8]) -> bool {
    let Some(expected) = latest.files.get(file) else {
        return false;
    };
    let got = Sha256::digest(bytes);
    let hex: String = got.iter().map(|b| format!("{b:02x}")).collect();
    hex.eq_ignore_ascii_case(expected)
}

/// 一个英雄在海斗的总体数据。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hero {
    pub games: i64,
    pub wins: i64,
    pub win_rate: f64,
    pub pick_rate: f64,
    pub rank: i64,
}

/// 一个海克斯。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Augment {
    pub name: String,
    /// silver / gold / prismatic。
    pub rarity: String,
}

/// 英雄 × 海克斯的一格。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub games: i64,
    pub wins: i64,
}

/// 读好的一份数据。
#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub patch: String,
    pub snapshot_at: String,
    pub matches: i64,
    pub heroes: HashMap<i64, Hero>,
    pub augments: HashMap<i64, Augment>,
    pub cells: HashMap<i64, HashMap<i64, Cell>>,
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn f(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn id(key: &str) -> Option<i64> {
    key.parse().ok().filter(|x: &i64| *x > 0)
}

impl Dataset {
    pub fn from_json(latest: &Latest, heroes: &Value, augments: &Value, cells: &Value) -> Self {
        let obj = |v: &Value| v.as_object().cloned().unwrap_or_default();
        Self {
            patch: latest.patch.clone(),
            snapshot_at: latest.snapshot_at.clone(),
            matches: latest.matches,
            heroes: obj(heroes)
                .iter()
                .filter_map(|(k, h)| {
                    Some((
                        id(k)?,
                        Hero {
                            games: i(h, "games"),
                            wins: i(h, "wins"),
                            win_rate: f(h, "win_rate"),
                            pick_rate: f(h, "pick_rate"),
                            rank: i(h, "rank"),
                        },
                    ))
                })
                .collect(),
            augments: obj(augments)
                .iter()
                .filter_map(|(k, a)| {
                    Some((
                        id(k)?,
                        Augment {
                            name: a.get("name")?.as_str()?.to_string(),
                            rarity: a
                                .get("rarity")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        },
                    ))
                })
                .collect(),
            cells: obj(cells)
                .iter()
                .filter_map(|(k, row)| {
                    let row = row.as_object()?;
                    Some((
                        id(k)?,
                        row.iter()
                            .filter_map(|(a, c)| {
                                Some((
                                    id(a)?,
                                    Cell {
                                        games: i(c, "games"),
                                        wins: i(c, "wins"),
                                    },
                                ))
                            })
                            .collect(),
                    ))
                })
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.heroes.is_empty()
    }

    /// 这个英雄拿这个海克斯的分（[`score`]）；没有数据时 None。
    pub fn score(&self, hero: i64, augment: i64) -> Option<AugmentScore> {
        let base = self.heroes.get(&hero)?.win_rate;
        let cell = self.cells.get(&hero)?.get(&augment)?;
        Some(score(base, *cell))
    }

    /// 这个英雄最好的几个海克斯（按分数，样本至少 `min_games` 局）。
    pub fn best_for(&self, hero: i64, limit: usize, min_games: i64) -> Vec<(i64, AugmentScore)> {
        let Some(base) = self.heroes.get(&hero).map(|h| h.win_rate) else {
            return Vec::new();
        };
        let mut out: Vec<(i64, AugmentScore)> = self
            .cells
            .get(&hero)
            .map(|row| {
                row.iter()
                    .filter(|(_, c)| c.games >= min_games)
                    .map(|(a, c)| (*a, score(base, *c)))
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b.1.smoothed.total_cmp(&a.1.smoothed));
        out.truncate(limit);
        out
    }

    /// 按名字找海克斯（截图识别出来的名字；去掉空白比较）。
    pub fn augment_by_name(&self, name: &str) -> Option<i64> {
        let want: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        if want.is_empty() {
            return None;
        }
        self.augments
            .iter()
            .find(|(_, a)| {
                a.name
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    == want
            })
            .map(|(id, _)| *id)
    }
}

/// 一个海克斯给一个英雄的分。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AugmentScore {
    pub games: i64,
    /// 原始胜率（0..1）。
    pub win_rate: f64,
    /// 平滑后的胜率。
    pub smoothed: f64,
    /// 比这个英雄的平均高（正）/ 低（负）几个百分点（平滑后）。
    pub delta: f64,
    /// 样本少（不到 k 局）。
    pub low_sample: bool,
}

pub fn score(hero_win_rate: f64, cell: Cell) -> AugmentScore {
    let games = cell.games.max(0) as f64;
    let raw = if games > 0.0 {
        cell.wins as f64 / games
    } else {
        hero_win_rate
    };
    let smoothed = (cell.wins as f64 + hero_win_rate * SMOOTH_K) / (games + SMOOTH_K);
    AugmentScore {
        games: cell.games,
        win_rate: (raw * 1e4).round() / 1e4,
        smoothed: (smoothed * 1e4).round() / 1e4,
        delta: ((smoothed - hero_win_rate) * 1000.0).round() / 10.0,
        low_sample: games < SMOOTH_K,
    }
}

/// 页面上要写的署名。
pub fn attribution(d: &Dataset) -> String {
    format!(
        "海斗数据：{REPO}（{LICENSE}，版本 {}，截至 {}，{} 局）",
        d.patch, d.snapshot_at, d.matches
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn latest() -> Latest {
        parse_latest(
            &json!({"path": "patches/16.23/normal", "patch": "16.23", "seq": 20,
            "snapshot_at": "2026-11-20T08:00:00+08:00", "matches": 4991976,
            "files": {"heroes.json": {"sha256": "abc"}}}),
        )
        .unwrap()
    }

    #[test]
    fn latest_and_verify() {
        let l = latest();
        assert_eq!(l.path, "patches/16.23/normal");
        assert_eq!(l.seq, 20);
        assert!(parse_latest(&json!({"path": "../etc"})).is_none());
        assert!(parse_latest(&json!({"path": "https://x"})).is_none());
        let mut l = l;
        // sha256("hi")
        l.files.insert(
            "x.json".into(),
            "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4".into(),
        );
        assert!(verify(&l, "x.json", b"hi"));
        assert!(!verify(&l, "x.json", b"ho"));
        assert!(!verify(&l, "y.json", b"hi"), "没写 SHA-256 的不收");
    }

    #[test]
    fn scores_are_smoothed_against_the_hero() {
        let d = Dataset::from_json(
            &latest(),
            &json!({"39": {"games": 100000, "wins": 50000, "win_rate": 0.5, "pick_rate": 0.01, "rank": 3}}),
            &json!({"1001": {"name": "泰坦的坚决", "rarity": "prismatic"}, "1002": {"name": "尖端发明家", "rarity": "gold"}}),
            &json!({"39": {"1001": {"games": 3000, "wins": 1800}, "1002": {"games": 30, "wins": 30}}}),
        );
        let big = d.score(39, 1001).unwrap();
        assert!(!big.low_sample);
        assert!(big.delta > 8.0 && big.delta < 10.0, "{big:?}");
        // 30 局全胜：样本少，平滑后只高一点点
        let small = d.score(39, 1002).unwrap();
        assert!(small.low_sample && small.win_rate == 1.0);
        assert!(small.delta < big.delta);
        assert!(d.score(39, 9999).is_none());
        assert_eq!(d.best_for(39, 5, 100).len(), 1);
        assert_eq!(d.augment_by_name("泰坦的 坚决"), Some(1001));
        assert!(attribution(&d).contains("CC BY-NC 4.0"));
    }
}
