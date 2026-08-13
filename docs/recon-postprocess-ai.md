# Kotone AI 后处理侦察报告（润色 · 翻译 · 2026-08）

> 任务：为 ADR-009 的 Phase 2 选可落地的「重」后处理。范围限于 **AI 润色**（口癖清理、口语转书面、轻度纠错）和 **翻译**（中英为主，可扩日韩）。不覆盖已落地的屏蔽词过滤。  
> 结论先行：**游戏开着时 GPU 基本不可用，4B+ 本地 LLM 不是默认路径。在线先上 OpenAI-compatible + Qwen-MT；本地只保留 ≤2B 的专用小模型。标点/ITN 不要再做一遍——X-ASR 与 Fun-ASR-Nano 已经带了。**

核实日期：2026-08-13。优先采用 2025 下半年至 2026 年的发布与评测；2022–2024 的 NLLB / MADLAD / Opus-MT / 学术 CSC 小模型只作为对照，不当默认方案。

---

## 0. 产品约束（从 Kotone 反推，不是从模型反推）

| 约束 | 技术含义 |
|------|----------|
| 游戏已占 GPU / 部分 CPU | 本地推理默认按 **CPU-only** 设计；GPU offload 只能是「局外 / 用户明确授权」 |
| 发送链路要短 | 后处理预算建议 **p50 ≤ 400ms、p95 ≤ 800ms、硬超时 1.5s 后 fail-open** |
| 典型句长 8–40 汉字 | 生成只有 15–60 token；**TTFT 比吞吐更重要**；thinking / reasoning 模式直接否决 |
| 中文游戏黑话 | 「闪现 / 大龙 / gank / baron」必须原样留下；通用润色最容易把它们改坏 |
| 已有 STT 标点 | X-ASR 自带标点；Fun-ASR-Nano 自带标点 + ITN。独立标点恢复模型优先级极低 |
| 双文本边界（ADR-009） | eval 永远看 `RecognizedText`；只有 `ReadyText` 进 Preview / Injector |
| 凭据不得进 pipeline JSON | 润色 / 翻译只能引用 connection ID |
| 分类已预留 | `writing` / `translation` / `utility`；网络边界 `none` / `local` / `internet` |

行业对照：2026 年听写产品（Wispr Flow、Superwhisper、VoiceInk）的差异已经不在 ASR，而在 **转写后再过一层 LLM 清理**。Wispr Flow 公开路径是云端 ASR + 微调 Llama 做 filler / 口误 / 语气；Superwhisper 是本地 ASR + 可选 BYOK 云端 LLM。Kotone 的 Processing 状态正好对应这一层。

---

## 1. 先拆任务，不要一上来就上大模型

「AI 润色」在游戏聊天里其实是四件不同的事：

| 子任务 | 要不要 LLM | 建议落点 |
|--------|------------|----------|
| 口癖 / 重复词（那个、就是、嗯、啊、然后然后） | 要 | 并进 AI 润色。规则表漏召回、误伤黑话，不做独立工序 |
| 标点 / 数字 ITN | 否（STT 已做） | 不做；只在无标点引擎上才考虑 |
| 口语 → 可发送短句、修 ASR 同音字 | 要，但要「少改」 | 小 LLM 或在线 flash 档 |
| 跨语言发送（国服中文 → 外服英文等） | 要专用翻译，不要通用聊天模型 | 在线 Qwen-MT；本地 HY-MT1.5-1.8B |

学术 CSC/CGEC（MacBERT、Soft-Masked BERT 一类）在 2023 年论文里仍优于当时的通用 LLM，但那是「改错别字」不是「把口语收成能发的话」。2026 年产品侧已经统一走 LLM rewrite；Kotone 不该为 CSC 单独引入一套 2022 年 encoder。

**Thinking / 深度推理必须关。** 一句「闪现交了中路可以压」若先想 1–2 秒再写，体感比不润色更差。Qwen3 / Qwen3.5 一律 `enable_thinking=false`，`max_tokens` 卡在 64–96。

---

## 2. 在线方案

### 2.1 润色（writing）

短句场景下，延迟几乎等于 **TTFT + 一轮 RTT**。生成 30 token 在 200 tok/s 上只要 150ms，在 Groq 上只有几十毫秒。

