//! 设置项说明表：给内置琴音 Agent 和外部 Agent 看的“每个配置是什么、能填什么、在哪、怎么生效”。
//!
//! 设置是一个嵌套的 JSON 对象（camelCase）。这里按点分路径（如 `overlay.style`）列出用户能改的项；
//! 不给 Agent 改、或要用专用工具改的项列在 [`INTERNAL`] 里。测试保证默认设置里的每个叶子
//! 路径要么有说明、要么在 `INTERNAL` 里，新增设置忘了写说明会直接失败；有可选值的项，
//! 每个可选值都能被设置反序列化接受。

use serde::Serialize;
use serde_json::Value;

/// 改了以后怎么生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    /// 保存后立即生效。
    Immediate,
    /// 语音识别正在运行时，要重启语音识别才生效（`runtime.start`；没在运行则下次启动生效）。
    RestartVoice,
    /// 要重启 Kotone 才生效（`app.restart`）。
    RestartApp,
    /// 下次启动 Kotone 时生效，不必现在重启。
    NextLaunch,
}

impl Effect {
    pub fn label(self) -> &'static str {
        match self {
            Effect::Immediate => "立即生效",
            Effect::RestartVoice => "要重启语音识别",
            Effect::RestartApp => "要重启 Kotone",
            Effect::NextLaunch => "下次启动生效",
        }
    }
}

/// 一个设置项。`path` 以 `.*` 结尾表示一组同类的键（映射）。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub path: &'static str,
    pub label: &'static str,
    /// 设置页里的位置。
    pub page: &'static str,
    pub description: &'static str,
    /// 可选值：(值, 含义)。空 = 不是枚举（见 description 里的范围）。
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub values: &'static [(&'static str, &'static str)],
    pub effect: Effect,
}

const fn f(
    path: &'static str,
    label: &'static str,
    page: &'static str,
    description: &'static str,
    values: &'static [(&'static str, &'static str)],
    effect: Effect,
) -> Field {
    Field {
        path,
        label,
        page,
        description,
        values,
        effect,
    }
}

use Effect::{Immediate as NOW, NextLaunch as NEXT, RestartApp as APP, RestartVoice as VOICE};

const NONE: &[(&str, &str)] = &[];
const EFFORTS: &[(&str, &str)] = &[
    ("", "服务商默认"),
    ("none", "不思考"),
    ("low", "低"),
    ("high", "高"),
    ("max", "最高"),
];
/// 游戏里（语音入口）的思考强度：空 = 不思考（ADR 029），其余同 [`EFFORTS`]。
const GAME_EFFORTS: &[(&str, &str)] = &[
    ("", "不思考（默认）"),
    ("none", "不思考"),
    ("low", "低"),
    ("high", "高"),
    ("max", "最高"),
];
const RECORD_SOUNDS: &[(&str, &str)] = &[
    ("rise", "上扬"),
    ("ding-up", "叮（升）"),
    ("chirp-up", "啁啾（升）"),
    ("fall", "下落"),
    ("knock", "敲击"),
    ("ding-down", "叮（降）"),
];

