//! 统一模型清单的 TTS 类别，以及每家声音的服务商表（ADR 048，docs/product-evolution-game-tools.md §11.3）。
//!
//! 当前语音合成模型由 `assistant.speechEngine`（系统语音 / Kokoro）与新字段 `assistant.ttsModelId`
//! （在线引擎）共同决定，见 `AssistantConfig::tts_engine`；在线引擎不写进旧字段。
//! 声音编号写成 `提供方/声音`，例如 `system/<Windows 语音 token>`、`kokoro/zf_001`、`minimax/female-shaonv`、
//! `dashscope/Cherry`、`siliconflow/FunAudioLLM/CosyVoice2-0.5B:claire`。在线条目的设置（声音、模型、站）存在
//! `settings.modelConfigs[<模型 ID>]`，Key 只在凭据库（`tts:<模型 ID>`，`kotone_core::model_secrets`），
//! 没填就用同一家 API 连接的 Key（`kotone_core::voice_vendor`）。Edge TTS 不接入（非官方接口，2026-11-15 定的）。

use kotone_core::assistant::{
    TtsEngine, DASHSCOPE_TTS_MODEL_ID, DEEPSEEK_TTS_MODEL_ID, MINIMAX_TTS_MODEL_ID,
    OPENAI_TTS_MODEL_ID, SILICONFLOW_TTS_MODEL_ID, VOLCANO_TTS_MODEL_ID,
};
use kotone_core::model_catalog::{
    schema_of, CatalogBackend, CatalogModel, ModelCategory, StaticConfigField,
};
use kotone_core::speech::SpeechVoice;

use crate::online::OnlineKind;

/// Windows 系统语音（SAPI），系统自带，无需下载。
pub const SYSTEM_MODEL_ID: &str = "system-speech";
/// Kokoro 中文（sherpa-onnx int8 v1.1 包，约 147 MB）。
pub const KOKORO_MODEL_ID: &str = "kokoro-zh";
/// 自定义（OpenAI 兼容接口）。
pub const OPENAI_MODEL_ID: &str = OPENAI_TTS_MODEL_ID;
/// 火山语音合成（在线）。
pub const VOLCANO_MODEL_ID: &str = VOLCANO_TTS_MODEL_ID;
/// DeepSeek 官方语音合成（在线）。
pub const DEEPSEEK_MODEL_ID: &str = DEEPSEEK_TTS_MODEL_ID;
/// MiniMax 语音合成（在线）。
pub const MINIMAX_MODEL_ID: &str = MINIMAX_TTS_MODEL_ID;
/// 阿里云百炼 Qwen3-TTS-Flash（在线）。
pub const DASHSCOPE_MODEL_ID: &str = DASHSCOPE_TTS_MODEL_ID;
/// 硅基流动（在线，OpenAI 兼容接口的预设）。
pub const SILICONFLOW_MODEL_ID: &str = SILICONFLOW_TTS_MODEL_ID;
/// Kokoro 包下载大小（tar.bz2）。
pub const KOKORO_ARCHIVE_BYTES: u64 = 147_031_220;