| 方案 | 档位 | 延迟（公开/第三方，2026） | 中文口语 / 黑话 | 价格（约） | 接入 | 推荐 |
|------|------|---------------------------|-----------------|------------|------|------|
| **通义 Qwen-Turbo / Flash**（北京） | 通用小模型 | 国内 RTT 低，适合国服玩家 | 最强中文理解 | Turbo 国际约 $0.05 / $0.20 / 1M | OpenAI-compat | ★★★★★ 国内默认 |
| **Gemini 2.5 Flash-Lite** | 便宜生产档 | TTFT ≈ 0.31s（Artificial Analysis） | 多语好，黑话一般 | $0.10 / $0.40 / 1M | OpenAI-compat | ★★★★ 海外默认 |
| **Groq Llama 4 Scout / 8B Instant** | 专用推理硅 | TTFT 80–150ms，800+ tok/s | 英文强，中文黑话弱 | Scout 约 $0.50 / $1.50 / 1M | OpenAI-compat | ★★★★ 只要极低延迟 |
| **DeepSeek V4-Flash** | 性价比旗舰 | 官方 TTFT ~1.9s；第三方可到 ~0.8s | 中文强 | $0.14 / $0.28 / 1M | OpenAI-compat | ★★ 官方端太慢，不当默认 |
| **GPT-5 nano / mini** | 通用 | 中等 | 稳，但不懂 LOL 词 | nano 约 $0.20 / $1.25 / 1M | 原生 | ★★ BYOK 即可，不必主推 |
| **DeepSeek 官方 reasoner / 任何 thinking 档** | — | 数秒级 | 与场景无关 | — | — | 排除 |

说明：

- Gemini 2.5 Flash-Lite 官方文档写明 **2026-10-16 退役**，后续应对齐 3.x Flash-Lite，不要把 model id 写死。
- DeepSeek 官方端的 TTFT 对局内聊天不可接受；若用户自己填第三方兼容端点（DeepInfra / SiliconFlow）可以当可选项。
- 游戏聊天体量极小：一场 100 句 ×（80 in + 40 out）token ≈ 1.2 万 token。Qwen-Turbo 单场成本远低于一分钱。**钱不是瓶颈，延迟和黑话才是。**

### 2.2 翻译（translation）

2025-07 上线、2026-06 文档仍在更新的 **Qwen-MT** 是目前最贴 Kotone 的在线翻译：从 Qwen3 微调，92 语，带 **术语干预 / 领域提示 / 翻译记忆**，官方点名「实时聊天、直播评论」。

| 模型 | 官方定位 | 语种 | 流式 | 术语干预 | 价格量级（国际） | 推荐 |
|------|----------|------|------|----------|------------------|------|
| **qwen-mt-lite** | 实时聊天 / 弹幕，最快最便宜 | 31 | 增量 | 有 | $0.12 / $0.36 / 1M | ★★★★★ 局内默认 |
| **qwen-mt-flash** | 通用推荐 | 92 | 增量 | 有 | $0.16 / $0.49 / 1M | ★★★★ 质量档 |
| qwen-mt-plus | 公文 / 论文 | 92 | 无增量 | 有 | $2.46 / $7.37 / 1M | 排除（慢且贵） |
| qwen-mt-turbo | 旧档，官方说不再更新 | 92 | 无增量 | 有 | 同 flash | 不要新接 |
| DeepL API | 欧语书面 | ~33 | — | glossary | ~$25 / 百万字符 | ★ 中文游戏句不值这个价 |
| Google / Azure 传统 MT | 通用 NMT | 100+ | — | 有限 | $10–20 / 百万字符 | ★ 无黑话、无口语 |

Qwen-MT 的 `translation_options.terms` 可以直接灌 LOL 词表（闪现→Flash、大龙→Baron、gank 保持 gank）。这比「在 system prompt 里求模型高抬贵手」可靠一个数量级。API 是 OpenAI-compat 外壳，但 **必须走 `extra_body.translation_options`**，不能当成普通 chat。

通用大模型也能翻译，且中英质量不差（Qwen / DeepSeek 在 ZH↔EN 上经常压过 DeepL）。问题是：更慢、更贵、术语不受控、偶尔会解释一句。局内发送不要走这条路。

---

## 3. 本地方案

### 3.1 硬约束：和游戏抢 GPU 就会输

LOL /  Valorant 吃 2–4GB 显存；3A 吃满一张卡。再塞一个 4B Q4（~2.5GB）或 7B Q4（~4.5GB）是在赌掉帧。本地默认路径必须是：

- **CPU + 量化小模型**，或
- 用户明确「仅在桌面 / 非全屏时用 GPU」。

Rust 侧不要为了本地 LLM 引入 Python。可选运行时：