pub const FIELDS: &[Field] = &[
    // ---- 语音输入（说话 → 识别 → 原话打进游戏聊天框；不经过 Agent）----
    f("hotkey.key", "语音输入热键", "插件 → 语音输入 → 输入", "按键名，如 CapsLock、F8、Ctrl+Alt+Q。改之前可用 hotkeys.conflicts 查冲突。", NONE, NOW),
    f("hotkey.mode", "热键方式", "插件 → 语音输入 → 输入", "按住说话还是按一下开始、再按一下结束。", &[("hold", "按住说话，松开结束"), ("toggle", "按一下开始，再按一下结束")], NOW),
    f("interactionMode", "语音输入方式", "插件 → 语音输入 → 交互模式", "识别完以后怎么处理。", &[("push-to-talk", "对讲机：按住说话，松开发送"), ("dictation", "录音笔：按一下开始，再按一下结束，预览确认后发送"), ("one-shot", "说一句就走：按一下开始，说完自动停、自动发送"), ("solo", "独奏模式：持续收音，说一句发一句，再按一下停止")], NOW),
    f("autoSend", "识别后自动发送", "插件 → 语音输入 → 交互模式", "true 时识别完直接发到游戏聊天，不等确认。", NONE, NOW),
    f("vadSilenceMs", "停顿多久算说完", "设置 → 调参", "毫秒，自动结束收音的静音时长。常用 500–1500。", NONE, NOW),
    f("vad.threshold", "人声检测灵敏度", "设置 → 调参", "0–1，越大越不容易把噪声当人声。", NONE, NOW),
    f("vad.minSpeechMs", "最短语音", "设置 → 调参", "毫秒，短于它的声音不算说话。", NONE, NOW),
    f("vad.minSilenceMs", "最短静音", "设置 → 调参", "毫秒，人声检测内部的最短静音段。", NONE, NOW),
    f("audioDeviceId", "麦克风", "设置 → 声音", "设备 ID，用 audio.devices 查；空或 default = 系统默认。语音输入和呼叫她共用这一个麦克风。", NONE, NOW),
    f("lowerSpeakerVolumeWhileListening", "录音时降低扬声器音量", "设置 → 声音", "默认关闭。开启后，下次按键录音或琴音开始倾听时暂时降低 Windows 默认播放设备的音量，采集结束后恢复。", NONE, NOW),
    f("sttEngine", "语音识别引擎", "设置 → 模型 → 语音识别", "引擎 ID，用 stt.engines 查。一般用 models.activate 同时选引擎和模型。", NONE, VOICE),
    f("activeModelId", "语音识别模型", "设置 → 模型 → 语音识别", "用 models.activate 选择（会同时校验模型已下载）。", NONE, VOICE),
    f("engineOptions.*", "引擎选项", "设置 → 模型", "各引擎的附加选项，如 sherpa-streaming 的 provider（cpu）。", NONE, VOICE),
    f("hotwordsScore", "热词加权", "设置 → 调参", "游戏配置里热词的识别加权，常用 1–5。", NONE, VOICE),
    f("activeProfileId", "当前游戏配置", "游戏列表", "游戏配置 ID，用 profiles.list 查。热词随游戏配置变化。", NONE, VOICE),
    f("channelCycleHotkey", "切换频道热键", "插件 → 语音输入 → 输入 → 其他热键", "语音输入时在游戏的聊天频道之间切换；空 = 不用。", NONE, NOW),
    f("resendLastHotkey", "重发上一句热键", "插件 → 语音输入 → 输入 → 其他热键", "把语音输入上一句发成功的话再发一次；空 = 不用。", NONE, NOW),
    f("togglePostProcessingHotkey", "开关文字处理热键", "插件 → 语音输入 → 输入 → 其他热键", "空 = 不用。", NONE, NOW),
    f("cyclePostProcessingHotkey", "切换文字处理方案热键", "插件 → 语音输入 → 输入 → 其他热键", "空 = 不用。", NONE, NOW),
    f("runtimeToggleHotkey", "启动 / 停止本地语音识别热键", "设置 → 模型 → 语音识别", "只在用本地语音识别时显示和生效：按一下启动（把本地模型加载进内存），再按一下停止。在线识别打开 Kotone 就能用，没有启动这回事；换成在线识别时这个值保留，换回本地时还在。空 = 不用。", NONE, NOW),
    f("automationStopHotkey", "停下热键", "设置 → 系统 → 停下", "停下：立刻停止她正在做的事（操作游戏、后台的事）和所有自动化，按住的键一起松开。也可以按 Esc（她操作游戏时）、说“停下”或用托盘菜单「停下」。空 = 不用（托盘仍能停）。", NONE, NOW),
    f("hotkeyBackend", "热键实现方式", "设置 → 系统", "游戏里热键不灵时可以换。", &[("auto", "自动"), ("llhook", "低层键盘钩子"), ("register", "系统热键注册")], NOW),
    f("hotkeys.bindings", "热键表", "设置 → 热键", "全部热键存在这里：数组，每条 {key, command}。command 是 <来源>.<命令>：kotone.call 呼叫、kotone.stop 停下、kotone.speech-toggle 启动 / 停止本地语音识别、voice-input.talk 说话、voice-input.channel-cycle 切换频道、voice-input.resend-last 重发上一句、voice-input.post-process-toggle 开关文字处理、voice-input.post-process-cycle 切换文字处理方案。整组替换；只改一个键时照旧写 hotkey.key、channelCycleHotkey、assistant.hotkey 这些字段也行，会改进表里。看全部热键和冲突用 hotkeys.status，改之前用 hotkeys.conflicts 查。", NONE, NOW),
    f("postProcessing.enabled", "文字处理开关", "插件 → 语音输入 → 文字处理", "语音输入的识别结果发送前是否经过文字处理（屏蔽词、润色、翻译等）。", NONE, NOW),
    f("postProcessing.activePipelineId", "当前文字处理方案", "插件 → 语音输入 → 文字处理", "方案 ID（见 postProcessing.pipelines）。", NONE, NOW),
    f("postProcessing.pipelines", "文字处理方案列表", "插件 → 语音输入 → 文字处理", "整组替换；处理器用 postprocess.processors 查，可用 postprocess.test 试。", NONE, NOW),
    // ---- 悬浮窗与声音 ----
    f("overlay.visibility", "悬浮窗显示", "插件 → 语音输入 → 悬浮窗", "说话时显示识别中的文字。", &[("always", "一直显示"), ("on_demand", "说话时显示"), ("never", "不显示")], NOW),
    f("overlay.style", "悬浮窗样式", "插件 → 语音输入 → 悬浮窗", "", &[("capsule", "胶囊"), ("card", "卡片")], NOW),
    f("overlay.position", "悬浮窗位置", "插件 → 语音输入 → 悬浮窗", "custom 时用 overlay.customX / customY（像素）。", &[("auto", "智能"), ("top_left", "左上"), ("top_center", "上方居中"), ("top_right", "右上"), ("center", "正中"), ("bottom_left", "左下"), ("bottom_center", "下方居中"), ("bottom_right", "右下"), ("custom", "自定义")], NOW),
    f("overlay.draggable", "悬浮窗可拖动", "插件 → 语音输入 → 悬浮窗", "", NONE, NOW),
    f("overlay.clickThrough", "悬浮窗点击穿透", "插件 → 语音输入 → 悬浮窗", "true 时鼠标点击穿过悬浮窗，不影响游戏。", NONE, NOW),
    f("soundFeedback.record.enabled", "开始录音提示音", "设置 → 声音", "", NONE, NOW),
    f("soundFeedback.record.volume", "开始录音提示音音量", "设置 → 声音", "0–100。", NONE, NOW),
    f("soundFeedback.record.soundId", "开始录音提示音", "设置 → 声音", "", RECORD_SOUNDS, NOW),
    f("soundFeedback.send.enabled", "发送提示音", "设置 → 声音", "", NONE, NOW),
    f("soundFeedback.send.volume", "发送提示音音量", "设置 → 声音", "0–100。", NONE, NOW),
    f("soundFeedback.send.soundId", "发送提示音", "设置 → 声音", "", RECORD_SOUNDS, NOW),
    // ---- 游戏陪玩（角色 / Agent）：呼叫 = 呼叫键、唤醒词、聊天页的呼叫按钮 ----
    f("assistant.hotkey", "呼叫键", "琴音 → 唤醒 → 呼叫键，或提醒助手 → 提醒设置", "新用户的呼叫键，默认 Ctrl+Backquote（Ctrl + ~；单独的 ~ 是英雄联盟的“只攻击英雄”键，别设成它）：在游戏里按一下，她就在气泡里听你说，说完自动交给她。0.1.x 升级用户这里是原来的提醒键，呼叫键另存在 assistant.companionBindings.hotkey；空 = 不用。", NONE, NOW),
    f("assistant.companionBindings.hotkey", "升级用户的呼叫键", "琴音 → 唤醒 → 呼叫键", "仅 0.1.x 升级用户启用琴音后使用；按一下在气泡里听，说完自动交给她。", NONE, NOW),
    f("assistant.interactionMode", "提醒键的说话方式（0.1.x 提醒助手；呼叫键固定为说完自动提交）", "提醒助手 → 提醒设置", "只影响 0.1.x 升级用户保留的提醒键；呼叫她不看这一项。", &[("push-to-talk", "按住说话，松开提交"), ("dictation", "按一下开始，停顿自动提交"), ("one-shot", "按一下说一句，说完自动提交")], NOW),
    f("assistant.connectionId", "API 连接", "聊天页 → 她用的模型，或提醒助手 → 提醒设置", "她用哪条连接（连接 ID，用 connections.list 查）。0.1.x 升级用户在此保留原提醒连接，她的连接另存在 assistant.companionBindings.connectionId。", NONE, NOW),
    f("assistant.companionBindings.connectionId", "升级用户的琴音连接", "聊天页 → 她用的模型", "仅 0.1.x 升级用户启用琴音后使用；连接 ID，用 connections.list 查。", NONE, NOW),
    f("assistant.callIdleSeconds", "通话", "琴音 → 唤醒", "秒，0–600，缺省 30。按呼叫键或喊唤醒词就是和她通话：通话里她一直在听，开口就是下一句，不用再按键或喊唤醒词；说“退下 / 安静 / 挂了 / 没事了 / 拜拜”、再按一次呼叫键、或这么久没人说话（她在说、在做、在等确认时不算）就挂断。0 = 不通话：一问一答，答完不再听。", NONE, NOW),
    f("assistant.echoCancellation", "开着音箱也能打断她（回声消除）", "琴音 → 唤醒", "true（缺省）时通话里消掉音箱放出来的声音（她的声音、游戏声），玩家直接开口就能打断她；false 时她说话时不听，要打断就按呼叫键。戴耳机开着也没坏处。", NONE, NOW),
    f("assistant.route", "呼叫交给谁", "琴音 → 唤醒", "", &[("reminders", "内置（琴音 / 旧的单次陪玩）"), ("agent", "转交外部 Agent（需开启工具服务）")], NOW),
    f("assistant.defaultDelivery", "回答方式", "琴音 → 交互 → 声音与弹幕", "数组，元素 voice（语音）/ danmaku（弹幕），1–2 个不重复。", NONE, NOW),
    f("assistant.speechEngine", "本地语音合成", "琴音 → 交互 → 声音与弹幕", "assistant.ttsModelId 为空时使用。", &[("system", "Windows 系统语音"), ("kokoro", "Kokoro 本地中文语音（需下载）")], NOW),
    f("assistant.ttsModelId", "在线语音合成", "设置 → 模型 → 语音合成", "空 = 自动（默认）：她用的 API 连接是 DeepSeek 且有 Key 时用 DeepSeek 语音，否则用本地的 assistant.speechEngine。在线语音按字数计费。一般用 models.activate {category: tts}。", &[("", "自动（默认）"), ("local", "本地（assistant.speechEngine）"), ("deepseek-tts", "DeepSeek 语音（在线）"), ("openai-compat-tts", "OpenAI 兼容在线语音"), ("volcano-tts", "火山在线语音")], NOW),
    f("assistant.voiceId", "系统语音音色", "琴音 → 交互 → 声音与弹幕", "Windows 系统语音的音色 ID；空 = 默认。", NONE, NOW),
    f("assistant.kokoroVoiceId", "Kokoro 音色", "琴音 → 交互 → 声音与弹幕", "如 kokoro:zf_001。", NONE, NOW),
    f("assistant.speechVolume", "语音音量", "琴音 → 交互 → 声音与弹幕", "0–100。", NONE, NOW),
    f("assistant.speechRate", "语速", "琴音 → 交互 → 声音与弹幕", "-10–10，0 为正常。", NONE, NOW),
    f("assistant.danmakuDurationMs", "弹幕显示时间", "琴音 → 交互 → 声音与弹幕", "毫秒，1000–60000。", NONE, NOW),
    f("assistant.danmakuSpeedPxPerSec", "弹幕速度", "琴音 → 交互 → 声音与弹幕", "像素/秒，40–600。", NONE, NOW),
    f("assistant.danmakuFontSize", "弹幕字号", "琴音 → 交互 → 声音与弹幕", "16–72。", NONE, NOW),
    f("assistant.danmakuTopOffsetPx", "弹幕距顶部", "琴音 → 交互 → 声音与弹幕", "像素，0–2160。", NONE, NOW),
    f("assistant.danmakuColor", "弹幕颜色", "琴音 → 交互 → 声音与弹幕", "#RRGGBB。", NONE, NOW),
    f("assistant.danmakuEmote", "弹幕琴音表情", "琴音 → 交互 → 声音与弹幕", "", &[("none", "不显示"), ("hello", "打招呼"), ("cheering", "加油"), ("pointing", "指"), ("amazed", "惊讶")], NOW),
    f("assistant.companion.autoSend", "替你打字后直接发送", "琴音 → 唤醒 → 快捷指令", "false 时只把字打进聊天框，由你按回车。", NONE, NOW),
    f("assistant.companion.quickMessages", "快捷指令", "琴音 → 唤醒 → 快捷指令", "数组：每条绑一个热键，发一句话给琴音或直接运行一个已保存的自动化。整组替换。", NONE, NOW),
    f("agent.inGame", "通话交给她", "琴音 → 权限", "true：通话（呼叫键、唤醒词、快捷指令）交给她，多步、能用全部工具；false：旧的单次陪玩。", NONE, NOW),
    f(
        "agent.progressSpeech",
        "执行中先接一句",
        "琴音 → 权限",
        "1.5 秒还没出声，先用语音说一句“好，我看看”。",
        NONE,
        NOW,
    ),
    f(
        "assistant.bubble.enabled",
        "气泡",
        "琴音 → 交互 → 气泡",
        "呼叫她时，在屏幕角落用气泡显示她在听、在想、在说什么（鼠标会直接穿过去，不挡游戏）。气泡只属于她，悬浮窗只属于语音输入：关掉气泡后只用语音和弹幕回答，不会借用语音输入的悬浮窗。",
        NONE,
        NOW,
    ),
    f(
        "assistant.bubble.corner",
        "气泡位置",
        "琴音 → 交互 → 气泡",
        "放在游戏所在屏幕的哪个角。",
        &[("bottom-right", "右下角"), ("bottom-left", "左下角"), ("top-right", "右上角"), ("top-left", "左上角")],
        NOW,
    ),
    f(
        "assistant.bubble.offsetY",
        "气泡离边缘的距离",
        "琴音 → 交互 → 气泡",
        "逻辑像素，0–800；往屏幕中间挪开一点，避开小地图等。",
        NONE,
        NOW,
    ),
    f(
        "assistant.bubble.alsoDanmaku",
        "回答同时用弹幕",
        "琴音 → 交互 → 气泡",
        "默认只在气泡里显示回答；打开后同时滚动弹幕。",
        NONE,
        NOW,
    ),
    f(
        "agent.autoCards.enabled",
        "自动卡片",
        "琴音 → 交互 → 自动卡片",
        "true（默认）：插件看到游戏里的事，自己往聊天里发一张卡片，不叫她、不花钱（英雄联盟：进加载时“开局战绩”、结算时“这局总结”，一局最多两张）；同时给她留一句很短的说明，之后的对话里她看得到。关了两种卡片都不发，也不弹消息提示。",
        NONE,
        NOW,
    ),
    f(
        "agent.autoCards.popup",
        "消息提示",
        "琴音 → 交互 → 自动卡片",
        "true（默认）：发自动卡片时屏幕右下角弹一个小窗（她的头像 + 一句话，点它打开聊天、定位到那张卡片），不抢焦点，6 秒后自己消失；游戏独占全屏、Kotone 窗口在前台时不弹。关了卡片照发。",
        NONE,
        NOW,
    ),
    f(
        "assistant.wake.enabled",
        "语音唤醒",
        "琴音 → 唤醒 → 语音唤醒",
        "喊唤醒词叫醒她，接着说要做的事。开启后常开麦克风（只在本机检测，不上传），需要先下载约 5 MB 的唤醒模型（wake.model.download）。建议戴耳机。用本地语音识别时，本地识别启动后才开始听；在线识别打开 Kotone 就开始听。",
        NONE,
        NOW,
    ),
    f(
        "assistant.wake.phrase",
        "唤醒词",
        "琴音 → 唤醒 → 语音唤醒",
        "",
        &[("qinyin-qinyin", "琴音琴音"), ("nihao-qinyin", "你好琴音")],
        NOW,
    ),
    f(
        "assistant.wake.sensitivity",
        "唤醒灵敏度",
        "琴音 → 唤醒 → 语音唤醒",
        "",
        &[("low", "低：不容易误唤醒"), ("normal", "中"), ("high", "高：更容易唤醒，也更容易误触发")],
        NOW,
    ),
    f(
        "assistant.wake.ack",
        "唤醒后的应答",
        "琴音 → 唤醒 → 语音唤醒",
        "连着往下说时不应答。",
        &[("voice", "说“嗯？”/“我在”"), ("tone", "提示音"), ("none", "不提示")],
        NOW,
    ),
    f(
        "assistant.wake.noSpeechTimeoutMs",
        "唤醒后等多久",
        "琴音 → 唤醒 → 语音唤醒",
        "毫秒，2000–15000；唤醒后一直不说话就安静结束。",
        NONE,
        NOW,
    ),
    f("agent.hiddenAutomations", "不交给琴音的自动化", "自动化", "已保存自动化的 ID 数组。", NONE, NOW),
    f("search.activeModelId", "联网搜索", "设置 → 模型 → 联网搜索", "琴音的 web.search / web.fetch 用哪个后端：deepseek-search（DeepSeek 搜索与抓取接口，每次成功 0.005 元）或 off（不联网）；空 = 自动（有 DeepSeek 连接就用 DeepSeek）。用 models.activate {category: search} 选择。", NONE, NOW),
    f("ocr.activeModelId", "文字识别模型", "设置 → 模型 → 文字识别", "看游戏聊天用。用 models.activate {category: ocr} 选择。", NONE, NOW),
    // ---- 系统与其他 ----
    f("history.mode", "历史记录", "插件 → 语音输入 → 历史", "", &[("capped", "保留最近若干条"), ("keep-all", "全部保留"), ("off", "不记录")], NOW),
    f("history.maxRecords", "历史记录条数上限", "插件 → 语音输入 → 历史", "history.mode = capped 时有效。", NONE, NOW),
    f("history.includeAudio", "历史记录保存录音", "插件 → 语音输入 → 历史", "", NONE, NOW),
    f("diagnostics.recording", "记录诊断信息", "设置 → 系统", "出问题时导出诊断用（diagnostics.export）。", NONE, NOW),
    f("download.source", "模型下载源", "设置 → 模型", "", &[("auto", "自动（魔搭优先，回退官方）"), ("official", "官方"), ("mirror", "镜像")], NOW),
    f("download.ghProxy", "GitHub 下载代理", "设置 → 模型", "形如 https://ghfast.top/ 的前缀；空 = 不用。", NONE, NOW),
    f("runAsAdminOnStart", "以管理员身份启动", "设置 → 系统", "部分游戏以管理员运行时需要。现在就要管理员权限用 elevation.restart。", NONE, NEXT),
    f("ui.autoStart", "打开 Kotone 时自动启动本地语音识别", "设置 → 模型 → 语音识别", "只对本地语音识别有意义：true 时每次打开 Kotone 就把本地模型加载进内存，语音输入热键、呼叫键和唤醒词马上能用；false 时要先手动启动（启动热键、标题栏、设置 → 模型 → 语音识别或托盘）。在线识别打开 Kotone 就能用，不看这一项；换成在线识别时值保留。这不是开机自启：开机自启是系统启动项，用 autostart.status / autostart.set。", NONE, NEXT),
    f("ui.startPage", "打开软件时显示", "设置 → 系统", "打开 Kotone 窗口时先显示哪一页。空 = 聊天（默认）；不认识的值也按聊天处理。", &[("", "聊天（默认）"), ("chat", "聊天"), ("voiceInput", "语音输入")], NOW),
    f("ui.updatePromptDisabled", "不再提醒升级", "关于", "", NONE, NOW),
    f("agent.identity.name", "琴音的名字", "琴音 → 角色", "空 = 默认“琴音”（或工作区 IDENTITY.md 里的名字）。改了以后她会用新名字介绍自己，界面上也跟着变；唤醒词要在语音唤醒里另外设。", NONE, NOW),
    f("agent.matchRecap", "对局复盘", "琴音 → 权限", "true（默认）：游戏退出时，琴音把这一局的对话、提醒、自动化整理成复盘，写进这个游戏的记忆，并在聊天里留一条；只整理玩了 3 分钟以上、而且这一局里叫过她或跑过自动化的。每次一次 AI 调用。", NONE, NOW),
    f("agent.safetyCheck", "DeepSeek 内容审核", "琴音 → 权限", "true（默认）：用 DeepSeek 时请求带 safety_check，审核没通过的回答会撤回并说明；审核不另收费。只对 DeepSeek 生效。", NONE, NOW),
    f("agent.gameEffort", "通话里的思考强度", "琴音 → 权限", "只对 DeepSeek 生效。通话里空 = 不思考（ADR 029：开着思考实测一句回答要多等十几秒）；玩家明确选了 low / high / max 才思考。", GAME_EFFORTS, NOW),
    f("agent.chatEffort", "聊天窗口里的思考强度", "琴音 → 权限", "只对 DeepSeek 生效。空 = 服务商默认（high）。", EFFORTS, NOW),
    f("plugins.*", "插件", "插件", "插件在左侧栏的“插件”里：每个插件一张卡片（一个独立的 App），卡片上有开关，点开是它自己的界面。按插件 id 一份：enabled（插件开关，true / false；没写按插件的默认）、sources.<开关 id>（插件里的数据开关，没写按默认）、settings（插件自己的设置）。现有插件：lol（英雄联盟，默认开；sources.client 读客户端数据：读英雄联盟客户端和局内的本地接口（只读），查这局玩家的战绩和突破口、记对面交的召唤师技能，默认开；国服用这类功能可能违反腾讯的用户协议，风险由玩家自己决定）；game-voice（游戏语音，默认关：开着就把玩家的麦克风和你“说给队友”的话一起转进虚拟麦克风 VB-CABLE，游戏里的麦克风选 CABLE Output 队友就能听到；Kotone 没运行时队友听不到玩家。settings.includeMic 混入玩家自己的麦克风，默认 true；settings.micDeviceId 玩家的麦克风，设备名，用 audio.devices 查，default = 系统默认录音设备，不能选 CABLE Output；settings.micGain 音量百分比 0–400，默认 100；previousDefaultMic / previousCommunicationsMic 由 voice_chat.default_mic 自动记，不要改）；music（网易云音乐，默认开：网易云的歌由 Kotone 自己放；settings.quality 音质：exhigh（默认，极高 320k，没有网易云 VIP 时最高就是这个）、standard（标准，省流量）、lossless（无损，要网易云 VIP）；settings.volume 音乐音量 0–100，默认 70，她说话时音乐会自动压低、说完恢复，不用为这个改音量）。关掉插件或它的数据开关，它的页面、给你的工具、它的游戏事件都跟着没了；你的工具清单从下一段对话起变。", NONE, NOW),
    f("ui.sidebar.order", "左侧栏页面的顺序", "左侧栏 → ⋯", "页面 id 的顺序，各自在自己的分组里排：游戏陪玩组 assistant（琴音）、game（游戏）、automation（自动化）、plugins（插件：每个插件一张卡片），语音输入组 general（语音输入）。单个插件不在左侧栏，从“插件”的卡片点开（音乐在放的时候左侧栏底部有小播放条）。空 = 默认；没写到的页面按默认位置。玩家说“把自动化放到最上面”就改这里。", NONE, NOW),
    f("ui.sidebar.hidden", "左侧栏隐藏的页面", "左侧栏 → ⋯", "不在左侧栏显示的页面 id（同上）；隐藏了的还能从“⋯ → 全部页面”打开。advanced（设置）不能隐藏。玩家说“别显示语音输入”就把 general 加进来。", NONE, NOW),
    f("agent.compactAtTokens", "对话多长时整理成摘要", "琴音 → 用量", "一段对话估到这么多 token 时，琴音把较早的部分整理成摘要、之后只看摘要和最近几轮（整理那一次多花一点，之后又能命中缓存）。默认 64000；0 = 不整理。", NONE, NOW),
    f("agent.subagents.model", "后台的事用的模型", "聊天页 → 她用的模型", "交给后台的事（例如同传）用的模型 id；空 = 和琴音同一个模型。建议填同一家服务商更快更便宜的模型。", NONE, NOW),
    f("toolService.enabled", "对外工具服务", "设置 → 系统", "开启后外部 Agent（命令行 / MCP / 插件）可以连接本机 Kotone；只监听本机。", NONE, APP),
    f("browser.enabled", "内置浏览器", "设置 → 浏览器", "true（默认）：聊天页右侧栏有“网页”，玩家自己能上网，她在后台的事里能打开网页、替玩家操作（browser.*）。false：右侧栏不再显示“网页”，开着的网页都关掉；她不能在右侧栏打开网页，后台的事里没有 browser.*，她的工具清单从下一段对话起变。再打开就都回来（登录过的账号和扩展都还在）。扩展、清除浏览器登录也在“设置 → 浏览器”，只能由玩家自己动。", NONE, NOW),
];