/// 一家在线声音：显示名、拿 Key 的地方、怎么计费（页面上照写，ADR 048）。
#[derive(Debug, Clone, Copy)]
pub struct Provider {
    pub kind: OnlineKind,
    pub model_id: &'static str,
    pub display_name: &'static str,
    /// 一句话说明。
    pub description: &'static str,
    /// 怎么计费（给玩家看的一句话）。
    pub billing: &'static str,
    pub api_key_url: Option<&'static str>,
    /// 默认的合成模型（空 = 要自己填，或者这家不分模型）。
    pub default_model: &'static str,
    /// 默认的声音（服务商自己的声音名，不带前缀；空 = 要自己选）。
    pub default_voice: &'static str,
    /// 能不能调语速。
    pub rate: bool,
    /// Key 分国内 / 国际站（保存时自动判断）。
    pub regions: bool,
    /// 只有一个 Key（DeepSeek、MiniMax、百炼、硅基流动）；火山要 App ID + Access Token，自定义要地址。
    pub key_only: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        kind: OnlineKind::Deepseek,
        model_id: DEEPSEEK_MODEL_ID,
        display_name: "DeepSeek",
        description: "DeepSeek 官方语音，声音海星（女声）/ 暗潮（男声）；默认用她正在用的 DeepSeek 连接的 Key",
        billing: "按字数计费：约 1.5 元 / 万字符（限时价，原价 3 元），一个汉字算 2 个字符；同样的话只花一次钱",
        api_key_url: Some("https://platform.deepseek.com/api_keys"),
        default_model: "",
        default_voice: "stella",
        rate: false,
        regions: false,
        key_only: true,
    },
    Provider {
        kind: OnlineKind::Minimax,
        model_id: MINIMAX_MODEL_ID,
        display_name: "MiniMax",
        description: "MiniMax 语音（Speech 2.8），普通话声音五十多个，也能用你在 MiniMax 复刻的声音；Key 可以和 MiniMax 的聊天连接共用",
        billing: "按字数计费：Speech 2.8 Turbo 约 2 元 / 万字符、HD 约 3.5 元，一个汉字算 2 个字符；同样的话只花一次钱",
        api_key_url: Some("https://platform.minimaxi.com/user-center/basic-information/interface-key"),
        default_model: crate::online::MINIMAX_DEFAULT_MODEL,
        default_voice: crate::online::MINIMAX_DEFAULT_VOICE,
        rate: true,
        regions: true,
        key_only: true,
    },
    Provider {
        kind: OnlineKind::Dashscope,
        model_id: DASHSCOPE_MODEL_ID,
        display_name: "阿里云百炼",
        description: "通义千问语音（Qwen3-TTS-Flash），48 个声音，含四川话、粤语、上海话等方言；Key 可以和通义千问的聊天连接共用",
        billing: "按字数计费：约 0.8 元 / 万字符（北京），一个汉字算 2 个字符；同样的话只花一次钱",
        api_key_url: Some("https://bailian.console.aliyun.com/?apiKey=1#/api-key"),
        default_model: crate::online::DASHSCOPE_DEFAULT_MODEL,
        default_voice: crate::online::DASHSCOPE_DEFAULT_VOICE,
        rate: false,
        regions: true,
        key_only: true,
    },
    Provider {
        kind: OnlineKind::Siliconflow,
        model_id: SILICONFLOW_MODEL_ID,
        display_name: "硅基流动",
        description: "硅基流动的 CosyVoice2 语音，8 个系统声音",
        billing: "按输入文字的字节数计费（一个汉字约 3 个字节），单价见硅基流动官网；同样的话只花一次钱",
        api_key_url: Some("https://cloud.siliconflow.cn/account/ak"),
        default_model: crate::online::SILICONFLOW_DEFAULT_MODEL,
        default_voice: crate::online::SILICONFLOW_DEFAULT_VOICE,
        rate: true,
        regions: false,
        key_only: true,
    },
    Provider {
        kind: OnlineKind::Volcano,
        model_id: VOLCANO_MODEL_ID,
        display_name: "火山引擎（豆包语音）",
        description: "豆包语音 HTTP 接口；App ID 与 Access Token 没填时复用语音识别里填的那一套（需在控制台为该应用开通语音合成）",
        billing: "按字数计费，单价见火山引擎控制台；同样的话只花一次钱",
        api_key_url: Some("https://console.volcengine.com/speech/app"),
        default_model: crate::online::VOLCANO_DEFAULT_CLUSTER,
        default_voice: "BV700_streaming",
        rate: true,
        regions: false,
        key_only: false,
    },
    Provider {
        kind: OnlineKind::OpenAi,
        model_id: OPENAI_MODEL_ID,
        display_name: "自定义（OpenAI 兼容接口）",
        description: "填地址、模型和声音名，接任何支持 /audio/speech 的服务（本机的语音服务也行）。OpenAI 官方的 tts-1、gpt-4o-mini-tts 2027-01-06 停用",
        billing: "看你接的服务：在线服务一般按字数计费，本机的服务不花钱",
        api_key_url: None,
        default_model: "",
        default_voice: "",
        rate: true,
        regions: false,
        key_only: false,
    },
];