| 运行时 | 优点 | 缺点 | 建议 |
|--------|------|------|------|
| **llama.cpp sidecar**（OpenAI-compat `llama-server`） | 与现有 STT sidecar 思路一致；崩溃隔离；预编译 Windows 二进制 | 多一个进程 | **首选** |
| `llama-cpp-2` / `llama-cpp-rs` 进程内 | 延迟略低 | Windows 上 CUDA/Vulkan feature 组合、MSVC 链接都疼 | 稳定后再考虑 |
| Ollama 当本地后端 | 用户可能已经装着 | 多一层、版本漂移 | 只做 connection 预设，不当依赖 |

本地处理器的 `networkAccess` 标 `local`，走和在线同一套 OpenAI-compat adapter，只是 base URL 指 `127.0.0.1`。

### 3.2 本地润色模型

| 模型 | 时间 | 许可 | Q4 体积 | CPU 体感（短句） | 指令遵循 / 中文 | 推荐 |
|------|------|------|---------|------------------|-----------------|------|
| **Qwen3.5-0.8B** | 2026-03 | Apache-2.0 | ~0.6GB | 现代 CPU 约 20–60 tok/s → 0.3–1.0s | C-Eval 46 / IFEval 52，官方自己写「适合原型和微调」 | ★★★ 可做离线兜底，**建议后续用 Kotone 语料微调** |
| **Qwen3.5-2B** | 2026-03 | Apache-2.0 | ~1.4GB | 约 10–25 tok/s → 0.8–2s | C-Eval 65 / IFEval 61，离「少改」更近 | ★★★★ 本地质感档，中高配 CPU |
| Qwen3-1.7B / 4B（2025-04） | 仍可用 | Apache-2.0 | 1.1 / 2.5GB | 4B CPU 常 >1.5s | 4B 质量明显好，但局内 CPU 偏慢 | ★★ 有独显且局外可用 |
| Qwen3.5-4B / 9B | 2026 | Apache-2.0 | 2.5 / 5.5GB | 必须 GPU | 写得最好 | ★ 仅「游戏未占 GPU」高级选项 |
| Phi-4 Mini / Gemma 3 1B | 2025 | 各异 | <3GB | 快 | 中文与游戏黑话弱 | 排除作中文主力 |

Qwen3.5-0.8B 的 WMT24++ 只有 27.2（对比 Qwen3-4B 的 58.9）——**千万不要用它做翻译**。润色可以，翻译不行。

本地润色要靠 prompt 锁死行为，否则小模型会把「闪了」扩写成「我已经使用了闪现技能」：

```text
只输出清理后的游戏聊天短句。
删除口癖（那个/就是/嗯/啊），修正明显识别错误。
不要改变意思，不要变正式，不要解释。
保留游戏术语原文：闪现、大龙、小龙、gank、flash、baron。
```

### 3.3 本地翻译模型（2025 末–2026 才是该看的）

| 模型 | 发布 | 许可 | 规模 / Q4 | 质量信号 | 术语 | 推荐 |
|------|------|------|-----------|----------|------|------|
| **HY-MT1.5-1.8B** | 2025-12，2026 仍在更 | Apache-2.0（同门 7B 已确认） | ~1.1GB GGUF，1–2GB 即可跑 | 专用 MT；官方按实时 / 端侧宣传；支持术语与解释性翻译 | 有 prompt 协议 | ★★★★★ 本地翻译首选 |
| TranslateGemma-4B | 2026-01 | Gemma（HF 门控） | ~2.5GB | WMT24++ MetricX 5.32 / COMET ~81；4B 接近旧 12B 基线 | 弱于 Qwen-MT | ★★★ 质量好，但更重、许可要过法务 |
| Hunyuan-MT-7B / HY-MT1.5-7B | 2025-09 / 1.5 | Apache-2.0 | ~4.5GB | WMT2025 公开成绩很强 | 有 | ★★ 质量高，局内太重 |
| Seed-X-7B | 2025 | 需核对 | ~4.5GB | 专用 MT，中英强 | 一般 | ★★ 同样偏重 |
| 通用 Qwen3.5-0.8B/2B 当翻译 | — | Apache-2.0 | 小 | WMT24++ 27–46，不够 | 靠 prompt | 排除 |
| NLLB-200 / MADLAD-400 / Opus-MT | 2022–2024 | NLLB 还是 **CC-BY-NC** | 0.6B–10B | 不再是 SOTA；MADLAD 吃 30GB+ RAM | 无 | 排除作默认 |

TranslateGemma-12B/27B 在开放权重里质量更高，但对「游戏旁边挂着」没有意义。

---

## 4. 多维对照（按 Kotone 真实句长估算）

假设输入 20 汉字 + 短 system / 术语表，输出 25 token。数字是量级，不是实验室均值。

### 4.1 润色

