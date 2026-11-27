# ADR 048：她的声音在一个地方配——选服务商、就地填 Key、选声音；加 MiniMax、阿里云百炼、硅基流动

状态：Accepted（2026-11-27）/ 第 1 步（后端）、第 2 步（界面）已实现，待真 Key（见文末“实现情况”）。

修订：[ADR 017](017-voice-wake.md) 的 TTS 一行（在线引擎不止 OpenAI 兼容和火山）、[ADR 024](024-prefix-cache-compaction-usage.md) 的价格表（加 MiniMax、百炼）、[ADR 028](028-game-companion-interaction.md) 的“自动”语音（只认 DeepSeek → 跟着她用的模型走）、[ADR 039](039-desktop-companion-and-game-apps.md) 的设置结构（「设置 → 模型」里不再有语音合成，第 2 步做）。调研（2026-11-27，结论写在下面“背景”）：Coopanion（本体不带语音合成，靠扩展市场；看的是 MIT 的自然语音扩展 `guyin24/coopanion-natural-voice`，Coopanion 本体 AGPL 只看做法）、airi（MIT）、Open-LLM-VTuber（MIT）、LobeHub、SillyTavern（AGPL，只看界面），以及 MiniMax、阿里云百炼、火山、硅基流动的官方文档。

## 背景

玩家实测 rc.14：

> “tts模型选择有点问题，现在这个选择默认需要选择API连接，我建议参考一下git上成熟的应用是怎么让用户配置tts的，比如 Coopanion，而且我建议增加一些tts供应商，比如minimax”

现在选了在线声音（OpenAI 兼容 / 火山），页面只让你去「设置 → 模型 → 语音合成」；那里第一项是“API 连接（必填）”，没有就得去「设置 → API 连接」建一条聊天用的连接（要填聊天模型名），再回来选。火山的 Access Token 还要“放在一条连接里”。MiniMax 没有：它没有 OpenAI 的 `/v1/audio/speech`（不带 Key 实测 404），拿 MiniMax 连接配“OpenAI 兼容”只会看到 HTTP 404。

别人的做法：Coopanion 的语音扩展（自然语音、气泡朗读、大肥鱼语音包……）都是一个面板——选平台、就地填这家的 Key（每家分开存，大肥鱼包早先只存一份、切换服务商会把上一家的 Key 发过去，后来都改成分开存）、读声音列表、试听；和对话模型的连接完全分开。LobeHub、SillyTavern 是同一家的 Key 对话和语音共用。airi 要先到「服务商」页配好再回来选——这正是玩家卡住的地方。

## 决定

1. **一个地方**：她的声音只在角色页 → 交互 → 声音与弹幕配：选服务商 → 就地填 Key → 选声音（能试听）。「设置 → 模型」里的语音合成那一块去掉，声音页上不出现“API 连接”下拉（第 2 步）。
2. **Key 一家一份**：先用声音设置里为这一家填的 Key（凭据库 `tts:<模型 ID>`）；没填就用同一家 API 连接的 Key（她正在用的那条是这一家就用它，否则第一条；`kotone_core::voice_vendor::vendor_connection`）；再往后是旧写法（`modelConfigs[…].connection` 选的连接、火山复用语音识别的 Access Token）。声音设置不会去新建或改动 API 连接。
3. **站不问玩家**：MiniMax 国内 / 国际、百炼北京 / 新加坡的 Key 不通用。Key 来自连接就跟着连接的地址；自己填的 Key 保存时先试国内，Key 被拒再试国际，记下能用的（`modelConfigs[…].region`）。
4. **保存时先试一次**：只用不花钱的接口（MiniMax 列声音、百炼 / 硅基流动 / DeepSeek 列模型）；火山和自定义接口没有这种接口，就合成一个字。Key 被拒就不存；网络这类原因没试通照样存（免得白填）。
5. **“自动”跟着她用的模型走**（默认）：她的 API 连接是 DeepSeek / MiniMax / 阿里云百炼、那家有 Key，就用那家的默认声音（海星 / 少女音色 / 芊悦）；不是就用 Windows 系统语音。原来“只认 DeepSeek”的特例换成一张表（`voice_vendor::AUTO_ENGINES`）。页面要写明现在用的是哪家、怎么计费（`kotone_tts::catalog::PROVIDERS` 的 `billing`）。
6. **这一轮加三家**：MiniMax（`/v1/t2a_v2`，非流式 WAV + hex；HTTP 200 里也可能是失败，先看 `base_resp.status_code`，错误码翻成一句中文；`/v1/get_voice` 列声音，含复刻的）、阿里云百炼 Qwen3-TTS-Flash（返回一个 WAV 链接：只从官方存储 `dashscope-result-*.oss-*.aliyuncs.com` 下，改走 https，不带 Key、不跟随跳转、限 16 MB；一次最多 600 字，先分段）、硅基流动（OpenAI 兼容接口的预设，声音写成 `<模型>:<声音>`）。所有在线请求都不跟随跳转，Key 不会被带去别的地址。
7. **“OpenAI 兼容”改叫“自定义（OpenAI 兼容接口）”**，去掉写死的默认模型和声音（OpenAI 官方的 tts-1、tts-1-hd、gpt-4o-mini-tts 2027-01-06 停用）；可以没有 Key（本机的语音服务）。以前选了它、没填模型 / 声音的旧配置，读配置时照以前的效果写明 `gpt-4o-mini-tts` / `alloy`，照样能用。
8. **模型配置里的 Key 不进 `config.json`**：语音识别里火山的 Access Token、讯飞的 APISecret 以前明文写在 `modelConfigs`。现在读配置时挪进凭据库（`asr:volcano-asr`、`asr:iflytek-asr`；凭据库里还没有才写，写不进去就先留着、下次启动再试），保存设置时补丁里写进来的也存进凭据库（覆盖；空的当“不改”；存不进去就报错，不改成明文存）。会话配置要用时从凭据库补上（`stt::add_remote_connection`），引擎查“配好没有”也看凭据库。名字是 `<类别>:<模型 ID>`，和插件的 `<插件 id>:<名字>` 一个写法。
9. **不接 Edge TTS**（非官方接口，2026-11-15 玩家定的，`docs/product-evolution-game-tools.md`）。