/// 这一家的服务商表项。
pub fn provider(kind: OnlineKind) -> &'static Provider {
    PROVIDERS
        .iter()
        .find(|p| p.kind == kind)
        .expect("每一家都在服务商表里")
}

/// 按清单 ID 找服务商表项。
pub fn provider_by_id(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.model_id == id)
}

const OPENAI_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "connection",
        label: "API 连接（旧写法：用这条连接的地址和 Key）",
        kind: "connection",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "baseUrl",
        label: "接口地址（例如 http://127.0.0.1:8880/v1）",
        kind: "string",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "model",
        label: "合成模型",
        kind: "string",
        default: "",
        options: &[],
        required: true,
    },
    StaticConfigField {
        key: "voice",
        label: "声音（服务商的声音名）",
        kind: "string",
        default: "",
        options: &[],
        required: true,
    },
];

const DEEPSEEK_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "connection",
        label: "DeepSeek 连接（留空 = 用她正在用的 DeepSeek 连接）",
        kind: "connection",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "voice",
        label: "声音",
        kind: "enum",
        default: "stella",
        options: &[("stella", "海星（女声）"), ("tide", "暗潮（男声）")],
        required: true,
    },
];

const MINIMAX_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "voice",
        label: "声音（MiniMax 的声音 ID，如 female-shaonv）",
        kind: "string",
        default: crate::online::MINIMAX_DEFAULT_VOICE,
        options: &[],
        required: true,
    },
    StaticConfigField {
        key: "model",
        label: "模型",
        kind: "enum",
        default: crate::online::MINIMAX_DEFAULT_MODEL,
        options: &[
            ("speech-2.8-turbo", "Speech 2.8 Turbo（快、便宜）"),
            ("speech-2.8-hd", "Speech 2.8 HD（更自然，贵 75%）"),
        ],
        required: false,
    },
    StaticConfigField {
        key: "region",
        label: "站（保存 Key 时自动判断；用连接的 Key 时跟着连接）",
        kind: "enum",
        default: "",
        options: &[("", "自动"), ("cn", "国内"), ("intl", "国际")],
        required: false,
    },
];

const DASHSCOPE_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "voice",
        label: "声音（如 Cherry）",
        kind: "string",
        default: crate::online::DASHSCOPE_DEFAULT_VOICE,
        options: &[],
        required: true,
    },
    StaticConfigField {
        key: "region",
        label: "地域（保存 Key 时自动判断；用连接的 Key 时跟着连接）",
        kind: "enum",
        default: "",
        options: &[("", "自动"), ("cn", "北京"), ("intl", "新加坡")],
        required: false,
    },
];

const SILICONFLOW_SCHEMA: &[StaticConfigField] = &[StaticConfigField {
    key: "voice",
    label: "声音（如 FunAudioLLM/CosyVoice2-0.5B:claire）",
    kind: "string",
    default: crate::online::SILICONFLOW_DEFAULT_VOICE,
    options: &[],
    required: true,
}];

const VOLCANO_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "appId",
        label: "App ID（留空则复用语音识别里填的火山 App ID）",
        kind: "string",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "connection",
        label: "Access Token 所在的连接（旧写法；留空则复用语音识别里填的 Access Token）",
        kind: "connection",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "voice",
        label: "声音",
        kind: "enum",
        default: "BV700_streaming",
        options: &[
            ("BV700_streaming", "灿灿（女声）"),
            ("BV001_streaming", "通用女声"),
            ("BV002_streaming", "通用男声"),
        ],
        required: true,
    },
    StaticConfigField {
        key: "cluster",
        label: "Cluster",
        kind: "string",
        default: crate::online::VOLCANO_DEFAULT_CLUSTER,
        options: &[],
        required: false,
    },
];

