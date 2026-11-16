<p align="center">
  <img src="assets/banner.webp" alt="Kotone — 琴音 · 游戏玩家的语音输入中继站" width="100%">
</p>

<h3 align="center">
  <samp>🎤 说话 &nbsp;→&nbsp; ⚡ 成字 &nbsp;→&nbsp; 🎮 发送</samp>
</h3>

<p align="center">
  <samp>手指不离开键盘。想说的话，一秒都别等。</samp>
</p>

<p align="center">
  <a href="https://github.com/l1veIn/kotone/releases"><img src="https://img.shields.io/github/v/release/l1veIn/kotone?style=flat-square&label=release&color=00E5FF" alt="release"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%2F%2011%20x64-1A1A2E?style=flat-square" alt="platform">
  <img src="https://img.shields.io/badge/Tauri%202-Rust%20workspace-FF2D78?style=flat-square" alt="tauri">
  <img src="https://img.shields.io/badge/STT-local%20by%20default-7B2FFF?style=flat-square" alt="local by default stt">
</p>

<div align="center">

https://github.com/user-attachments/assets/b0b9a55f-9d74-4bdc-ba55-de02883a8d7f

</div>

<p align="center">
  <samp>按住热键说话，松手进游戏聊天框。</samp>
</p>

<p align="center">
  <samp>
    <a href="#-为什么需要-kotone">为什么</a> ·
    <a href="#-三步上手">上手</a> ·
    <a href="#-功能速览">功能</a> ·
    <a href="#-为-agent-而生">Agent</a> ·
    <a href="#-界面">界面</a> ·
    <a href="#-角色卡">角色卡</a> ·
    <a href="#-技术栈">技术栈</a> ·
    <a href="#-开发">开发</a> ·
    <a href="#-鸣谢">鸣谢</a>
  </samp>
</p>

## 🥊 为什么需要 Kotone

<table>
  <tr>
    <td width="50%">
      <h4>😰 以前的你</h4>
      <sub>打团打到一半，队友 ping 你报点。<br>你停下操作 → 打开聊天框 → 打字 → 回车。<br>抬起头，你的角色已经站在原地三秒了。</sub>
    </td>
    <td width="50%">
      <h4>😎 现在的你</h4>
      <sub>按住热键，说一句「打野在下路」。<br>松开热键，文字已经发送。<br>你的手指，从头到尾没离开过键盘。</sub>
    </td>
  </tr>
</table>

<br>

**Kotone** 是一款专为游戏玩家打造的 Windows 桌面语音输入工具：默认用本机 X-ASR 实时转写，对讲机模式下按住热键说话、松开发送到游戏聊天框。也可选择在线识别、AI 润色/翻译或语音提醒；在线识别会发送音频，AI 功能会发送相应文本到你配置的服务，详见 [隐私政策](PRIVACY.md)。

> <i>“想说的话，一秒都别等。”</i> — 琴音

<br>

## 🚀 三步上手

<p align="center">
  <table>
    <tr>
      <td align="center" width="33%">
        <samp>STEP 01</samp>
        <h3>📥 装</h3>
        <sub>从 <a href="https://github.com/l1veIn/kotone/releases">Releases</a> 下载 <code>Kotone_*_x64-setup.exe</code><br>支持 Windows 10 / 11（x64）<br>安装器自带简体中文界面</sub>
      </td>
      <td align="center" width="33%">
        <samp>STEP 02</samp>
        <h3>🧭 配</h3>
        <sub>首次启动向导一条龙：<br>选「英雄联盟」或「通用输入」<br>→ 下载本地模型 → 设热键 → 真实发送测试</sub>
      </td>
      <td align="center" width="33%">
        <samp>STEP 03</samp>
        <h3>🎤 说</h3>
        <sub>之后的一切交给热键。<br>内置自动更新，<br>新版本发布自动提醒。</sub>
      </td>
    </tr>
  </table>
</p>

> 暂未接入 Windows 代码签名，系统可能显示 SmartScreen 提示。请只从本仓库的 GitHub Release 下载，并核对 Release 中公布的 SHA-256。

<br>

## ✨ 功能速览