/// 不给 Agent 改，或要用专用工具改的设置（前缀匹配）。
pub const INTERNAL: &[(&str, &str)] = &[
    (
        "connections",
        "用 connections.upsert / connections.delete；Key 只能由用户在连接页粘贴",
    ),
    ("models.dir", "用 models.dir.set（会迁移已有模型）"),
    ("modelConfigs", "用 models.activate"),
    ("agent.permissions", "琴音的权限只能由用户在设置页修改"),
    (
        "assistant.followUpSeconds",
        "ADR 033 起不再使用（通话取代，用 assistant.callIdleSeconds），只为旧版本保留",
    ),
    ("agent.identity.avatar", "头像要在设置页上传图片"),
    (
        "agent.roleId",
        "换角色用 roles.switch（会同时换名字、头像，开一段新对话）",
    ),
    (
        "assistant.callHotkeyDefaulted",
        "迁移标记：老配置补过默认呼叫键",
    ),
    (
        "assistant.callHotkeyRevision",
        "迁移标记：默认呼叫键改成 Ctrl+` 时，老配置里的旧默认跟着改过",
    ),
    (
        "assistant.companion.check",
        "测试连接的结果，由“测试连接”写入（只用于显示）",
    ),
    ("history.dir", "历史目录"),
    ("language", "识别语言，目前只有中文"),
    ("toolService.ocr", "旧字段，保留兼容"),
    ("ui.firstRunCompleted", "首次向导是否完成"),
    ("ui.legacyUpgrade", "由玩家在升级欢迎页或新手引导中选择"),
    ("adminPromptDismissed", "界面提示状态"),
    ("autoAdminPromptDismissed", "界面提示状态"),
    ("evalRecording", "开发评测用"),
    (
        "browser.extensions",
        "扩展只能由玩家在「设置 → 浏览器」里加、删、开关（ADR 031）",
    ),
    (
        "browser.pendingUninstall",
        "浏览器没开时删掉的扩展，下次打开浏览器时卸掉（内部记录）",
    ),
];

