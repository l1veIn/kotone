# ADR-008：游戏聊天频道抽象与频道切换热键

状态：已接受（2026-07，随 0.1.5 落地生效）

## 背景

LOL 存在「队伍 / 所有人」两个聊天频道，且游戏原生支持两种发全体的方式：
按 Enter 开框后按住 Shift 回车，或在文本前加 `/all ` 前缀。用户在录音笔 /
对讲机模式下发现「发送前按住 Shift」可以把消息发到所有人频道，说明
发送策略需要按频道维度抽象，而不是写死在 profile 的一组按键里。

同时频道抽象必须面向未来：Dota2、瓦洛兰特、三角洲行动等游戏适配会陆续
加入，有的游戏本身支持频道切换（只需要一个「默认频道」），有的游戏有
多个频道且各频道的开框键 / 文本前缀不同。因此频道声明放在**游戏配置层**
（profile），热键与 UI 放在设置层。

## 决策

### 1. profile 层声明 `channels[]`，按键策略与前缀策略正交

```jsonc
{
  "channels": [
    { "id": "team", "displayName": "队伍", "default": true },
    { "id": "all", "displayName": "所有人", "openChatKey": "Shift+Enter" }
  ]
}
```

- 每个频道可声明 `openChatKey`（该频道专属开框键，缺省沿用 profile 级
  `openChatKey`）与 `textPrefix`（发送时拼在文本前，如 `/all `），两者
  正交、可同时设置。LOL 的「所有人」用 `Shift+Enter` 开框（游戏会自动
  预填 /all），无需前缀。
- `default: true` 标记默认频道；缺省取第一个。
- **空 `channels` = 单频道**：从 profile 级 `openChatKey` 合成唯一默认
  频道，存量 profile 行为零变化。`merge_builtin_hotwords_in` 扩展为同时
  补缺失的 channels（只补缺失、不覆盖用户改动），老配置自动获得 LOL
  双频道。

### 2. 独立的「频道切换热键」，默认 Shift+CapsLock，放高级页

- 新设置项 `channelCycleHotkey`，默认 `Shift+CapsLock`，配置 UI 在
  「高级」页（录制热键仍在「通用」页）。按声明顺序**循环**切换。
- 曾考虑「Shift+录制键」派生方案，否决：录制键本身可能是组合键，且
  按住 Shift 再按录制键会物理触发录制，边界情况太多。
- **冲突校验是双向的**：前端保存录制键 / 切换键时都用
  `combosConflict`（与 Rust `combos_conflict` 同语义：修饰键集合 + 主键
  完全相同）预检并拒绝；后端 `HotkeyManager.apply_cycle_key` 注册时
  再校验一次，冲突则不注册并写入 `HotkeyStatus.cycle_error` 展示。
- 匹配器严格修饰键相等，因此 `Shift+CapsLock`（切换）与裸 `CapsLock`
  （录制）互不误触。**关键实现细节**：切换键分支未命中修饰键时必须
  落回主键分支重判，否则同 vk 的裸 CapsLock 录制会被短路。

### 3. Sticky 频道态，发送时刻读取

- orchestrator 存 `(profile_id, channel_id)`；profile 不符（切了游戏
  适配）的残留视为「位于默认频道」，重启同理回默认。
- 频道在 **do_send 发送时刻**读取（经纯函数 `resolve_send_strategy`
  解析，克隆 profile 换开框键、`wire_text = prefix + text`），因此
  toggle / solo 模式会话中切换即时生效；hold 模式物理上按不出组合键，
  无需特判。用户原文不被前缀污染。
- 单频道 profile 下切换键是无操作（no-op）。

### 4. 悬浮窗反馈：瞬时露出 + 常驻徽标

- 切换时 emit `kotone://channel { channelId, displayName, isDefault }`。
- 悬浮窗两种布局（capsule / card）在非默认频道时显示频道徽标，频道
  变化时用 `{#key}` 重放弹跳动画提示「已切换」；默认频道不挂徽标。
- `overlay.visibility = on_demand` 时悬浮窗平时隐藏，壳层在 channel
  事件上短暂 `show_window_no_focus` ~1.2s 再收回（vis_gen 代际防新
  会话误藏）；`always` 模式运行期间常显，无需处理。
- 「游戏适配」页的 profile 卡片展示该游戏支持的频道列表与切换键，
  让用户在配置层就能看到频道能力。

## 后果

- 新增游戏适配时只需在 profile JSON 里声明 `channels[]`，发送链路、
  热键、UI 全部复用；单频道游戏什么也不用写。
- 切换键与录制键、Esc 取消键、LL 钩子 / 系统热键两种后端完全解耦，
  `HotkeySource::set_cycle_key` 默认空实现，CLI 与壳各自接线。