| 路径 | p50 延迟 | 质量 | 硬件 | 隐私 | 掉帧风险 | 适合 |
|------|----------|------|------|------|----------|------|
| 规则口癖表 | <5ms | 只去口癖 | 无 | 本地 | 无 | 默认常开 |
| 在线 Qwen-Turbo（北京） | 200–500ms | 高 | 无 | 出网 | 无 | 国服主力 |
| 在线 Gemini Flash-Lite | 300–600ms | 中高 | 无 | 出网 | 无 | 海外主力 |
| 在线 Groq Scout | 150–300ms | 中（中文偏弱） | 无 | 出网 | 无 | 极速档 |
| 本地 Qwen3.5-0.8B CPU | 300–1000ms | 中，易多改 | +0.6GB RAM | 本地 | 低 | 离线兜底 |
| 本地 Qwen3.5-2B CPU | 800–2000ms | 中高 | +1.4GB RAM | 本地 | 低–中（抢 CPU） | 高配离线 |
| 本地 4B+ GPU | 150–400ms | 高 | 抢 2.5GB+ 显存 | 本地 | **高** | 仅局外 |

### 4.2 翻译

| 路径 | p50 延迟 | 中英 + 黑话 | 硬件 | 适合 |
|------|----------|-------------|------|------|
| **qwen-mt-lite + terms** | 150–400ms | 高（术语硬约束） | 无 | **局内默认** |
| qwen-mt-flash + terms | 250–600ms | 更高 | 无 | 质量档 |
| 本地 HY-MT1.5-1.8B Q4 | 200–800ms（CPU） | 高，需自备词表 prompt | +1.1GB | 离线 / 隐私档 |
| 本地 TranslateGemma-4B | 0.5–2s CPU / 快但占 GPU | 高、语种多 | 2.5GB+ | 不作默认 |
| 通用 LLM「请翻译」 | 更慢、偶发解释 | 中高但不稳 | 看模型 | 不要 |

### 4.3 隐私与合规

- 在线润色 / 翻译只上传 **已经识别出的短文本**，不上传音频。比云端 ASR 轻一档，但仍要在设置页按 `networkAccess=internet` 明示。
- 国服玩家走 DashScope 北京；海外走新加坡 / Gemini / Groq。Connection 要能选区域。
- Gemma 许可带使用门槛；Qwen3.5 / HY-MT 的 Apache-2.0 更适合进安装包。
- NLLB 的 CC-BY-NC 直接排除商用分发。

---

## 5. 明确排除

| 候选 | 原因 |
|------|------|
| 默认捆绑 7B / 14B 本地 LLM | 和游戏抢 GPU，冷启动数秒 |
| Qwen3 thinking / DeepSeek reasoner / 任何「先想再写」 | 延迟爆炸 |
| 规则口癖词表当独立工序 | 漏召回、误伤黑话；口癖交给润色模型 |
| 独立中文标点恢复模型 | STT 已做 |
| 学术 CSC encoder（MacBERT 等）当润色 | 解决的不是口语整理 |
| NLLB / MADLAD / Helsinki Opus-MT 作默认翻译 | 不是 2026 SOTA；NLLB 还不能商用 |
| 用 0.8B 通用模型做翻译 | WMT24++ 27 分，会瞎翻 |
| DeepL 作中文游戏默认 | 贵、欧语取向、无黑话优势 |
| DeepSeek 官方端作默认润色 | TTFT ~2s |
| 把 API Key 写进 pipeline step JSON | 违反 ADR-009 |
| Python sidecar 跑 vLLM | 桌面分发成本归零收益 |

---

## 6. 推荐落地（对齐 ADR-009，不改 orchestrator）

主链路保持：

```text
RecognizedText
  → [可选] writing.openai-compat      writing / internet|local
  → [可选] translation.qwen-mt        translation / internet
        或 translation.local-mt       translation / local
  → [可选] builtin.blocklist-filter   utility / local     已有
  → ReadyText
```

失败策略默认 **best-effort**：超时或 5xx 留下一步文本继续发。游戏里「晚到的完美句子」不如「准时的原句」。

### Phase 2a — 只做在线（下一迭代）

第一阶段 **不实现本地推理**。Processor 只看见 HTTP：`connectionId` → base URL + key + model。以后无论是本机 Ollama、Kotone 自管的 llama-server，还是 Groq，对润色 / 翻译步骤都只是另一条 connection。