/// 内置 Agent 改设置时不允许出现的路径（权限不能由 Agent 自己改）：
/// - `agent.permissions`：她自己的权限（模式、每个工具的规则，含第三方工具的“总是允许”）；
/// - `plugins.*.settings.trusted`：第三方插件的信任（记的是玩家信任时那份配置的指纹，MCP 方案第 2 期）。
///   信任以后它说只读的工具就不再问，只能由玩家在「插件」里点（审查后加的，2026-11-25）。
///
/// 中间的 `*` 是任意一段（插件 id 里没有点）。
pub const AGENT_PROTECTED: &[&str] = &["agent.permissions", "plugins.*.settings.trusted"];

fn path_matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return path == prefix || path.starts_with(&format!("{prefix}."));
    }
    if pattern.contains('*') {
        // 一段一段比，`*` 是任意一段；路径比规则长（往下的字段）也算
        let wanted: Vec<&str> = pattern.split('.').collect();
        let got: Vec<&str> = path.split('.').collect();
        return got.len() >= wanted.len()
            && wanted.iter().zip(&got).all(|(w, g)| *w == "*" || w == g);
    }
    path == pattern || path.starts_with(&format!("{pattern}."))
}

/// 某个路径对应的说明（映射类按前缀）。
pub fn field_for(path: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| path_matches(f.path, path))
}