fn schema(kind: OnlineKind) -> &'static [StaticConfigField] {
    match kind {
        OnlineKind::OpenAi => OPENAI_SCHEMA,
        OnlineKind::Deepseek => DEEPSEEK_SCHEMA,
        OnlineKind::Minimax => MINIMAX_SCHEMA,
        OnlineKind::Dashscope => DASHSCOPE_SCHEMA,
        OnlineKind::Siliconflow => SILICONFLOW_SCHEMA,
        OnlineKind::Volcano => VOLCANO_SCHEMA,
    }
}

/// 清单 ID → 语音引擎。
pub fn engine_of(id: &str) -> Option<TtsEngine> {
    match id {
        SYSTEM_MODEL_ID => Some(TtsEngine::System),
        KOKORO_MODEL_ID => Some(TtsEngine::Kokoro),
        id => provider_by_id(id).map(|p| engine_of_kind(p.kind)),
    }
}

/// 语音引擎 → 清单 ID。
pub fn model_of(engine: TtsEngine) -> &'static str {
    match kind_of(engine) {
        Some(kind) => provider(kind).model_id,
        None if engine == TtsEngine::Kokoro => KOKORO_MODEL_ID,
        None => SYSTEM_MODEL_ID,
    }
}

/// 在线引擎 → 在线合成的种类（系统语音、Kokoro 是 None）。
pub fn kind_of(engine: TtsEngine) -> Option<OnlineKind> {
    match engine {
        TtsEngine::System | TtsEngine::Kokoro => None,
        TtsEngine::Openai => Some(OnlineKind::OpenAi),
        TtsEngine::Volcano => Some(OnlineKind::Volcano),
        TtsEngine::Deepseek => Some(OnlineKind::Deepseek),
        TtsEngine::Minimax => Some(OnlineKind::Minimax),
        TtsEngine::Dashscope => Some(OnlineKind::Dashscope),
        TtsEngine::Siliconflow => Some(OnlineKind::Siliconflow),
    }
}

/// 在线合成的种类 → 语音引擎。
pub fn engine_of_kind(kind: OnlineKind) -> TtsEngine {
    match kind {
        OnlineKind::OpenAi => TtsEngine::Openai,
        OnlineKind::Volcano => TtsEngine::Volcano,
        OnlineKind::Deepseek => TtsEngine::Deepseek,
        OnlineKind::Minimax => TtsEngine::Minimax,
        OnlineKind::Dashscope => TtsEngine::Dashscope,
        OnlineKind::Siliconflow => TtsEngine::Siliconflow,
    }
}

/// 声音编号的提供方前缀。
pub fn provider_of(engine: TtsEngine) -> &'static str {
    match kind_of(engine) {
        Some(kind) => kind.provider(),
        None if engine == TtsEngine::Kokoro => crate::voice::KOKORO,
        None => crate::voice::SYSTEM,
    }
}

pub fn is_known(id: &str) -> bool {
    engine_of(id).is_some()
}