## 设置文件兼容

照 `docs/development.md` 的规矩：选的是哪家继续写 `assistant.ttsModelId`（新值 `minimax-tts` / `dashscope-tts` / `siliconflow-tts`），`speechEngine` 只写 `system` / `kokoro`；每家的声音、模型、站在 `modelConfigs[<模型 ID>]`；Key 只在凭据库。这一版读到不认识的 `ttsModelId` 照旧版本的读法用本地引擎。**已知的坑**：rc.14 及更早的版本保存设置时会校验 `ttsModelId`，选了新三家以后装回旧版本，保存设置会报“在线语音合成模型无效”，改回别的声音就好（读配置不受影响）。

## 实现情况

第 1 步（后端，2026-11-27，子代理 T）：
- `kotone-tts`：`online.rs`（MiniMax、百炼、硅基流动；不跟随跳转；`check` / `detect_region` / `minimax_voices`）、`catalog.rs`（服务商表 `PROVIDERS`：显示名、计费、拿 Key 的地方、默认模型和声音；MiniMax 58 个普通话声音、百炼 48 个、硅基流动 8 个内置列表）、`voice.rs`（`minimax/`、`dashscope/`、`siliconflow/` 前缀）、`cache.rs`（`disk::also`：试听那句也落盘）。
- `kotone-core`：`TtsEngine` 加三家、`tts_engine_auto` 改成跟着她的模型走；`voice_vendor.rs`（哪条连接是哪一家的声音、哪个站）；`model_secrets.rs`（Key 字段表、迁移、保存、补进会话配置）；`usage.rs`（MiniMax turbo 2 元 / hd 3.5 元、百炼 0.8 元每万字符，汉字算 2 个）。
- 桌面端：`tts_online.rs`（按第 2 条的顺序解析 Key、站、模型、声音；`KeySource`）、`speech_cmd.rs`（`speech_providers`、`speech_key_save`（先试一次、判断站）、`speech_key_clear`、`speech_check`、`speech_voices`、`speech_try_voice`）、启动时迁移明文 Key、保存设置时 `model_secrets::take`；工具 `speech.status` 多了 `engine`、`providers` 和可选的 `check`。
- 测试：本机模拟服务（wiremock）覆盖 MiniMax 成功、HTTP 200 里的 1004 / 1008、1004 后改试国际站、超大响应、跳转不跟；百炼下载不带 Key、非官方地址不下、401 翻成中文、超大音频；硅基流动预设的请求体；Key 的查找顺序、“自动”的表、明文 Key 迁移与保存。

第 2 步（界面，2026-11-28，子代理 T，分支 `claude/voice-settings`）：
- 声音与弹幕页（`SpeechSettings.svelte`）：服务商下拉（自动 · 系统语音 · Kokoro · 在线的六家）；选了在线的一家，就地显示这一家要的东西——Key（密码框，“保存”先试一次，Key 被拒就不存，结果写在下面，MiniMax / 百炼带上“国内站 / 国际站”）、Key 从哪来（“✓ 已保存，Key 交给 Windows 保管” / “✓ 用的是 API 连接「…」的 Key” / 火山“和语音识别共用”，可以“换一个”“清掉”）、火山的 App ID、自定义接口的地址 / 合成模型 / 声音名；声音下拉旁边“试听”（`speech_try_voice`）；语速对不能调的那几家（DeepSeek、百炼）是灰的并写明；页面底下写明每次说话会把文字发给谁、怎么计费（按服务商取界面文案）；MiniMax 的“更多”里选 Turbo / HD。“自动”写明现在用的是哪一家（`speech_current`）。
- 「设置 → 模型」：语音合成那一栏只剩一行“她的声音在角色页的「交互 → 声音与弹幕」里选”和“去看看”；模型配置里的 `secret` 字段（火山 Access Token、讯飞 APISecret）显示成密码框，存了就写“已保存（交给 Windows 保管，这里不显示）”，不回显（`model_secret_saved`）。
- 文案：`docs/glossary.md`（声音一行、“试听”也指先听一下声音或音效）、`docs/assistant.md`、`PRIVACY.md`、`settings_guide.rs`（`assistant.ttsModelId` 的位置和取值）、`kotone-config/SKILL.md`；游戏语音“系统语音不能送进去”的提示改指声音与弹幕页。
- 没做：“自动”时在线声音出错（Key 过期、余额不足）这一句改用系统语音念——边生成边念的那条路是按句先合成、排队播，失败的那句换系统语音会和前后句的声音叠在一起，要在 `SpeechStream` 里加一种“这一句改用系统语音念完再接着播”的处理，留到下一轮。

真 Key 到了以后要测：MiniMax 国内 / 国际各一个 Key 保存时判断站对不对、合成一句（turbo 约 0.006 元）、`get_voice` 里中文声音的 `voice_name` 长什么样；百炼合成一句、音频链接的实际域名在不在白名单里、北京 / 新加坡判断；硅基流动合成一句。