fn internal_for(path: &str) -> Option<&'static (&'static str, &'static str)> {
    INTERNAL.iter().find(|(p, _)| path_matches(p, path))
}

/// 按关键词查（匹配路径、名称、位置、说明与可选值；空关键词返回全部）。
pub fn describe(query: Option<&str>) -> Vec<&'static Field> {
    let terms: Vec<String> = query
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    FIELDS
        .iter()
        .filter(|f| {
            if terms.is_empty() {
                return true;
            }
            let hay = format!(
                "{} {} {} {} {}",
                f.path,
                f.label,
                f.page,
                f.description,
                f.values
                    .iter()
                    .map(|(v, l)| format!("{v} {l}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            terms.iter().any(|t| hay.contains(t.as_str()))
        })
        .collect()
}

/// 两份设置（JSON）之间变化了的叶子路径（数组整体算一个叶子）。
pub fn changed_paths(old: &Value, new: &Value) -> Vec<String> {
    let mut out = Vec::new();
    diff("", old, new, &mut out);
    out
}

fn diff(prefix: &str, old: &Value, new: &Value, out: &mut Vec<String>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                diff(
                    &path,
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        _ if old != new => out.push(prefix.to_string()),
        _ => {}
    }
}

/// 一项设置在界面上的名字和位置（如“琴音 → 唤醒 → 呼叫键”）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub path: String,
    pub label: String,
    pub page: String,
}

/// 改动的生效方式汇总（`settings.update` 返回给 Agent）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Effects {
    pub changed: Vec<String>,
    /// 改了的设置项在界面上的位置（聊天里的工具卡片据此给出“去看看”的跳转）。
    pub locations: Vec<Location>,
    /// 语音识别正在运行，要重启它（`runtime.start`）才生效。
    pub restart_voice: bool,
    /// 要重启 Kotone（`app.restart`）才生效的项。
    pub restart_app: Vec<String>,
    /// 下次启动生效的项。
    pub next_launch: Vec<String>,
    /// 给用户看的一句话。
    pub summary: String,
}