1. **Connection / 凭据层**：记录里预留 `kind`（`remote` / `attach` / `managed`），2a 只实现并暴露 `remote`。key 进系统凭据库，pipeline 只引 `connectionId`。
2. **`openai-compat` 润色**：DashScope / Gemini / Groq / Grok / 自定义 URL。system prompt、temperature=0.2、max_tokens、timeout。
3. **`qwen-mt` 翻译**：同一 HTTP 客户端，发 `translation_options`。默认 `qwen-mt-lite` + 游戏术语表。
4. 设置页用现有 descriptor；联网模块打隐私提示。
5. **不做规则口癖表。** 去口癖、修口误是润色模型的事；词表漏召回且容易误伤「闪了 / 那个巴龙」。

不在 2a 做：llama-server 启停、Ollama 探活、LLM GGUF 下载、本机端口占用。这些不挡在线主路径。

### Phase 2b — 本地 = 再接同一种 HTTP connection

Processor / factory / pipeline **零改动**。新增的是 Runtime 侧把 `attach` / `managed` 变成一条已探活的 localhost URL，然后照旧注入 `openai-compat` / 翻译步。

详见 §6.1–6.3。模型不进默认安装包。

### Phase 2c — 只有数据证明 0.8B 乱改时才做

9. 用「识别原文 → 玩家接受的发送文」微调 Qwen3.5-0.8B/2B。这是 Wispr Flow 那条路，也是本地质量的真正上限。在此之前不要幻想通用小模型能稳定「少改」。

### 预设组合（给设置页，而不是写死在 orchestrator）

| 预设 | 步骤 | 给谁 |
|------|------|------|
| 干净原句 | blocklist | 默认、怕延迟 |
| 国内增强 | 润色 → blocklist | 国服、要去口癖 |
| 外服发送 | qwen-mt-lite(zh→en, LOL terms) → blocklist | 国际服 |
| 离线（2b） | 本地 0.8B（best-effort）→ 可选本地 HY-MT | 无网 / 隐私 |

不要默认串「润色 + 翻译」两段在线 LLM：两段 TTFT 很容易把 p95 打穿。要译就直接译识别原文，不要先润色成散文再译。

### 6.1 本地推理怎么管：三种连接，一个 Supervisor

连接不是「又一种 Processor」，是 Runtime 级服务。kind 只有三种：

| kind | 含义 | Kotone 管不管进程 |
|------|------|-------------------|
| `remote` | DashScope / Grok / DeepSeek / Gemini | 不管。只保管 key + base URL |
| `attach` | 用户自己开的 LM Studio / Ollama / 已有 llama-server | **不管启停**。Runtime 只探 `GET /health` 或 `GET /v1/models` |
| `managed` | Kotone 自管的 `llama-server` | **管启停**。与 STT 同一条 Runtime start/stop |

`attach` 不是权宜之计，是一等公民：很多用户已经在跑 Ollama。Kotone 不杀别人的进程，也不在他们没开时偷偷拉起一份。

`managed` 才是「Kotone 自己管 llama-server」。规则一次定死：

**进程形态**

- 二进制：`~/.kotone/bin/llama-server.exe`（官方 llama.cpp Windows 发布包，默认 **CPU** 构建；Vulkan 包另列高级下载）。下载复用 `kotone-stt` 的清单 + SHA256 路径，由壳调用，不把 reqwest 拉进 core。
- 模型：`~/.kotone/models/llm/`。首发两枚可选包：`qwen3.5-0.8b-instruct-q4`（润色）、`hy-mt1.5-1.8b-q4`（翻译）。
- 监听：只绑 `127.0.0.1:18790`（避开 8080/11434，免得撞上用户自己的 LM Studio / Ollama）。
- 参数：`--jinja -c 2048 --parallel 1 -ngl 0 -t max(1, CPU-2)`。默认不上 GPU，给游戏留核。
- 窗口：`CREATE_NO_WINDOW`，与 whisper-cli 相同。
- 一个 Kotone 实例只驻留 **一台** managed server、**一个** 模型。`localInference.maxResident = 1`。pipeline 若同时引用两个不同的 managed 模型，compile 直接拒绝，提示改成 attach 第二条或换成在线翻译。

**何时起、何时停（跟 STT 对齐，但失败策略相反）**

```text
Runtime start
  1. 现有路径：校验 STT 模型 → warmup STT（失败则整次 start 失败）
  2. 扫描「当前启用 pipeline」里引用的 connection
  3. 若存在 kind=managed：
       模型文件不齐 → 记警告，connection.ready=false，**不阻断 start**
       文件齐 → spawn llama-server，等 /health（超时 20s）
       拉起失败 → 同样不阻断 start，本地步 best-effort
  4. 若存在 kind=attach：只探活，失败只标 not ready
  5. 进入 Running

Runtime stop
  先 unload STT，再 SIG/kill 自己拉起的 llama-server
  绝不杀 attach 的外部进程

pipeline / 模型 / ngl / threads 相对启动快照发生变化
  → restartNeeded（沿用 RuntimeManager 的提示，用户点重启）
```

