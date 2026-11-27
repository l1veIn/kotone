//! 固定的规则和叫法：各模式打几回合、什么时候算赛点；地图、模式的中文名。只说游戏里的事实。
//!
//! 回合数照 CS2 的模式设置（`gamemode_competitive.cfg` 的 `mp_maxrounds 24`、`mp_overtime_enable 1`、
//! `mp_overtime_maxrounds 6`；搭档 `mp_maxrounds 16`）：竞技和优先 24 回合（先赢 13），12 : 12 进加时，加时每段 6 回合
//! （先多赢 4）；搭档 16 回合（先赢 9），加时的规矩没核实，不判断。别的模式（休闲、死亡竞赛……）不判断赛点。

use crate::gsi::Side;

/// 一个模式打几回合（常规时间）、加时每段几回合（没有加时 = None）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    pub regulation: i64,
    pub overtime: Option<i64>,
}

/// 这个模式的回合规矩；不知道的模式 None（不判断赛点）。`premier` 是防着游戏哪天把优先模式单独报出来。
pub fn rules(mode: &str) -> Option<Rules> {
    match mode {
        "competitive" | "premier" => Some(Rules {
            regulation: 24,
            overtime: Some(6),
        }),
        "scrimcomp2v2" => Some(Rules {
            regulation: 16,
            overtime: None,
        }),
        _ => None,
    }
}

/// 这个比分下，哪边再赢一回合就赢下这张图（不是赛点时 None）。
///
/// 常规时间先赢 `regulation / 2 + 1`；打满常规时间（平）进加时，第 k 段加时（从 0 数）开始时两边都是
/// `regulation / 2 + k × overtime / 2`，先到 `+ overtime / 2 + 1` 的赢。两边同时差一回合只会出现在一段的末尾（平了进下一段），
/// 所以要求另一边还差得更多。
pub fn match_point(rules: Rules, ct: i64, t: i64) -> Option<Side> {
    let (ct, t) = (ct.max(0), t.max(0));
    let total = ct + t;
    let half = rules.regulation / 2;
    let target = if total < rules.regulation {
        half + 1
    } else {
        let ot = rules.overtime.filter(|ot| *ot >= 2)?;
        let k = (total - rules.regulation) / ot;
        half + (ot / 2) * k + ot / 2 + 1
    };
    if ct == target - 1 && t < target - 1 {
        Some(Side::Ct)
    } else if t == target - 1 && ct < target - 1 {
        Some(Side::T)
    } else {
        None
    }
}

/// 地图的中文名（国服的叫法）；不认识的为空，用游戏里的名字。
pub fn map_name(map: &str) -> &'static str {
    match map {
        "de_mirage" => "荒漠迷城",
        "de_inferno" => "炼狱小镇",
        "de_nuke" => "核子危机",
        "de_ancient" => "远古遗迹",
        "de_anubis" => "阿努比斯",
        "de_dust2" => "炙热沙城 2",
        "de_vertigo" => "殒命大厦",
        "de_overpass" => "死亡游乐园",
        "de_train" => "列车停放站",
        "cs_office" => "办公室",
        "cs_italy" => "意大利小镇",
        _ => "",
    }
}

/// 模式的中文名；不认识的为空。
pub fn mode_name(mode: &str) -> &'static str {
    match mode {
        "competitive" => "竞技",
        "premier" => "优先",
        "scrimcomp2v2" => "搭档",
        "casual" => "休闲",
        "deathmatch" => "死亡竞赛",
        "gungameprogressive" => "军备竞赛",
        "retakes" => "回防",
        _ => "",
    }
}

/// 给她看、记进说明的地图名：有中文名用中文名，没有用游戏里的名字。
pub fn map_label(map: &str) -> String {
    match map_name(map) {
        "" => map.to_string(),
        name => name.to_string(),
    }
}

/// 同上，模式。
pub fn mode_label(mode: &str) -> String {
    match mode_name(mode) {
        "" => mode.to_string(),
        name => name.to_string(),
    }
}

/// `round_wins` 里的写法 → 怎么赢的（事件里的 `reason`）。
pub fn win_reason(raw: &str) -> &'static str {
    match raw {
        "ct_win_elimination" | "t_win_elimination" => "elimination",
        "t_win_bomb" => "bomb",
        "ct_win_defuse" => "defuse",
        "ct_win_time" => "time",
        _ => "",
    }
}

/// 炸弹从下好到爆炸几秒（CS2 的 `mp_c4timer` 默认 40）。自己打时 GSI 不给倒数，按这个算。
pub const C4_SECONDS: f64 = 40.0;

#[cfg(test)]
mod tests {
    use super::*;

    const COMP: Rules = Rules {
        regulation: 24,
        overtime: Some(6),
    };

    #[test]
    fn regulation_match_point_is_twelve_wins() {
        assert_eq!(match_point(COMP, 12, 11), Some(Side::Ct));
        assert_eq!(match_point(COMP, 3, 12), Some(Side::T));
        assert_eq!(match_point(COMP, 12, 0), Some(Side::Ct));
        assert_eq!(match_point(COMP, 11, 11), None);
        assert_eq!(match_point(COMP, 0, 0), None);
        // 12 : 12 进加时，不是赛点
        assert_eq!(match_point(COMP, 12, 12), None);
    }

    #[test]
    fn overtime_match_point_is_four_of_six() {
        assert_eq!(match_point(COMP, 13, 12), None, "加时第一段 13 : 12 还差 3");
        assert_eq!(match_point(COMP, 15, 14), Some(Side::Ct));
        assert_eq!(match_point(COMP, 14, 15), Some(Side::T));
        assert_eq!(match_point(COMP, 15, 13), Some(Side::Ct));
        // 15 : 15 进第二段加时，再到 18 才是赛点
        assert_eq!(match_point(COMP, 15, 15), None);
        assert_eq!(match_point(COMP, 18, 17), Some(Side::Ct));
        assert_eq!(match_point(COMP, 17, 18), Some(Side::T));
    }

    #[test]
    fn wingman_has_no_overtime_rule() {
        let wingman = rules("scrimcomp2v2").unwrap();
        assert_eq!(match_point(wingman, 8, 5), Some(Side::Ct));
        assert_eq!(match_point(wingman, 8, 8), None);
        assert_eq!(match_point(wingman, 9, 8), None, "加时没核实，不判断");
        assert!(rules("casual").is_none());
        assert!(rules("deathmatch").is_none());
        assert_eq!(rules("competitive"), Some(COMP));
    }

    #[test]
    fn names() {
        assert_eq!(map_label("de_mirage"), "荒漠迷城");
        assert_eq!(map_label("workshop/1/aim"), "workshop/1/aim");
        assert_eq!(mode_label("competitive"), "竞技");
        assert_eq!(mode_label("weird"), "weird");
        assert_eq!(win_reason("t_win_bomb"), "bomb");
        assert_eq!(win_reason("x"), "");
    }
}