/// `runtime_restart_needed`：改完后语音识别的实际状态（运行中且配置与启动时不同）。
pub fn effects(changed: Vec<String>, runtime_restart_needed: bool) -> Effects {
    let mut restart_app = Vec::new();
    let mut next_launch = Vec::new();
    for path in &changed {
        match field_for(path).map(|f| f.effect) {
            Some(Effect::RestartApp) => restart_app.push(path.clone()),
            Some(Effect::NextLaunch) => next_launch.push(path.clone()),
            _ => {}
        }
    }
    let summary = if changed.is_empty() {
        "没有改动".to_string()
    } else {
        let mut parts = Vec::new();
        if !restart_app.is_empty() {
            parts.push("需要重启 Kotone 才生效（可以用 app.restart，先问用户）".to_string());
        }
        if runtime_restart_needed {
            parts.push("需要重启语音识别才生效（可以用 runtime.start，先问用户）".to_string());
        }
        if !next_launch.is_empty() {
            parts.push("部分改动下次启动 Kotone 时生效".to_string());
        }
        if parts.is_empty() {
            "已保存，立即生效".to_string()
        } else {
            format!("已保存；{}", parts.join("；"))
        }
    };
    let mut locations: Vec<Location> = Vec::new();
    for path in &changed {
        if let Some(field) = field_for(path) {
            if !locations.iter().any(|l| l.label == field.label) {
                locations.push(Location {
                    path: path.clone(),
                    label: field.label.to_string(),
                    page: field.page.to_string(),
                });
            }
        }
    }
    Effects {
        changed,
        locations,
        restart_voice: runtime_restart_needed,
        restart_app,
        next_launch,
        summary,
    }
}