/// MiniMax 普通话系统声音（官方“系统音色列表”，2026-11-27 核对）：没有 Key、在线列表拉不到时用它，
/// 在线列表里也按它把中文声音排在前面。
const MINIMAX_MANDARIN: &[(&str, &str)] = &[
    ("female-shaonv", "少女音色"),
    ("female-yujie", "御姐音色"),
    ("female-chengshu", "成熟女性音色"),
    ("female-tianmei", "甜美女性音色"),
    ("tianxin_xiaoling", "甜心小玲"),
    ("qiaopi_mengmei", "俏皮萌妹"),
    ("wumei_yujie", "妩媚御姐"),
    ("diadia_xuemei", "嗲嗲学妹"),
    ("danya_xuejie", "淡雅学姐"),
    ("lovely_girl", "萌萌女童"),
    ("Chinese (Mandarin)_Warm_Girl", "温暖少女"),
    ("Chinese (Mandarin)_Crisp_Girl", "清脆少女"),
    ("Chinese (Mandarin)_Soft_Girl", "柔和少女"),
    ("Chinese (Mandarin)_Gentle_Senior", "温柔学姐"),
    ("Chinese (Mandarin)_Warm_Bestie", "温暖闺蜜"),
    ("Chinese (Mandarin)_Sweet_Lady", "甜美女声"),
    ("Chinese (Mandarin)_News_Anchor", "新闻女声"),
    ("Chinese (Mandarin)_Mature_Woman", "傲娇御姐"),
    ("Chinese (Mandarin)_Wise_Women", "阅历姐姐"),
    ("Chinese (Mandarin)_HK_Flight_Attendant", "港普空姐"),
    ("Chinese (Mandarin)_Kind-hearted_Antie", "热心大婶"),
    ("Chinese (Mandarin)_Kind-hearted_Elder", "花甲奶奶"),
    ("Arrogant_Miss", "嚣张小姐"),
    ("female-shaonv-jingpin", "少女音色-beta"),
    ("female-yujie-jingpin", "御姐音色-beta"),
    ("female-chengshu-jingpin", "成熟女性音色-beta"),
    ("female-tianmei-jingpin", "甜美女性音色-beta"),
    ("male-qn-qingse", "青涩青年音色"),
    ("male-qn-jingying", "精英青年音色"),
    ("male-qn-badao", "霸道青年音色"),
    ("male-qn-daxuesheng", "青年大学生音色"),
    ("male-qn-qingse-jingpin", "青涩青年音色-beta"),
    ("male-qn-jingying-jingpin", "精英青年音色-beta"),
    ("male-qn-badao-jingpin", "霸道青年音色-beta"),
    ("male-qn-daxuesheng-jingpin", "青年大学生音色-beta"),
    ("clever_boy", "聪明男童"),
    ("cute_boy", "可爱男童"),
    ("cartoon_pig", "卡通猪小琪"),
    ("bingjiao_didi", "病娇弟弟"),
    ("junlang_nanyou", "俊朗男友"),
    ("chunzhen_xuedi", "纯真学弟"),
    ("lengdan_xiongzhang", "冷淡学长"),
    ("badao_shaoye", "霸道少爷"),
    ("Chinese (Mandarin)_Reliable_Executive", "沉稳高管"),
    ("Chinese (Mandarin)_Unrestrained_Young_Man", "不羁青年"),
    ("Robot_Armor", "机械战甲"),
    ("Chinese (Mandarin)_Humorous_Elder", "搞笑大爷"),
    ("Chinese (Mandarin)_Gentleman", "温润男声"),
    ("Chinese (Mandarin)_Male_Announcer", "播报男声"),
    ("Chinese (Mandarin)_Southern_Young_Man", "南方小哥"),
    ("Chinese (Mandarin)_Gentle_Youth", "温润青年"),
    ("Chinese (Mandarin)_Cute_Spirit", "憨憨萌兽"),
    ("Chinese (Mandarin)_Radio_Host", "电台男主播"),
    ("Chinese (Mandarin)_Lyrical_Voice", "抒情男声"),
    ("Chinese (Mandarin)_Straightforward_Boy", "率真弟弟"),
    ("Chinese (Mandarin)_Sincere_Adult", "真诚青年"),
    ("Chinese (Mandarin)_Stubborn_Friend", "嘴硬竹马"),
    ("Chinese (Mandarin)_Pure-hearted_Boy", "清澈邻家弟弟"),
];

/// 这个 MiniMax 声音是不是普通话系统声音（在线列表里排前面用）。
pub fn is_minimax_mandarin(id: &str) -> bool {
    MINIMAX_MANDARIN.iter().any(|(v, _)| *v == id)
}