STT 是主链路，模型不齐不能听。本地 LLM 是增强，**缺了仍能语音发送原文**。这是和 STT warmup 唯一刻意相反的地方。

**禁止的做法**

- 在 `process()` 里现启现杀。0.8B 冷加载就要 1–3s，局内不可用。
- 每句拉起 `llama-cli` 一次性推理（旧 whisper-cli 模式）。听写可以忍受，润色不行。
- Runtime 空闲 N 分钟自动卸模型。第一次开口会再付冷启动；若以后要做「节能」，必须是设置项且默认关。
- 游戏开着自动改 `-ngl`。没有可靠的「显卡已被 3A 占满」信号；GPU 层数只允许用户改，改完走 restartNeeded。
- 把 Supervisor 做成又一个 Processor。它是 Runtime 的兄弟对象，和 `EngineRegistry.warmup` 同级。

**代码落点（不新开 crate）**

| 模块 | 放哪 |
|------|------|
| `Connection` 记录（kind / baseUrl / modelId，无 key） | `kotone-core` 设置模型 |
| `LocalInferenceSupervisor`（spawn / health / kill / 当前快照） | `kotone-postprocess`，壳在 start/stop 里调用 |
| 模型与 llama-server 清单 | `kotone-postprocess` 常量；下载走壳 → 已有 downloader |
| keyring、连接 CRUD IPC | 桌面壳 / CLI |
| `process()` | 只打已探活的 `127.0.0.1:18790` 或 attach URL |

Supervisor 对 Processor 只暴露：`endpoint_for(connection_id) -> Option<ReadyClient>`。没 ready 就让该 step 按 best-effort 留下原文。

### 6.2 本地部署栈：借什么、写什么（2026-08 核实）

「下载 / 安装 / 部署 / 启停」看起来像一套完整产品，社区里确实有现成东西。逐项核过之后，**推理引擎借 llama-server，下载复用 Kotone 已有 downloader，启停薄封装自己写。不要把 Ollama / LM Studio / LocalAI 嵌进安装包。**

| 候选 | 许可证 | 给我们什么 | 为什么不当 managed 运行时 |
|------|--------|------------|---------------------------|
| **llama.cpp `llama-server`** | MIT | 预编译 Windows 包约 90MB；OpenAI-compat；`-ngl` / 线程可控；只绑 127.0.0.1 | **这就是 managed 要用的引擎** |
| **Ollama** | MIT，官方文档写明可 embed（`ollama-windows-amd64.zip` + `ollama serve`） | `ollama pull` 自带模型库 | 比裸 llama.cpp 慢一截；模型进 `~\.ollama` 不进 `~\.kotone`；默认会抢 GPU；Setup 带托盘，和游戏副工具叠两套常驻；国内拉库不如我们已有的魔搭镜像 |
| LM Studio | 闭源 Electron | GUI + 本地 API | 安装包 500MB+，不能嵌、不能分发 |
| LocalAI | 偏 Docker | OpenAI-compat | 桌面分发过重 |
| `llama-cpp-2` 进程内 FFI | MIT | 少一个进程 | Windows 上 CPU/Vulkan/CUDA 要多套链接，和当初否决 whisper-rs 同一条理由 |
| 某个 Rust「LLM 生命周期 crate」 | — | 无对等物 | 启停就是 spawn + `/health` + kill，Kotone 在 whisper-cli 上写过一遍 |

同类听写产品怎么做：VoiceInk / Superwhisper 的本地 LLM 都是 **attach 用户已安装的 Ollama**，并不在自己进程里管模型下载。Kotone 的 `attach` kind 就是这条经过验证的路。

**下载不要交给 `llama-server -hf`。** 它能从 Hugging Face 拉 GGUF，但国服玩家 HF 不稳定，而且 STT 已经有「魔搭优先 + SHA256 + 进度 + 原子落盘」。LLM GGUF 走同一条 downloader，钉死 URL 和哈希，再把路径传给 `-m`。这不是新造下载器，是复用 `kotone-stt` 的清单机制（由壳调用，core 仍然不沾 reqwest）。

**我们自己写的只有 Supervisor 胶水**（大约和 whisper sidecar 同级）：拼命令行、`CREATE_NO_WINDOW`、等 health、stop 时杀掉、崩溃标 not ready。不写推理、不写量化、不写模型转换。

