//! 语音推送谁先说（ADR 043 D3）：纯逻辑，桌面端（`apps/desktop/src-tauri/src/agent/push.rs`）照它排、用她的声音说。
//!
//! 规矩（讨论记录第二节“同一时刻只说最要紧的一句，几十秒内不连着说”，ADR 043 D3 写成数字）：
//!
//! - **同一时刻只说最要紧的一句**：[`GATHER`] 之内来的几条算同时，只留级别最高的（同级留先来的），别的不说（一行和
//!   聊天里的记录照旧）。
//! - **说过一句以后 [`COOLDOWN`] 之内，同级和更低级的不再说**；更高级的可以说，但**不打断正在说的那句**，等它说完。
//! - **过时了就不说**：她自己正在说话时排在她后面（桌面端的播报锁），从推来的时候算，排了 [`STALE`] 还没轮到就不说了；
//!   排在另一条推送的语音后面也一样。没说出来的不算“说过”（不占冷却）。
//!
//! 只排推送的语音；她自己的话、提醒、插件的 `Host::speak` 不归这里管（那些照旧排在播报锁上）。

use std::time::{Duration, Instant};

use kotone_plugin_api::PushPriority;

/// 这么近来的几条算“同一时刻”：等一下，只说最要紧的那条。
pub const GATHER: Duration = Duration::from_millis(300);
/// 说过一句以后多久之内，同级和更低级的不再说。
pub const COOLDOWN: Duration = Duration::from_secs(20);
/// 从推来的时候算，排了多久还没轮到就不说了（过时了）。
pub const STALE: Duration = Duration::from_secs(5);

/// 不说的原因（只写日志）。
pub const COOLED: &str = "20 秒内说过同级或更要紧的一句";
pub const OUTRANKED: &str = "同一时刻有同级或更要紧的一句在等";
pub const REPLACED: &str = "被更要紧的一句换掉了";
pub const TOO_LATE: &str = "排了 5 秒还没轮到，过时了";