/// 阿里云百炼 Qwen3-TTS-Flash 的 48 个系统声音（官方声音列表，2026-11-27 核对）：38 个普通话、10 个方言。
const DASHSCOPE_VOICES: &[(&str, &str)] = &[
    ("Cherry", "芊悦（女）"),
    ("Serena", "苏瑶（女）"),
    ("Chelsie", "千雪（女）"),
    ("Momo", "茉兔（女）"),
    ("Vivian", "十三（女）"),
    ("Maia", "四月（女）"),
    ("Bella", "萌宝（女）"),
    ("Jennifer", "詹妮弗（女）"),
    ("Katerina", "卡捷琳娜（女）"),
    ("Elias", "墨讲师（女）"),
    ("Mia", "乖小妹（女）"),
    ("Bellona", "燕铮莺（女）"),
    ("Bunny", "萌小姬（女）"),
    ("Nini", "邻家妹妹（女）"),
    ("Seren", "小婉（女）"),
    ("Stella", "少女阿月（女）"),
    ("Sonrisa", "索尼莎（女）"),
    ("Sohee", "素熙（女）"),
    ("Ono Anna", "小野杏（女）"),
    ("Ethan", "晨煦（男）"),
    ("Moon", "月白（男）"),
    ("Kai", "凯（男）"),
    ("Nofish", "不吃鱼（男）"),
    ("Ryan", "甜茶（男）"),
    ("Aiden", "艾登（男）"),
    ("Eldric Sage", "沧明子（男）"),
    ("Mochi", "沙小弥（男）"),
    ("Vincent", "田叔（男）"),
    ("Neil", "阿闻（男）"),
    ("Arthur", "徐大爷（男）"),
    ("Pip", "顽屁小孩（男）"),
    ("Bodega", "博德加（男）"),
    ("Alek", "阿列克（男）"),
    ("Dolce", "多尔切（男）"),
    ("Lenn", "莱恩（男）"),
    ("Emilien", "埃米尔安（男）"),
    ("Andre", "安德雷（男）"),
    ("Radio Gol", "拉迪奥·戈尔（男）"),
    ("Sunny", "四川话 · 晴儿（女）"),
    ("Kiki", "粤语 · 阿清（女）"),
    ("Jada", "上海话 · 阿珍（女）"),
    ("Eric", "四川话 · 程川（男）"),
    ("Rocky", "粤语 · 阿强（男）"),
    ("Dylan", "北京话 · 晓东（男）"),
    ("Li", "南京话 · 老李（男）"),
    ("Marcus", "陕西话 · 秦川（男）"),
    ("Roy", "闽南话 · 阿杰（男）"),
    ("Peter", "天津话 · 李彼得（男）"),
];

/// 硅基流动 CosyVoice2 的 8 个系统声音。
const SILICONFLOW_VOICES: &[(&str, &str)] = &[
    ("FunAudioLLM/CosyVoice2-0.5B:claire", "温柔女声（claire）"),
    ("FunAudioLLM/CosyVoice2-0.5B:anna", "沉稳女声（anna）"),
    ("FunAudioLLM/CosyVoice2-0.5B:bella", "激情女声（bella）"),
    ("FunAudioLLM/CosyVoice2-0.5B:diana", "欢快女声（diana）"),
    ("FunAudioLLM/CosyVoice2-0.5B:alex", "沉稳男声（alex）"),
    (
        "FunAudioLLM/CosyVoice2-0.5B:benjamin",
        "低沉男声（benjamin）",
    ),
    ("FunAudioLLM/CosyVoice2-0.5B:charles", "磁性男声（charles）"),
    ("FunAudioLLM/CosyVoice2-0.5B:david", "欢快男声（david）"),
];

/// 在线引擎内置的声音（MiniMax 有 Key 时另外在线拉，见 `online::minimax_voices`；自定义接口的声音名自己填）。
pub fn online_voices(kind: OnlineKind) -> Vec<SpeechVoice> {
    let list: &[(&str, &str)] = match kind {
        OnlineKind::OpenAi => &[],
        OnlineKind::Volcano => &[
            ("BV700_streaming", "灿灿（女声）"),
            ("BV001_streaming", "通用女声"),
            ("BV002_streaming", "通用男声"),
        ],
        OnlineKind::Deepseek => &[("stella", "海星（女声）"), ("tide", "暗潮（男声）")],
        OnlineKind::Minimax => MINIMAX_MANDARIN,
        OnlineKind::Dashscope => DASHSCOPE_VOICES,
        OnlineKind::Siliconflow => SILICONFLOW_VOICES,
    };
    list.iter()
        .map(|(id, name)| SpeechVoice {
            id: format!("{}/{id}", kind.provider()),
            name: (*name).into(),
        })
        .collect()
}