**Qwen-MT 没有可下载的本地权重。** 2025-07 官方博文和 2026-06 阿里云文档都只提供 API（`qwen-mt-lite` / `flash` / `plus`）。Hugging Face 上没有对应 GGUF。所以：

| 场景 | 用什么 |
|------|--------|
| 在线翻译（默认） | **Qwen-MT API**（lite / flash + `terms`） |
| 离线 / 隐私翻译 | **不是 Qwen-MT**，是 HY-MT1.5-1.8B（或用户 attach 的本机模型） |
| 在线润色 | Qwen-Turbo / Gemini Flash-Lite / Groq / Grok，普通 chat |
| 离线润色 | Qwen3.5-0.8B Instruct Q4，经同一 llama-server |

翻译模块可以共用一个 factory：`connection.kind=remote` 且 provider=qwen-mt 时走 `translation_options`；`kind=managed|attach` 时改成普通 chat prompt（「译成英语，保留术语：…」）。用户看到的还是「翻译」，底下两套协议。

### 6.3 不要为本地生命周期新开 crate

llama-server 启停、Ollama 探活、LLM 模型下载 **都不单独成 crate**。理由和 ADR-001 否决「每引擎一个 crate」相同：这不是重 SDK，是壳已经会做的事。

| 以后要补的能力 | 放哪 | 为什么不是新 crate |
|----------------|------|---------------------|
| Ollama / LM Studio 探活 | `kotone-postprocess` 的 HTTP 客户端（`GET /health` 或 `/v1/models`） | 就是一次 HTTP，和在线探活同一条路 |
| llama-server spawn / kill | `kotone-postprocess::local_server`，由 Runtime start/stop 调用 | 和 whisper-cli 同级胶水，不是新领域 |
| llama-server.exe + GGUF 清单 | `kotone-postprocess` 常量 | 与 STT 模型清单同形态 |
| 真正的字节下载 | **已有** `kotone-stt::download`，壳来调 | 禁止再写一套 downloader；禁止 postprocess 依赖 stt |

唯一以后才考虑拆的是：若 CLI 和桌面都要直接调下载、又不想让 postprocess 依赖 stt，再把 `download.rs` 抽成 `kotone-download`。那是去重，不是「本地 AI 基础设施 crate」。2a / 2b 都不做这件事。

Processor 始终不认识子进程。本地对它来说就是 `http://127.0.0.1:…`。

**第一期（2a）和第二期（2b）的分界**

- 2a：三种 kind 的数据模型就位；实现 `remote` + `attach`（探活、不启停）。用户本机已有 Ollama 当天能用。
- 2b：实现 `managed` Supervisor + 两个可选 GGUF。这时才出现「Kotone 自己管的 llama-server」。

2a 不是逃避启停，是先把「不管别人的进程 / 管自己的进程」这条边界写进模型和 Runtime 扫描；2b 只是把 managed 分支的 spawn 补上。

---

## 7. 接入时要注意的坑

- Qwen-MT **禁止** system 消息、禁止多轮；`messages` 只能有一条 user。领域风格走 `domains` 字段，不要塞 chat template。
- 小模型必须设 `max_tokens`，并校验输出非空、长度不超过原文的 ~2 倍，防止「扩写说明书」。超长或空串按 ADR-009 视为 `InvalidOutput`。
- 取消 token 已经有了：在线请求要能 abort HTTP；本地 sidecar 要能取消 in-flight completion。
- 诊断只记 connection 类型、耗时、字符数，不记中间文本，不记 key。
- 历史要同时留 `RecognizedText` 和 `ReadyText`（ADR-009 后续项），否则 eval 会被润色污染。
- 中国大陆访问 Groq / Gemini / api.openai.com 不稳定；预设里按区域拆，不要写死一个全球端点。

---

## 8. 信息来源（均为 2025-07 之后，或仍为现行文档）