/// 内置 Agent 的设置补丁里不允许出现的路径；有则返回原因。
pub fn agent_protected_violation(patch: &Value) -> Option<String> {
    // 补丁里写到的每一个叶子（和空对象）：`diff` 从空对象比只到第一层（`plugins`），往下的规则对不上
    fn leaves(prefix: &str, value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                for (key, child) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    leaves(&path, child, out);
                }
            }
            _ => out.push(prefix.to_string()),
        }
    }
    let mut paths = Vec::new();
    leaves("", patch, &mut paths);
    for path in paths {
        if let Some(protected) = AGENT_PROTECTED.iter().find(|p| path_matches(p, &path)) {
            if protected.starts_with("plugins.") {
                return Some(format!(
                    "{path}：第三方插件信不信任只能由玩家在左侧栏的「插件」里这个插件的设置里改，琴音不能替玩家信任"
                ));
            }
            return Some(format!(
                "{protected} 只能由用户在「琴音 → 权限」里修改，琴音不能改自己的权限"
            ));
        }
    }
    // 扩展能读写浏览器里所有登录过的网站：只能由玩家自己加（ADR 031）。后台能写文件（ADR 030），
    // 不能让它写个扩展再自己登记上。内置浏览器的开关（`browser.enabled`）她可以照玩家的话改。
    if let Some(browser) = patch.get("browser") {
        let only_switch = browser
            .as_object()
            .is_some_and(|b| b.keys().all(|key| key == "enabled"));
        if !only_switch {
            return Some("扩展只能由玩家自己在「设置 → 浏览器」里加、删、开关，她不能改".into());
        }
    }
    // 整个对象写成 {"agent": {"permissions": ...}} 以外的形式（如 null）也不放过
    if patch
        .get("agent")
        .is_some_and(|a| !a.is_object() || a.get("permissions").is_some())
    {
        return Some("agent.permissions 只能由用户修改，琴音不能改自己的权限".into());
    }
    None
}