- 🎤 **语音转写** — 默认 X-ASR 中英标点模型在本机边说边出字；可选本地非流式模型和在线 ASR。VAD 判停组件随应用分发。
- ⌨️ **全局热键** — 按住说话 / 点按切换两种模式，任意键自定义；提示驻留期内再按热键，直接开始下一句。
- 🎮 **游戏术语热词** — 内置 100 个英雄联盟术语热词；版本更新的新词条自动并入，你的自定义与删除永不丢失。
- 🌙 **电竞悬浮窗** — 卡片 / 胶囊两种样式，整面可拖动并记忆位置，鼠标点击穿透；流式引擎实时出字，非流式引擎显示声波动画。
- 🕘 **历史与回放** — 历史可关闭或清理；开启“保存录音”后，新记录可回放原始音频。空转写不发送，并给出提示。
- 🔒 **本地优先** — 默认本地识别，录音保存默认关闭；可选网络服务的数据边界公开，诊断包由你主动导出分享。
- 🤖 **琴音 Agent** — 内置的游戏 Agent：在聊天窗口里打字，或在游戏里按住陪玩热键说一句话，琴音会看聊天、替你在游戏里发言、设提醒、运行和编写自动化、打开或切换游戏；有风险的操作先问你。游戏内的回答用语音或弹幕播出（默认 Windows 系统语音，可选 Kokoro 本地中文语音与在线语音）。需要你自己的 AI 连接。使用方法见 [琴音](docs/assistant.md)。
- ⚙️ **自动化** — 沙盒里的小程序替你按键、点鼠标、找色找图找字，带延迟、绑定游戏窗口、最多执行一次、随时可停；可以保存、带参数，绑到快捷指令热键上直接运行。

<br>

## 🤖 为 Agent 而生

> **我们的判断：任何软件想把功能做完整，最终都会变得复杂；而复杂的软件，应该交给 Agent 使用，而不是让人类去学。所以每一个软件最后都会变成基础设施——“对 Agent 友好”是当下一切软件发展的方向。**

Kotone 因此不只是一个给人点的桌面应用，也是一组给 Agent 调用的本机基础服务。人类仍可以按住热键说话、在设置页里点选；同样的能力会逐步以带 schema、可发现、结果可验证的工具开放出来，由 Agent 决定何时用、怎么组合、如何向你解释。Kotone 负责把事情确定地做完并如实报告结果；理解意图、翻译、决策交给 Agent。

- **现在**：Kotone 自带一个游戏 Agent（琴音），同时把同一套 79 个工具开放给外部 Agent（工具服务默认不对外监听，`toolService.enabled` 打开）：窗口与截图、裁剪与预处理、多语种 OCR、聊天消息跟踪、自动化、打开 / 切换 / 关闭游戏、设置与模型等全部配置。外部 Agent 能后台启动 / 退出 Kotone，并能收到你经 Kotone 说给它的话——把助手设为转交 Agent（`assistant.route agent`）后，按住助手热键说“我队友在说啥？”，Agent 就会处理并用语音或弹幕回答你。接入方式：命令行（安装版 `kotone.exe <子命令>`，与开发用 `kotone-cli` 相同）、MCP（`kotone.exe mcp`）、DSH 插件（独立仓库 `dsh-game-communicate`）、[Agent Skill](integrations/skills/kotone/SKILL.md)。见 [工具服务手册](docs/tool-service.md)。
- **接下来**：把 Kotone 其余的经典业务（语音识别、文本处理、游戏发送、播报与弹幕、提醒、热键、设置、连接、模型、历史、诊断）全部拆成基础服务，全部开放、不设授权分级。路线见 [演进计划](docs/product-evolution-game-tools.md)。

<br>

## 📸 界面

<table>
  <tr>
    <td width="50%" align="center">
      <img src="assets/screenshot-welcome.webp" alt="Kotone 首次启动向导" width="95%" style="border-radius: 8px; border: 1px solid #30363d;">
      <br>
      <samp>🚀 首次启动向导</samp>
    </td>
    <td width="50%" align="center">
      <img src="assets/screenshot-voice.webp" alt="Kotone 通用设置主界面" width="95%" style="border-radius: 8px; border: 1px solid #30363d;">
      <br>
      <samp>🎤 通用设置</samp>
    </td>
  </tr>
  <tr>
    <td width="50%" align="center">
      <img src="assets/screenshot-config.webp" alt="游戏适配设置" width="95%" style="border-radius: 8px; border: 1px solid #30363d;">
      <br>
      <samp>⚙️ 游戏适配设置</samp>
    </td>
    <td width="50%" align="center">
      <img src="assets/screenshot-history.webp" alt="历史记录与回放" width="95%" style="border-radius: 8px; border: 1px solid #30363d;">
      <br>
      <samp>🕘 历史记录与回放</samp>
    </td>
  </tr>