/// 在线条目的就绪状态（由桌面端按设置与凭据库判断）。
#[derive(Debug, Clone, Default)]
pub struct OnlineStatus {
    pub ready: bool,
    pub reason: Option<String>,
}

/// 列表所需的外部状态。
pub struct CatalogState {
    pub active: TtsEngine,
    pub kokoro_installed: bool,
    pub system_error: Option<String>,
    /// 每一家在线声音的状态（没给的当“还没配好”）。
    pub online: Vec<(OnlineKind, OnlineStatus)>,
}

fn online_entry(spec: &Provider, status: &OnlineStatus, active: bool) -> CatalogModel {
    CatalogModel {
        category: ModelCategory::Tts,
        id: spec.model_id.into(),
        engine_id: spec.kind.provider().into(),
        display_name: format!("{}（在线）", spec.display_name),
        description: format!(
            "{}；{}；每次说话都会把文字发给{}",
            spec.description, spec.billing, spec.display_name
        ),
        size_bytes: 0,
        backend: CatalogBackend::Remote,
        downloaded: true,
        ready: status.ready,
        state: if status.ready {
            "ready"
        } else {
            "not-configured"
        }
        .into(),
        reason: status.reason.clone(),
        active,
        recommended: spec.kind == OnlineKind::Deepseek,
        languages: vec!["zh".into()],
        data_egress: "text".into(),
        config_schema: schema_of(schema(spec.kind)),
        api_key_url: spec.api_key_url.map(str::to_string),
        license: "服务商条款".into(),
        files: Vec::new(),
    }
}