/// 说明里没写、也不在 INTERNAL 里的路径（测试与诊断用）。
pub fn undocumented(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| field_for(p).is_none() && internal_for(p).is_none())
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn leaves(v: &Value, prefix: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(m) if !m.is_empty() => {
                for (k, c) in m {
                    let p = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    leaves(c, &p, out);
                }
            }
            _ => out.push(prefix.to_string()),
        }
    }

    fn default_json() -> Value {
        serde_json::to_value(Settings::default()).unwrap()
    }

    #[test]
    fn every_setting_is_documented_or_internal() {
        let mut paths = Vec::new();
        leaves(&default_json(), "", &mut paths);
        assert!(paths.len() > 60);
        assert_eq!(undocumented(&paths), Vec::<String>::new());
    }

    #[test]
    fn documented_paths_exist_and_are_unique() {
        let mut settings = Settings::default();
        settings.assistant.companion_bindings =
            Some(crate::assistant::CompanionBindings::default());
        let json = serde_json::to_value(settings).unwrap();
        let mut seen = std::collections::HashSet::new();
        for field in FIELDS {
            assert!(seen.insert(field.path), "重复的说明 {}", field.path);
            assert!(!field.label.is_empty() && !field.page.is_empty());
            let base = field.path.trim_end_matches(".*");
            let mut node = &json;
            for part in base.split('.') {
                node = node
                    .get(part)
                    .unwrap_or_else(|| panic!("设置里没有 {}", field.path));
            }
        }
    }

    fn set(json: &mut Value, path: &str, value: Value) {
        let mut node = json;
        let parts: Vec<&str> = path.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            node = node.get_mut(*part).unwrap();
        }
        node[parts[parts.len() - 1]] = value;
    }

    #[test]
    fn every_listed_value_is_accepted() {
        for field in FIELDS.iter().filter(|f| !f.values.is_empty()) {
            for (value, _) in field.values {
                let mut json = default_json();
                set(&mut json, field.path, Value::String((*value).into()));
                let parsed: Settings = serde_json::from_value(json)
                    .unwrap_or_else(|e| panic!("{} = {value:?} 不被接受：{e}", field.path));
                parsed
                    .assistant
                    .validate()
                    .unwrap_or_else(|e| panic!("{} = {value:?} 校验失败：{e}", field.path));
            }
        }
    }

    #[test]
    fn describe_matches_labels_values_and_paths() {
        let hits = describe(Some("悬浮窗"));
        assert!(hits.iter().any(|f| f.path == "overlay.style"));
        assert!(describe(Some("bottom_right"))
            .iter()
            .any(|f| f.path == "overlay.position"));
        assert!(describe(Some("vadSilenceMs")).len() == 1);
        assert_eq!(describe(None).len(), FIELDS.len());
    }

    #[test]
    fn changed_paths_and_effects() {
        let old = default_json();
        let mut new = old.clone();
        set(&mut new, "overlay.style", "card".into());
        set(&mut new, "toolService.enabled", true.into());
        set(&mut new, "runAsAdminOnStart", true.into());
        let changed = changed_paths(&old, &new);
        assert_eq!(
            changed,
            vec!["overlay.style", "runAsAdminOnStart", "toolService.enabled"]
        );
        let e = effects(changed, true);
        assert!(e.restart_voice);
        assert_eq!(e.restart_app, vec!["toolService.enabled"]);
        assert_eq!(e.next_launch, vec!["runAsAdminOnStart"]);
        assert!(e.summary.contains("重启 Kotone") && e.summary.contains("重启语音识别"));
        let quiet = effects(vec!["overlay.style".into()], false);
        assert_eq!(quiet.summary, "已保存，立即生效");
        assert_eq!(effects(Vec::new(), false).summary, "没有改动");
    }

    /// 位置要用 0.2 的界面结构写（ADR 028）：前端按这些写法跳转（navigation.ts parseLocation），
    /// 旧写法（琴音 → 设置、其他热键、音效、交流）不能再出现在说明表里。
    /// 2026-11-25（交接第十一节第 8 条）：原「设置」的陪玩组搬进角色页（琴音 → 唤醒 / 交互 / 权限 / 用量），
    /// 她用的模型在聊天页输入框旁（聊天页 → 她用的模型）；「设置」只剩原来的通用组。
    #[test]
    fn every_page_uses_the_new_ui_structure() {
        const TOP: &[&str] = &[
            "语音输入",
            "琴音",
            "聊天页",
            "设置",
            "游戏列表",
            "自动化",
            "音乐",
            "插件",
            "左侧栏",
            "提醒助手",
            "关于",
        ];
        const SECOND: &[(&str, &[&str])] = &[
            (
                "语音输入",
                &["输入", "文字处理", "历史", "悬浮窗", "交互模式"],
            ),
            (
                "琴音",
                &["角色", "记忆", "技能", "唤醒", "交互", "权限", "用量"],
            ),
            ("聊天页", &["她用的模型"]),
            (
                "设置",
                &[
                    "模型",
                    "API 连接",
                    "声音",
                    "调参",
                    "外观",
                    "系统",
                    "浏览器",
                    "插件",
                    "热键",
                ],
            ),
            ("提醒助手", &["提醒设置"]),
        ];
        for field in FIELDS {
            for location in field.page.split("，或") {
                let parts: Vec<&str> = location.split('→').map(str::trim).collect();
                assert!(
                    TOP.contains(&parts[0]),
                    "{}：位置 {location} 的首段不认识",
                    field.path
                );
                if let (Some(second), Some((_, allowed))) = (
                    parts.get(1),
                    SECOND.iter().find(|(top, _)| *top == parts[0]),
                ) {
                    assert!(
                        allowed.contains(second),
                        "{}：位置 {location} 的第二段不认识",
                        field.path
                    );
                }
                for old in [
                    "设置 → 其他热键",
                    "音效",
                    "琴音 → 设置",
                    "交流",
                    "历史记录",
                    "怎么叫她",
                    "她怎么回答",
                    "她是谁",
                    "设置 → 权限",
                    "设置 → 用量",
                ] {
                    assert!(
                        !location.contains(old),
                        "{}：位置 {location} 还是旧写法",
                        field.path
                    );
                }
            }
            for banned in [
                "陪玩热键",
                "启动 Kotone 后生效",
                "右上角「启动」",
                "琴音陪玩",
            ] {
                assert!(
                    !field.label.contains(banned) && !field.description.contains(banned),
                    "{}：说明里还有“{banned}”",
                    field.path
                );
            }
        }
        assert!(
            agent_protected_violation(&serde_json::json!({"agent": {"permissions": {}}}))
                .is_some_and(|reason| !reason.contains("琴音陪玩"))
        );
    }

    #[test]
    fn engine_options_map_is_covered_by_prefix() {
        assert_eq!(
            field_for("engineOptions.sherpa-streaming.provider").map(|f| f.path),
            Some("engineOptions.*")
        );
        assert!(internal_for("connections").is_some());
    }

    #[test]
    fn agent_cannot_touch_its_own_permissions() {
        use serde_json::json;
        assert!(agent_protected_violation(&json!({"overlay": {"style": "card"}})).is_none());
        assert!(agent_protected_violation(&json!({"agent": {"inGame": false}})).is_none());
        assert!(agent_protected_violation(
            &json!({"agent": {"permissions": {"mode": "allow-all"}}})
        )
        .is_some());
        assert!(
            agent_protected_violation(&json!({"agent": {"permissions": {"rules": {}}}})).is_some()
        );
        assert!(agent_protected_violation(&json!({"agent": null})).is_some());
        // 扩展只能由玩家加；内置浏览器的开关她可以照玩家的话改
        assert!(
            agent_protected_violation(&json!({"browser": {"extensions": [{"path": "C:/x"}]}}))
                .is_some()
        );
        assert!(
            agent_protected_violation(&json!({"browser": {"pendingUninstall": ["x"]}})).is_some()
        );
        assert!(agent_protected_violation(
            &json!({"browser": {"enabled": true, "extensions": []}})
        )
        .is_some());
        assert!(agent_protected_violation(&json!({"browser": null})).is_some());
        assert!(agent_protected_violation(&json!({"browser": {"enabled": false}})).is_none());
    }

    /// 第三方插件的信任（MCP 方案第 2 期）她不能自己写；插件别的设置、开关照旧能改。
    #[test]
    fn agent_cannot_trust_third_party_plugins() {
        use serde_json::json;
        let trust = agent_protected_violation(
            &json!({"plugins": {"everything": {"settings": {"trusted": "0123456789abcdef"}}}}),
        )
        .expect("信任不能由她写");
        assert!(trust.contains("「插件」"), "{trust}");
        assert!(agent_protected_violation(
            &json!({"plugins": {"files": {"enabled": true, "settings": {"trusted": {"x": 1}}}}})
        )
        .is_some());
        assert!(
            agent_protected_violation(&json!({"plugins": {"everything": {"enabled": false}}}))
                .is_none()
        );
        assert!(agent_protected_violation(
            &json!({"plugins": {"music": {"settings": {"quality": "standard", "volume": 50}}}})
        )
        .is_none());
        assert!(path_matches(
            "plugins.*.settings.trusted",
            "plugins.a-b.settings.trusted"
        ));
        assert!(!path_matches(
            "plugins.*.settings.trusted",
            "plugins.a.settings.trustedness"
        ));
        assert!(!path_matches(
            "plugins.*.settings.trusted",
            "plugins.a.settings"
        ));
        // 原来的写法不变
        assert_eq!(
            field_for("plugins.lol.enabled").map(|f| f.path),
            Some("plugins.*")
        );
    }

    /// 内置浏览器的开关有说明、在“设置 → 浏览器”；扩展和待卸载的记录不给她改（ADR 038）。
    #[test]
    fn browser_switch_is_documented_and_extensions_stay_internal() {
        let field = field_for("browser.enabled").expect("browser.enabled 要有说明");
        assert_eq!(field.path, "browser.enabled");
        assert_eq!(field.page, "设置 → 浏览器");
        assert!(
            field.description.contains("browser.*") && field.description.contains("下一段对话")
        );
        assert!(field_for("browser.extensions").is_none());
        assert!(internal_for("browser.extensions").is_some());
        assert!(internal_for("browser.pendingUninstall").is_some());
        assert!(internal_for("browser.enabled").is_none());
        let e = effects(vec!["browser.enabled".into()], false);
        assert_eq!(e.summary, "已保存，立即生效");
        assert_eq!(e.locations[0].page, "设置 → 浏览器");
        for (_, reason) in INTERNAL.iter().filter(|(p, _)| p.starts_with("browser")) {
            assert!(
                reason.contains("扩展") && !reason.contains("插件"),
                "{reason}"
            );
        }
    }
}