</table>

<br>

## 🔧 技术栈

<p align="center">
  <img src="https://img.shields.io/badge/Tauri_2-24C8DB?style=for-the-badge&logo=tauri&logoColor=white" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/Svelte_5-FF3E00?style=for-the-badge&logo=svelte&logoColor=white" alt="Svelte 5">
  <img src="https://img.shields.io/badge/Tailwind_CSS-06B6D4?style=for-the-badge&logo=tailwindcss&logoColor=white" alt="Tailwind CSS">
  <img src="https://img.shields.io/badge/sherpa--onnx-7B2FFF?style=for-the-badge" alt="sherpa-onnx">
</p>

<samp>

- **Desktop** — Tauri 2 + Rust workspace：`kotone-core`（领域与端口）、`kotone-stt`（识别适配器与模型）、`kotone-ai`（共享 AI 连接与凭据）、`kotone-tts`（本地神经语音合成与播放）、`kotone-postprocess`（文本处理器）、`kotone-platform-windows`（热键 / 注入 / 系统语音 / 窗口与截图）、`kotone-tools`（[Agent 工具服务](docs/tool-service.md)）、`kotone-ocr`（本地 / 在线 OCR）、`kotone-cli`（[命令行前端](docs/cli.md)）
- **Voice** — 默认 sherpa-onnx · X-ASR · Silero VAD 本地识别链路；在线 ASR 按需选择；播报默认系统 SAPI，可选 Kokoro INT8 复用 sherpa-onnx
- **UI** — Svelte 5 + Tailwind CSS，深色电竞主题

</samp>

<br>

## 📂 仓库结构

```
apps/desktop            Tauri 桌面应用（Svelte 5 前端 + Rust 后端）
crates/kotone-core      会话编排、配置、历史、profile 的领域核心
crates/kotone-stt       识别适配器、模型清单与下载
crates/kotone-tts       可选 Kokoro 本地语音合成与播放
crates/kotone-ai        AI 连接、凭据、HTTP 与内置 Agent 循环（模型通信经 rig-core）
crates/kotone-agent     内置 Agent 的工具清单、权限与系统提示
crates/kotone-script    自动化的 Luau 沙盒运行时
crates/kotone-postprocess  文本过滤、润色、翻译处理器
crates/kotone-platform-windows  全局热键、窗口注入、提权、窗口观察与截图等 Windows 平台能力
crates/kotone-tools     本机 Agent 工具服务：协议、产物、预处理、聊天跟踪、HTTP 服务端/客户端
crates/kotone-ocr       OCR 后端：本地多语种 PP-OCRv5 与 DeepSeek 在线 OCR、模型安装
crates/kotone-cli       命令行前端（自动化测试、无人值守入口、工具服务客户端）
docs/                   ADR、CLI 参考、发布检查单、评测手册
```

<br>

## 🚀 开发

<details open>
<summary><b>快速开始</b></summary>

```bash
git clone https://github.com/l1veIn/kotone.git
cd kotone
pnpm install --frozen-lockfile
pnpm dev          # 需要 Rust 与 Windows C++ Build Tools
```

</details>

