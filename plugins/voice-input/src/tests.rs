//! 语音输入的单元测试（从 `kotone-core` 的 `orchestrator/tests.rs` 搬来，名字和内容不变）。

use kotone_core::profile::GameProfile;

use crate::records::{trim_speech_pcm, HistoryDraft};
use crate::send::resolve_send_strategy;

// ---------- 发送频道解析（ADR-008） ----------

#[test]
fn resolve_send_strategy_uses_selected_channel() {
    let lol = GameProfile::builtin_lol();
    let sel = Some(("lol".to_string(), "all".to_string()));
    let s = resolve_send_strategy(&lol, &sel);
    assert_eq!(s.id, "all");
    assert_eq!(s.open_chat_key, "Shift+Enter");
}

#[test]
fn resolve_send_strategy_falls_back_when_profile_switched() {
    // 频道残留属于 lol，但当前 profile 已是 generic → 默认频道
    let generic = GameProfile::builtin_generic();
    let sel = Some(("lol".to_string(), "all".to_string()));
    let s = resolve_send_strategy(&generic, &sel);
    assert!(s.is_default);
    assert_eq!(s.open_chat_key, generic.open_chat_key);
    // 未选择（None）同样回落默认
    let s2 = resolve_send_strategy(&lol_clone(), &None);
    assert_eq!(s2.id, "team");
}

fn lol_clone() -> GameProfile {
    GameProfile::builtin_lol()
}

#[test]
fn wire_profile_makes_the_selected_channel_the_default() {
    let mut dota = GameProfile::builtin_dota2();
    dota.channels[0].console_command = Some("say_team".into());
    dota.channels[1].console_command = Some("say".into());
    let sel = Some(("dota2".to_string(), "all".to_string()));
    let wire = dota.for_channel(&resolve_send_strategy(&dota, &sel));
    assert_eq!(wire.open_chat_key, "Shift+Enter");
    let sent = wire.channel_strategy(None);
    assert_eq!(sent.id, "all");
    assert_eq!(sent.console_command, "say");
    // 没选频道：默认频道（队伍）
    let wire = dota.for_channel(&resolve_send_strategy(&dota, &None));
    assert_eq!(wire.open_chat_key, "Enter");
    assert_eq!(wire.channel_strategy(None).console_command, "say_team");
}

// ---------- 历史音频 VAD 裁剪（trim_speech_pcm / push_speech_range） ----------

#[test]
fn trim_speech_pcm_no_segments_returns_original() {
    let pcm = vec![0.5f32; 1000];
    let out = trim_speech_pcm(&pcm, &[]);
    assert_eq!(out, pcm, "非 VAD 模式（无区间）应原样保存");
}

#[test]
fn trim_speech_pcm_single_segment_with_prepost_roll() {
    let pcm: Vec<f32> = (0..10000).map(|i| i as f32).collect();
    // 段 [1000,2000)：前导缓冲取到 0（PRE 2400 超出开头），尾随缓冲到 4400
    let out = trim_speech_pcm(&pcm, &[(1000, 2000)]);
    assert_eq!(out, pcm[0..4400]);
    // 段 [9000,9500)：尾随缓冲 clamp 到 pcm 末尾；前导取到 6600
    let out2 = trim_speech_pcm(&pcm, &[(9000, 9500)]);
    assert_eq!(out2, pcm[6600..10000]);
    // 段 [500,1500)：前后都 clamp 到界内
    let out3 = trim_speech_pcm(&pcm, &[(500, 1500)]);
    assert_eq!(out3, pcm[0..3900]);
}

#[test]
fn trim_speech_pcm_merges_close_segments_after_roll() {
    let pcm: Vec<f32> = (0..10000).map(|i| i as f32).collect();
    // 两段间隙 2000 采样（125ms）< 缓冲覆盖（PRE+POST=4800）→ 合并成一段
    let out = trim_speech_pcm(&pcm, &[(1000, 2000), (4000, 5000)]);
    assert_eq!(out, pcm[0..7400]);
    // 长停顿（间隙 6000 采样 = 375ms > 4800）→ 断开，中间静音裁掉
    let out2 = trim_speech_pcm(&pcm, &[(1000, 2000), (8000, 9000)]);
    assert_eq!(out2.len(), 4400 + 4400);
    assert_eq!(out2[..4400], pcm[0..4400]);
    assert_eq!(out2[4400..], pcm[5600..10000]);
}

#[test]
fn trim_speech_pcm_clamps_out_of_bounds() {
    let pcm: Vec<f32> = (0..50).map(|i| i as f32).collect();
    // 短 PCM + 越界区间：全部扩展后重叠合并、clamp 到 pcm 边界，不 panic
    let out = trim_speech_pcm(&pcm, &[(40, 100), (200, 300)]);
    assert_eq!(out, pcm, "区间越界时应整体 clamp 到 pcm 长度");
}

#[test]
fn push_speech_range_merges_close_segments() {
    let mut g = HistoryDraft::speech_harness();
    // 帧 0-10 语音，停顿 100ms（1600 采样 < 300ms 合并阈值）后又语音
    g.push_speech_range(0, 4800);
    g.push_speech_range(6400, 9600); // 间隙 1600 采样 = 100ms → 合并
    assert_eq!(g.speech_segments, vec![(0, 9600)]);
    // 长停顿（> 300ms）后断开
    g.push_speech_range(19200, 24000); // 距 9600 差 9600 采样 = 600ms → 新段
    assert_eq!(g.speech_segments, vec![(0, 9600), (19200, 24000)]);
    // 无效区间忽略
    g.push_speech_range(50000, 50000);
    assert_eq!(g.speech_segments.len(), 2);
}

#[test]
fn vad_speech_seen_defaults_false() {
    let g = HistoryDraft::speech_harness();
    assert!(!g.vad_speech_seen);
    assert!(g.speech_segments.is_empty());
}