- ADR-009 与现有 pipeline：`docs/adr/009-post-processing-pipeline.md`，`crates/kotone-core/src/postprocess.rs`
- Qwen-MT（2025-07 发布，文档更新至 2026-06）：https://qwenlm.github.io/blog/qwen-mt/ ；https://www.alibabacloud.com/help/en/model-studio/machine-translation
- Qwen-MT 价格：https://www.alibabacloud.com/help/en/model-studio/model-pricing （2026-08-12）
- Qwen3.5-0.8B 模型卡 / 基准：https://huggingface.co/Qwen/Qwen3.5-0.8B （2026-03，Apache-2.0）
- Qwen3.5 Small 系列：https://www.marktechpost.com/2026/03/02/alibaba-just-released-qwen-3-5-small-models-...
- TranslateGemma（2026-01）：https://huggingface.co/google/translategemma-12b-it ；技术报告 arXiv:2601.09012
- HY-MT1.5-1.8B（2025-12）：https://huggingface.co/tencent/HY-MT1.5-1.8B ；官方 GGUF 可供 llama.cpp
- Hunyuan-MT-7B（2025-09，Apache-2.0）：https://arxiv.org/html/2509.05209v1
- DeepSeek V4 定价与 TTFT：https://api-docs.deepseek.com/quick_start/pricing ；DeepInfra / Artificial Analysis 2026 基准
- Gemini 2.5 Flash-Lite 定价与退役：https://ai.google.dev/gemini-api/docs/pricing ；2026-10-16 sunset 见 2026-07 价格综述
- Groq / Cerebras 延迟：Eden AI 2026-08 latency 综述；Groq Llama 4 Scout 文档
- 听写产品 LLM 清理层：Wispr Flow / Superwhisper 2026 评测（gilricardo.com 2026-04；getvoibe.com；spokenly.app 写明 Flow 用微调 Llama）
- 开放翻译模型对比（2026-04）：HF discuss 175201（TranslateGemma / LMT-60 / MADLAD）
- 端侧 LLM 现状：https://v-chandra.github.io/on-device-llms/ （2026-01）

---

## 9. 一句话决策

**下一期（2a）只做在线：凭据连接、OpenAI-compat 润色、Qwen-MT 翻译（默认 lite + 游戏术语表）。** 口癖和口误由润色模型处理，不做规则词表。Connection 模型预留三种 kind，但 2a 只实现 `remote`。本地以后对 Processor 只是另一条 HTTP connection；启停 / 探活 / GGUF 下载不新开 crate。4B 以上、thinking、传统 NMT、规则口癖、再做一层标点，全部先放着。

---

## 10. 第一阶段施工顺序（2a）

只做在线。不改 orchestrator 状态机。Processor 仍然 `text in → text out`。下面按依赖排列，后一步可以吃前一步的接口。

**不做：** 规则口癖表、attach / managed、llama-server、Ollama 探活、多套已保存 pipeline、历史双文本、thinking、标点模型。

### 顺序

```text
1 契约
   Connection 记录进 Settings（无 key；kind 预留，UI 只露出 remote）
   ProcessorConfigFieldKind 增加 connection
   ConnectionResolver 端口（按 id 解析出 baseUrl / model / key）
   factory 在 composition root 注入 resolver；create() 仍只吃 step JSON

2 凭据 + IPC
   keyring；list / upsert / delete connection
   读回不回传明文 key
   CLI 与桌面共用，试跑才能打真 API

3 HTTP 薄层
   kotone-postprocess 内 reqwest async
   POST /v1/chat/completions
   认取消 token、step timeout、空串 / 超长校验
   支持 extra_body（给下一步 Qwen-MT）

4 润色处理器
   writing.openai-compat
   必填 connectionId；可选 system prompt / max_tokens
   默认 onError=best-effort，timeout ≈ 800ms
   预设：DashScope / Gemini / Groq / Grok / 自定义 URL
   system prompt 负责去口癖、修口误、少改、保住游戏术语

5 翻译处理器
   translation.qwen-mt
   同一客户端 + translation_options
   默认 qwen-mt-lite；source/target；terms 可先复用 profile 热词

6 设置页
   连接管理（增删改、密钥框）
   步骤表单出现 connection 选择器
   联网模块隐私提示
   添加润色/翻译时不要再用「必填未填则 required + 5s」的 mock 默认值

7 试跑与回归
   设置页试跑走真 compile（能解析 connection）
   单元：client mock server、缺 connectionId 编译失败
   e2e：现有 mock 编排不断；补一条「选连接 → 润色步骤出现在列表」
```

### 验收（2a 完成的标志）

- 用户能存一条 DashScope（或 Grok）连接，key 不进 `config.json`
- pipeline 可加：润色 → 屏蔽词，或 Qwen-MT → 屏蔽词
- 试跑和局内发送共用同一套 compile
- API 超时 / 5xx 时 best-effort 发出原文
- `cargo test --workspace` 与现有 post-processing e2e 全绿

### 建议切 PR 的方式

| PR | 内容 | 可单独合吗 |
|----|------|------------|
| A | 契约 + Settings 迁移 + Resolver 空实现 | 是 |
| B | keyring IPC + 连接管理 UI | 依赖 A |
| C | HTTP 客户端 + 润色 + 翻译 + 试跑接线 | 依赖 A、B |
| D | 步骤表单 connection 选择器、默认超时/策略、e2e | 依赖 C |
