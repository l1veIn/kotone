//! 自动化的定时（类似 crontab）：五段 `分 时 日 月 周`，本地时间。
//!
//! 支持 `*`、数字、列表 `1,15`、范围 `1-5`、步长 `*/10`、`9-18/3`，周几 0 和 7 都是周日；
//! 另有 `@hourly`、`@daily`（`@midnight`）、`@weekly`、`@monthly`。“日”和“周”都写了时按 crontab
//! 的规矩：满足任一个就算。错过的不补（Kotone 没开时到点的不会事后补跑）。

use chrono::{Datelike, Duration, Local, NaiveDateTime, TimeZone, Timelike};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: Vec<bool>,
    hours: Vec<bool>,
    days: Vec<bool>,
    months: Vec<bool>,
    weekdays: Vec<bool>,
    days_any: bool,
    weekdays_any: bool,
}

fn field(text: &str, min: u32, max: u32, what: &str) -> Result<(Vec<bool>, bool), String> {
    let mut set = vec![false; max as usize + 1];
    let any = text == "*";
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .map_err(|_| format!("{what}的步长「{step}」不是数字"))?;
                if step == 0 {
                    return Err(format!("{what}的步长不能是 0"));
                }
                (range, step)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let a: u32 = a.parse().map_err(|_| format!("{what}「{a}」不是数字"))?;
            let b: u32 = b.parse().map_err(|_| format!("{what}「{b}」不是数字"))?;
            (a, b)
        } else {
            let v: u32 = range
                .parse()
                .map_err(|_| format!("{what}「{range}」不是数字"))?;
            // `5/10` = 从 5 开始每 10 个
            (v, if part.contains('/') { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("{what}要在 {min}–{max} 之间"));
        }
        let mut v = lo;
        while v <= hi {
            set[v as usize] = true;
            v += step;
        }
    }
    Ok((set, any))
}

impl Cron {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let expanded = match text {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            other => other,
        };
        let parts: Vec<&str> = expanded.split_whitespace().collect();
        if parts.len() != 5 {
            return Err("定时要写五段：分 时 日 月 周（例如 0 8 * * * = 每天 8 点）".into());
        }
        let (minutes, _) = field(parts[0], 0, 59, "分钟")?;
        let (hours, _) = field(parts[1], 0, 23, "小时")?;
        let (days, days_any) = field(parts[2], 1, 31, "日期")?;
        let (months, _) = field(parts[3], 1, 12, "月份")?;
        let (mut weekdays, weekdays_any) = field(parts[4], 0, 7, "星期")?;
        if weekdays[7] {
            weekdays[0] = true;
        }
        weekdays.truncate(7);
        Ok(Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            days_any,
            weekdays_any,
        })
    }

    pub fn matches(&self, t: &NaiveDateTime) -> bool {
        let day_ok = self.days[t.day() as usize];
        let weekday_ok = self.weekdays[t.weekday().num_days_from_sunday() as usize];
        let date_ok = match (self.days_any, self.weekdays_any) {
            (true, true) => true,
            (true, false) => weekday_ok,
            (false, true) => day_ok,
            (false, false) => day_ok || weekday_ok,
        };
        self.minutes[t.minute() as usize]
            && self.hours[t.hour() as usize]
            && self.months[t.month() as usize]
            && date_ok
    }

    /// `after` 之后（不含）第一次到点的时刻（整分）。一年内都没有就 None（例如 2 月 30 日）。
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let mut t = after.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let limit = after + Duration::days(366 * 4);
        while t <= limit {
            if !self.months[t.month() as usize] {
                t = (t.date().with_day(1)? + chrono::Months::new(1)).and_hms_opt(0, 0, 0)?;
                continue;
            }
            let date_ok = {
                let day_ok = self.days[t.day() as usize];
                let weekday_ok = self.weekdays[t.weekday().num_days_from_sunday() as usize];
                match (self.days_any, self.weekdays_any) {
                    (true, true) => true,
                    (true, false) => weekday_ok,
                    (false, true) => day_ok,
                    (false, false) => day_ok || weekday_ok,
                }
            };
            if !date_ok {
                t = (t.date() + Duration::days(1)).and_hms_opt(0, 0, 0)?;
                continue;
            }
            if !self.hours[t.hour() as usize] {
                t = t.with_minute(0)? + Duration::hours(1);
                continue;
            }
            if self.minutes[t.minute() as usize] {
                return Some(t);
            }
            t += Duration::minutes(1);
        }
        None
    }
}

/// 本地时间里，`after_ms` 之后下一次到点的时刻（毫秒时间戳）。
pub fn next_run_ms(cron: &Cron, after_ms: u64) -> Option<u64> {
    let after = Local
        .timestamp_millis_opt(after_ms as i64)
        .single()?
        .naive_local();
    let next = cron.next_after(after)?;
    let local = Local.from_local_datetime(&next).earliest()?;
    u64::try_from(local.timestamp_millis()).ok()
}

/// (from_ms, to_ms] 之间有没有到点（定时器每次醒来检查这一段；错过太久的不补）。
pub fn due_between(cron: &Cron, from_ms: u64, to_ms: u64) -> bool {
    next_run_ms(cron, from_ms).is_some_and(|next| next <= to_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    #[test]
    fn parses_the_common_forms() {
        assert!(Cron::parse("0 8 * * *").is_ok());
        assert!(Cron::parse("*/10 9-18 * * 1-5").is_ok());
        assert!(Cron::parse("@daily").is_ok());
        assert!(Cron::parse("0 8 * *").is_err());
        assert!(Cron::parse("60 * * * *").is_err());
        assert!(Cron::parse("*/0 * * * *").is_err());
        assert!(Cron::parse("a * * * *").is_err());
    }

    #[test]
    fn finds_the_next_time() {
        let daily8 = Cron::parse("0 8 * * *").unwrap();
        assert_eq!(
            daily8.next_after(at(2026, 11, 18, 7, 59)),
            Some(at(2026, 11, 18, 8, 0))
        );
        assert_eq!(
            daily8.next_after(at(2026, 11, 18, 8, 0)),
            Some(at(2026, 11, 19, 8, 0))
        );
        let every10 = Cron::parse("*/10 * * * *").unwrap();
        assert_eq!(
            every10.next_after(at(2026, 11, 18, 12, 31)),
            Some(at(2026, 11, 18, 12, 40))
        );
        // 2026-11-21 是周六：工作日 9 点的下一次是周一
        let weekdays = Cron::parse("0 9 * * 1-5").unwrap();
        assert_eq!(
            weekdays.next_after(at(2026, 11, 20, 10, 0)),
            Some(at(2026, 11, 23, 9, 0))
        );
        // 日和周都写了：满足任一个（每月 1 号，或者周日）
        let either = Cron::parse("0 0 1 * 0").unwrap();
        assert_eq!(
            either.next_after(at(2026, 11, 18, 0, 0)),
            Some(at(2026, 11, 22, 0, 0))
        );
        assert!(either.matches(&at(2026, 12, 1, 0, 0)));
        // 周日写 7 也行；不存在的日期没有下一次
        assert!(Cron::parse("0 0 * * 7")
            .unwrap()
            .matches(&at(2026, 11, 22, 0, 0)));
        assert_eq!(
            Cron::parse("0 0 30 2 *")
                .unwrap()
                .next_after(at(2026, 1, 1, 0, 0)),
            None
        );
    }

    #[test]
    fn due_between_checks_a_window() {
        let cron = Cron::parse("* * * * *").unwrap();
        assert!(due_between(&cron, 0, 61_000));
        assert!(!due_between(&cron, 60_000, 60_500));
    }
}