<details>
<summary><b>质量门禁与构建</b></summary>

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm check
pnpm -C apps/desktop test:e2e
cargo test --workspace --locked
pnpm build        # NSIS 安装包输出到 target/release/bundle/nsis/
```

开发入口见 [文档索引](docs/README.md)、[架构](docs/architecture.md)、[术语表](docs/glossary.md) 和 [开发指南](docs/development.md)。完整门禁与发布检查项见 [发布清单](docs/release-checklist.md)。

</details>

隐私与第三方模型信息见 [`PRIVACY.md`](PRIVACY.md) 和 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)，变更历史见 [`CHANGELOG.md`](CHANGELOG.md)。

<br>

---

<br>

## 🎭 角色卡

<table>
  <tr>
    <td width="34%" align="center">
      <img src="assets/kotone-cutout.webp" alt="琴音立绘" width="270">
    </td>
    <td width="66%">
      <h3>琴音 · Kotone <sub>ことね</sub></h3>
      <p>
        <samp>AGE&nbsp;&nbsp;&nbsp;&nbsp;18</samp><br>
        <samp>JOB&nbsp;&nbsp;&nbsp;&nbsp;游戏主播 —「打字比打游戏快」</samp><br>
        <samp>FANS&nbsp;&nbsp;&nbsp;键盘侠（她起的，说要亲手平反）</samp><br>
        <samp>BASE&nbsp;&nbsp;&nbsp;中继站 · 被 RGB 霓虹和粉丝手写信包围的直播间</samp><br>
        <samp>MOTTO&nbsp;&nbsp;想说的话，一秒都别等</samp>
      </p>
      <p>观众来看她打游戏，留下来看她边打团边秒回弹幕——弹幕说她是<b>「人形语音输入法」</b>。她曾尝试用语音输入回弹幕，结果「谢谢老板」被识别成「谢谢老伴」，那面贴满翻车记录的「社死墙」至今还在直播间里。</p>
      <p><samp>「收到，已发送！✨」</samp></p>
    </td>
  </tr>
</table>


<p align="center">
  <img src="https://img.shields.io/badge/%E9%9C%93%E8%99%B9%E9%9D%92-%2300E5FF-00E5FF?style=flat-square" alt="霓虹青 #00E5FF">
  <img src="https://img.shields.io/badge/%E6%B7%B1%E9%82%83%E8%93%9D%E9%BB%91-%231A1A2E-1A1A2E?style=flat-square" alt="深邃蓝黑 #1A1A2E">
  <img src="https://img.shields.io/badge/%E5%93%81%E7%BA%A2%E8%83%BD%E9%87%8F-%23FF2D78-FF2D78?style=flat-square" alt="品红能量 #FF2D78">
  <img src="https://img.shields.io/badge/%E7%B4%AB%E7%94%B5-%237B2FFF-7B2FFF?style=flat-square" alt="紫电 #7B2FFF">
</p>

<br>

## 🙏 鸣谢

Kotone 站在许多优秀开源项目与模型的肩膀上。特别感谢：

| 项目 | 角色 |
| --- | --- |
| [**sherpa-onnx**](https://github.com/k2-fsa/sherpa-onnx) | 本地语音推理运行时与 Rust 绑定，流式 / 非流式引擎的基础设施 |
| [**X-ASR**](https://huggingface.co/GilgameshWind/X-ASR-zh-en) | 默认中英流式标点模型（Zipformer2 transducer），边说边出字的核心 |
| [**FunASR**](https://github.com/modelscope/FunASR) | 中文 ASR 工业级生态；SenseVoice、FunASR-Nano 等模型的上游 |
| [**SenseVoice**](https://github.com/FunAudioLLM/SenseVoice) | 可选多语非流式识别模型（中英日韩粤等） |
| [**FunASR-Nano**](https://github.com/modelscope/FunASR) · [ONNX 导出](https://github.com/Wasser1462/FunASR-nano-onnx) | 可选高质量非流式档位 |
| [**Silero VAD**](https://github.com/snakers4/silero-vad) | 语音活动检测，一句话判停与端点切分 |
| [**LeagueAkari**](https://github.com/LeagueAkari/LeagueAkari) | 英雄联盟游戏内文本注入与发送流程的参考实现 |
| [**Tauri**](https://github.com/tauri-apps/tauri) · [**Svelte**](https://github.com/sveltejs/svelte) | 桌面壳与前端框架 |
| [**RepoChan**](https://github.com/l1veIn/repochan-mono) | 人设 / 视觉 / 品牌资产流水线（分析 → 人设 → 艺术指导 → 画师 → 页面） |

许可证与模型归属细节见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。若有遗漏，欢迎提 PR 补全。

<br>

---

<br>

<p align="center">
  <img src="assets/kotone-poster.webp" alt="Kotone Risograph 海报" width="100%">
</p>

<p align="center">
  <samp>品牌由 <a href="https://github.com/l1veIn/repochan-mono"><b>RepoChan</b></a> 人设流水线一站式产出 — 分析 → 人设 → 艺术指导 → 画师 → 页面</samp>
  <br>
  <samp>made with 💖 by RepoChan</samp>
</p>