/// 列出 TTS 类别的全部条目：DeepSeek、系统语音、Kokoro，然后是其余在线的几家。
pub fn entries(state: &CatalogState) -> Vec<CatalogModel> {
    let active = state.active;
    let status = |kind: OnlineKind| {
        state
            .online
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, s)| s.clone())
            .unwrap_or_default()
    };
    let online = |kind: OnlineKind| {
        online_entry(
            provider(kind),
            &status(kind),
            active == engine_of_kind(kind),
        )
    };
    let mut list = vec![
        online(OnlineKind::Deepseek),
        CatalogModel {
            category: ModelCategory::Tts,
            id: SYSTEM_MODEL_ID.into(),
            engine_id: "system".into(),
            display_name: "Windows 系统语音".into(),
            description: "系统自带，无需下载；声音取决于系统已安装的语音包".into(),
            size_bytes: 0,
            backend: CatalogBackend::Local,
            downloaded: true,
            ready: state.system_error.is_none(),
            state: if state.system_error.is_none() {
                "ready"
            } else {
                "unavailable"
            }
            .into(),
            reason: state.system_error.clone(),
            active: active == TtsEngine::System,
            recommended: false,
            languages: vec!["zh".into()],
            data_egress: "none".into(),
            config_schema: Vec::new(),
            api_key_url: None,
            license: "Windows".into(),
            files: Vec::new(),
        },
        CatalogModel {
            category: ModelCategory::Tts,
            id: KOKORO_MODEL_ID.into(),
            engine_id: "kokoro".into(),
            display_name: "Kokoro 中文（本地神经语音）".into(),
            description: "更自然的本地中文女声，约 147 MB；本机 CPU 合成，不上传文字".into(),
            size_bytes: KOKORO_ARCHIVE_BYTES,
            backend: CatalogBackend::Local,
            downloaded: state.kokoro_installed,
            ready: state.kokoro_installed,
            state: if state.kokoro_installed {
                "ready"
            } else {
                "not-installed"
            }
            .into(),
            reason: None,
            active: active == TtsEngine::Kokoro,
            recommended: true,
            languages: vec!["zh".into()],
            data_egress: "none".into(),
            config_schema: Vec::new(),
            api_key_url: None,
            license: "Apache-2.0".into(),
            files: Vec::new(),
        },
    ];
    list.extend(
        PROVIDERS
            .iter()
            .filter(|p| p.kind != OnlineKind::Deepseek)
            .map(|p| online(p.kind)),
    );
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_entry_follows_the_speech_engine_and_online_entries_send_text() {
        let list = entries(&CatalogState {
            active: TtsEngine::Kokoro,
            kokoro_installed: false,
            system_error: None,
            online: vec![(
                OnlineKind::Volcano,
                OnlineStatus {
                    ready: true,
                    reason: None,
                },
            )],
        });
        assert_eq!(list.len(), 8);
        let deepseek = list.iter().find(|m| m.id == DEEPSEEK_MODEL_ID).unwrap();
        assert!(deepseek.recommended && deepseek.data_egress == "text" && !deepseek.active);
        let kokoro = list.iter().find(|m| m.id == KOKORO_MODEL_ID).unwrap();
        assert!(kokoro.active && !kokoro.ready && kokoro.state == "not-installed");
        let custom = list.iter().find(|m| m.id == OPENAI_MODEL_ID).unwrap();
        assert_eq!(custom.state, "not-configured");
        assert_eq!(custom.display_name, "自定义（OpenAI 兼容接口）（在线）");
        // 自定义接口没有写死的默认模型
        assert!(custom
            .config_schema
            .iter()
            .any(|f| f.key == "model" && f.default.is_empty() && f.required));
        assert!(
            list.iter()
                .find(|m| m.id == VOLCANO_MODEL_ID)
                .unwrap()
                .ready
        );
        for id in [MINIMAX_MODEL_ID, DASHSCOPE_MODEL_ID, SILICONFLOW_MODEL_ID] {
            let entry = list.iter().find(|m| m.id == id).unwrap();
            assert_eq!(entry.data_egress, "text");
            assert!(entry.description.contains("计费"), "{id}");
            // Key 不在清单的配置项里（凭据库）
            assert!(entry.config_schema.iter().all(|f| f.kind != "secret"));
        }
        for engine in [
            TtsEngine::System,
            TtsEngine::Kokoro,
            TtsEngine::Openai,
            TtsEngine::Volcano,
            TtsEngine::Deepseek,
            TtsEngine::Minimax,
            TtsEngine::Dashscope,
            TtsEngine::Siliconflow,
        ] {
            assert_eq!(engine_of(model_of(engine)), Some(engine));
            assert_eq!(
                kind_of(engine).map(engine_of_kind).unwrap_or(engine),
                engine
            );
        }
        assert!(!is_known("edge-tts"));
        assert!(online_voices(OnlineKind::Volcano)[0]
            .id
            .starts_with("volcano/"));
    }

    #[test]
    fn builtin_voice_lists_are_complete_and_prefixed() {
        assert_eq!(online_voices(OnlineKind::Dashscope).len(), 48);
        assert_eq!(online_voices(OnlineKind::Siliconflow).len(), 8);
        assert_eq!(online_voices(OnlineKind::Minimax).len(), 58);
        assert!(online_voices(OnlineKind::OpenAi).is_empty());
        for kind in OnlineKind::ALL {
            let p = provider(kind);
            assert_eq!(p.model_id, model_of(engine_of_kind(kind)));
            if !p.default_voice.is_empty() {
                let default = format!("{}/{}", kind.provider(), p.default_voice);
                assert!(
                    online_voices(kind).iter().any(|v| v.id == default),
                    "{kind:?} 的默认声音不在内置列表里"
                );
            }
        }
        assert!(is_minimax_mandarin("female-shaonv"));
        assert!(!is_minimax_mandarin("English_Graceful_Lady"));
    }
}