/// 排着的那条现在怎样（[`VoiceArbiter::poll`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 再等等（还在凑“同一时刻”，或者另一条推送的语音正在说）。
    Wait,
    /// 轮到了：说（说完调 [`VoiceArbiter::finished`]）。
    Speak,
    /// 不说了（原因）。
    Drop(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct Waiting {
    ticket: u64,
    priority: PushPriority,
    arrived: Instant,
}

#[derive(Debug, Clone, Copy)]
struct Speaking {
    ticket: u64,
    /// 开口前的“最近说过的一句”：这句最后没说出来（过时了、出错）时改回去，不占冷却。
    before: Option<(PushPriority, Instant)>,
}

/// 语音推送的排队（桌面端全进程一份）。时间都由调用方给（测试不用真等）。
#[derive(Debug, Default)]
pub struct VoiceArbiter {
    next: u64,
    waiting: Option<Waiting>,
    speaking: Option<Speaking>,
    /// 最近说出口的一句：级别和开口的时刻（冷却从开口算）。
    last: Option<(PushPriority, Instant)>,
}

impl VoiceArbiter {
    /// 来了一条要说的：排上（票号），或者不说（原因）。排着的同时只有一条：更要紧的换掉等着的那条。
    pub fn offer(&mut self, priority: PushPriority, now: Instant) -> Result<u64, &'static str> {
        if let Some((said, at)) = self.last {
            if now.saturating_duration_since(at) < COOLDOWN && priority <= said {
                return Err(COOLED);
            }
        }
        if self.waiting.is_some_and(|w| priority <= w.priority) {
            return Err(OUTRANKED);
        }
        self.next += 1;
        self.waiting = Some(Waiting {
            ticket: self.next,
            priority,
            arrived: now,
        });
        Ok(self.next)
    }

    /// 排着的那条（票号）现在怎样。轮到了（[`Verdict::Speak`]）就算开口了：记进冷却，同时只有这一条在说。
    pub fn poll(&mut self, ticket: u64, now: Instant) -> Verdict {
        let Some(waiting) = self.waiting.filter(|w| w.ticket == ticket) else {
            return Verdict::Drop(REPLACED);
        };
        let waited = now.saturating_duration_since(waiting.arrived);
        if waited >= STALE {
            self.waiting = None;
            return Verdict::Drop(TOO_LATE);
        }
        if waited < GATHER || self.speaking.is_some() {
            return Verdict::Wait;
        }
        self.waiting = None;
        self.speaking = Some(Speaking {
            ticket,
            before: self.last,
        });
        self.last = Some((waiting.priority, now));
        Verdict::Speak
    }

    /// 说完了（`spoken`），或者最后没说出来（排在她后面过时了、出错、被停下）：没说出来的不占冷却。
    pub fn finished(&mut self, ticket: u64, spoken: bool) {
        if let Some(speaking) = self.speaking.filter(|s| s.ticket == ticket) {
            if !spoken {
                self.last = speaking.before;
            }
            self.speaking = None;
        }
    }

    /// 从推来的时候算，最晚什么时候还得开口（过了就不说）：桌面端等播报锁时用。
    pub fn deadline(arrived: Instant) -> Instant {
        arrived + STALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PushPriority::{High, Low, Normal};

    const MS: Duration = Duration::from_millis(1);

    /// 来一条：凑“同一时刻”的那一下先等，之后就说；说完再来同级的，20 秒内不说。
    #[test]
    fn one_push_speaks_after_a_short_gather() {
        let t = Instant::now();
        let mut a = VoiceArbiter::default();
        let one = a.offer(Normal, t).unwrap();
        assert_eq!(a.poll(one, t + 100 * MS), Verdict::Wait);
        assert_eq!(a.poll(one, t + GATHER), Verdict::Speak);
        a.finished(one, true);
        assert_eq!(a.offer(Normal, t + 10_000 * MS), Err(COOLED));
        assert_eq!(a.offer(Low, t + 19_000 * MS), Err(COOLED));
        assert!(
            a.offer(Normal, t + GATHER + COOLDOWN).is_ok(),
            "冷却从开口算"
        );
    }

    /// 同一时刻来的几条只说最要紧的一句：先来低的、后来高的，低的被换掉；先来高的，后来的同级、低级不排。
    #[test]
    fn the_most_important_of_near_simultaneous_pushes_wins() {
        let t = Instant::now();
        let mut a = VoiceArbiter::default();
        let normal = a.offer(Normal, t).unwrap();
        let high = a.offer(High, t + 100 * MS).unwrap();
        assert_eq!(a.poll(normal, t + GATHER), Verdict::Drop(REPLACED));
        assert_eq!(a.poll(high, t + 100 * MS + GATHER), Verdict::Speak);
        a.finished(high, true);

        let t = t + 60_000 * MS;
        let mut a = VoiceArbiter::default();
        let first = a.offer(Normal, t).unwrap();
        assert_eq!(a.offer(Normal, t + 50 * MS), Err(OUTRANKED), "同级留先来的");
        assert_eq!(a.offer(Low, t + 80 * MS), Err(OUTRANKED));
        assert_eq!(a.poll(first, t + GATHER), Verdict::Speak);
    }

    /// 冷却里更高级的可以说，但不打断正在说的那句：等它说完才轮到。
    #[test]
    fn a_higher_push_may_speak_during_the_cooldown_but_never_cuts_in() {
        let t = Instant::now();
        let mut a = VoiceArbiter::default();
        let normal = a.offer(Normal, t).unwrap();
        assert_eq!(a.poll(normal, t + GATHER), Verdict::Speak);
        // 她正在说这一句（1.5 秒），更要紧的来了
        let high = a.offer(High, t + 800 * MS).unwrap();
        assert_eq!(
            a.poll(high, t + 1_200 * MS),
            Verdict::Wait,
            "不打断正在说的"
        );
        a.finished(normal, true);
        assert_eq!(a.poll(high, t + 1_900 * MS), Verdict::Speak);
        a.finished(high, true);
        // 冷却按最近说的那句（高级）算：普通的、高级的都要等 20 秒
        assert_eq!(a.offer(Normal, t + 5_000 * MS), Err(COOLED));
        assert_eq!(a.offer(High, t + 5_000 * MS), Err(COOLED));
    }

    /// 排了 5 秒还没轮到（另一条推送的语音一直在说）就不说了；没说出来的不占冷却。
    #[test]
    fn stale_pushes_are_dropped_and_unspoken_ones_do_not_count() {
        let t = Instant::now();
        let mut a = VoiceArbiter::default();
        let long = a.offer(Normal, t).unwrap();
        assert_eq!(a.poll(long, t + GATHER), Verdict::Speak);
        let high = a.offer(High, t + 1_000 * MS).unwrap();
        assert_eq!(a.poll(high, t + 5_900 * MS), Verdict::Wait);
        assert_eq!(a.poll(high, t + 6_000 * MS), Verdict::Drop(TOO_LATE));
        assert_eq!(
            VoiceArbiter::deadline(t + 1_000 * MS),
            t + 6_000 * MS,
            "等她的播报锁时用同一个期限"
        );
        // 那一句其实没说出来（排在她后面过时了）：不占冷却，同级的马上能说
        a.finished(long, false);
        let next = a.offer(Normal, t + 7_000 * MS).unwrap();
        assert_eq!(a.poll(next, t + 7_000 * MS + GATHER), Verdict::Speak);
        // 别人的票号、已经说完的票号：什么都不动
        a.finished(999, true);
        a.finished(long, true);
        assert_eq!(a.poll(long, t + 8_000 * MS), Verdict::Drop(REPLACED));
    }
}
