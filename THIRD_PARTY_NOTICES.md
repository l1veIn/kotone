# Third-party notices

Kotone does not bundle speech model weights in the installer. The user chooses and downloads a model separately from within the application.

## PP-OCRv5 (local OCR, optional)

The optional local Korean OCR uses PP-OCRv5 mobile detection and Korean recognition models. They are not bundled; `kotone-cli ocr install` downloads the ONNX files on request and verifies their SHA-256.

- Model project: PaddleOCR, https://github.com/PaddlePaddle/PaddleOCR (Apache License 2.0)
- ONNX conversion and distribution: RapidAI/RapidOCR v3.10.0, https://github.com/RapidAI/RapidOCR (Apache License 2.0), files from https://www.modelscope.cn/models/RapidAI/RapidOCR
- Inference uses the ONNX Runtime (MIT License) contained in the sherpa-onnx prebuilt package below; Kotone adds no separate OCR runtime.

## sherpa-onnx

Kotone uses the sherpa-onnx runtime and its Rust bindings for local inference.

- Project: https://github.com/k2-fsa/sherpa-onnx
- License: Apache License 2.0

## sherpa-onnx KWS zipformer WenetSpeech 3.3M (voice wake-up, optional)

Used for voice wake-up detection when the user enables it. Not bundled with Kotone: the user's copy is downloaded on request from the official k2-fsa/sherpa-onnx GitHub release (`kws-models`), and only the int8 encoder/joiner, fp32 decoder and tokens are kept.

- Model: https://github.com/k2-fsa/sherpa-onnx/releases/tag/kws-models (`sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01`)
- Model license as published: Apache License 2.0
- Training data: WenetSpeech, whose license permits non-commercial use only (https://arxiv.org/abs/2110.03370). This is why Kotone does not redistribute the model.

## X-ASR-zh-en

The default X-ASR streaming Chinese/English model is attributed to the X-ASR project and is converted for sherpa-onnx deployment.

- Model project: https://huggingface.co/GilgameshWind/X-ASR-zh-en
- Conversion/download source: https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models
- License: Apache License 2.0

## SenseVoice

The optional SenseVoice model is distributed by the FunASR project.

- Download source: https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17
- Original project: https://github.com/modelscope/FunASR
- Model license: FunASR Model Open Source License Agreement 1.1

The FunASR model license requires retaining the model name and attribution to the source and author.

## FunASR-Nano

The optional FunASR-Nano ONNX model is exported from the FunASR-Nano model family.

- Download source: https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30
- Export source: https://github.com/Wasser1462/FunASR-nano-onnx
- Upstream project and model license: https://github.com/modelscope/FunASR/blob/main/MODEL_LICENSE

## silero VAD

The optional voice activity detection model is distributed through sherpa-onnx:

- Download source: https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models

## Kokoro v1.1-zh

The optional local speech output uses the INT8 ONNX conversion of Kokoro v1.1-zh. The model is downloaded separately; its included `LICENSE` must be retained.

- Original model and model card: https://huggingface.co/hexgrad/Kokoro-82M-v1.1-zh
- Conversion: https://huggingface.co/csukuangfj/kokoro-int8-multi-lang-v1_1
- Download: https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2
- Model license: Apache License 2.0
- Voice presets used by Kotone: upstream Chinese female voices `zf_001`, `zf_003`, and `zf_017`; these are not custom voice clones.

## eSpeak NG frontend and data

The Kokoro model package also contains `espeak-ng-data`. Model licensing does not replace the licenses of the text frontend, its data, or the native inference dependencies.

- Project: https://github.com/espeak-ng/espeak-ng
- sherpa-onnx 1.13.5 integration: https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.5/cmake/espeak-ng-for-piper.cmake
- Pinned fork source: https://github.com/csukuangfj/espeak-ng/tree/ed530aa113046142eb5115cf2fc9157854d0ffe1
- GPL version 3 license text in that revision: https://github.com/csukuangfj/espeak-ng/blob/ed530aa113046142eb5115cf2fc9157854d0ffe1/COPYING

Redistribution must account for the actual native runtime and frontend/data licenses, including applicable notice and corresponding-source requirements. Do not describe the complete TTS dependency chain as Apache-only merely because the model weights use Apache-2.0. See `docs/research-tts.md` for the verified package identity and implementation notes.

## Settings window UI (frontend)

The settings window uses shadcn-svelte components copied as source, plus npm packages bundled into the frontend build. File-by-file origins and Kotone's changes are recorded in `docs/third-party/rambledesk-ports.md`.

- shadcn-svelte components (`apps/desktop/src/lib/components/ui/`), via RambleDesk: https://github.com/huntabyte/shadcn-svelte, MIT License
- bits-ui: https://github.com/huntabyte/bits-ui, MIT License
- Lucide icons (`@lucide/svelte`): https://github.com/lucide-icons/lucide, ISC License
- svelte-sonner: https://github.com/wobsoriano/svelte-sonner, MIT License
- tailwind-merge: https://github.com/dcastil/tailwind-merge, MIT License
- tailwind-variants: https://github.com/heroui-inc/tailwind-variants, MIT License
- clsx: https://github.com/lukeed/clsx, MIT License
- tw-animate-css: https://github.com/Wombosvideo/tw-animate-css, MIT License
- marked (Markdown lexer for the 琴音 chat): https://github.com/markedjs/marked, MIT License

### Codeg (theme palettes)

Parts of the appearance system (`apps/desktop/src/lib/appearance/palettes.css`, `appearanceSettings.ts`) and of the 琴音 chat (`apps/desktop/src/lib/agentChat/`: keyboard / IME guard, scroll anchor, tool status, thinking block, tool card, turn footer, transcript) are adapted from Codeg through RambleDesk, and keep their Apache-2.0 headers.

- Project: https://github.com/xintaofei/codeg, commit `3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1`
- Copyright: Codeg authors and contributors
- License: Apache License 2.0, full text in `licenses/codeg-APACHE-2.0.txt`

### RambleDesk

RambleDesk (MIT, Copyright (c) 2026 l1veIn — the same author and license as Kotone) is the intermediate source of the components and appearance code above.

### DeepSeek Harness (interface layout reference)

The settings window shell and the 琴音 chat (title bar, sidebar, content frame, message flow rows, approval card, input card, settings forms) follow the layout, information hierarchy and interaction patterns of DeepSeek Harness's client UI packages. Kotone re-implements them in Svelte with its own components; no DeepSeek branding (logo, brand assets, name or brand colours) is used. Files modelled on a specific upstream component name it in their header; details are in `docs/third-party/deepseek-harness-ui.md`.

- Project: https://github.com/deepseek-ai/deepseek-harness, commit `39163dd30f703a79079e2d2446545cff3c453738` (`packages/client/ui-*`)
- Copyright (c) 2026 DeepSeek
- License: MIT License, full text in `licenses/deepseek-harness-MIT.txt`

### pi-ai (AI connection presets)

The AI connection provider presets (names, groups, base URLs, model lists) are derived from pi-ai's model catalog.

- Project: pi-ai (`@earendil-works/pi-ai`), Copyright Earendil Works / the pi authors (see its own LICENSE)
- License: Apache License 2.0

## Game computer use (`game.look` / `game.act`, ADR 019)

The design of 琴音's "look at the screen, move the mouse" tools follows the open-source projects below. Kotone re-implements the behaviour in Rust; no upstream source file is copied. What was taken from each project is listed in `docs/adr/019-game-computer-use.md` and in the module headers of `crates/kotone-tools/src/computer_use/` and `apps/desktop/src-tauri/src/computer_use.rs`. Copyright belongs to the respective authors as stated in each repository's license file.

- Anthropic computer use (action names and parameters, screenshot scaling rule, typing groups of 50 characters with 12 ms gaps): `anthropic-quickstarts` `computer-use-demo/tools/computer.py`, https://github.com/anthropics/anthropic-quickstarts, MIT License; Anthropic computer use tool documentation
- dsh-computer-use-win (batch rules: validate the whole plan first, stop at the first failure, "Not executed: an earlier action failed.", stopReason, only report "sent", input tag so the player's own input stops the batch, POINT_COVERED, WINDOW_CHANGED, per-app consent): https://github.com/Yu-tao-Li/dsh-computer-use-win, MIT License; and deepseek-harness `packages/experimental/cua`, MIT License
- Microsoft UFO² (numbered element list `{id, text, box}` given to the model): https://github.com/microsoft/UFO, MIT License
- Microsoft OmniParser (numbered label style: palette colour, black text when luminance > 160, label positions tried above / left / right / above-right): https://github.com/microsoft/OmniParser, code under Creative Commons Attribution 4.0 (CC-BY-4.0) — attribution: OmniParser, Microsoft. Its icon detection model is not used.
- Cradle by BAAI (look → reflect → plan → act loop; relative mouse movement for camera control; "skills are code" = Kotone automations): https://github.com/BAAI-Agents/Cradle, MIT License
- GameVerse (key_down / key_up actions and hold-to-walk for real Windows games): MIT License
- UI-TARS-1.5 game prompt by ByteDance (prompt clauses: record new game rules, change approach when an action has no effect, release held keys, ask the user when stuck; 100 ms settle before clicking): https://github.com/bytedance/UI-TARS, Apache License 2.0

## Game voice chat (ADR 020)

### VB-CABLE (not bundled)

Kotone does not ship VB-CABLE. When the user asks for it, Kotone downloads the official, unmodified `VBCABLE_Driver_Pack46.zip` from https://vb-audio.com/Cable/ (SHA-256 pinned) and runs VB-Audio's own setup program; the user agrees to VB-Audio's license by installing it.

- VB-CABLE, Copyright Vincent Burel / VB-Audio Software. Origin: https://vb-cable.com. **VB-CABLE is a donationware, all participations are welcome.**
- Licensing: https://vb-audio.com/Services/licensing.htm (distribution with another application "AS IS", silent installation allowed "provided that our donationware model remains applicable"; VB-CABLE A+B / C+D are not used).

### Design references (no code copied)

- Soundux (GPL-3.0), https://github.com/Soundux/Soundux — downloads VB-CABLE and runs `VBCABLE_Setup_x64.exe` with `-h -i` in its installer
- TTS Voice Wizard (MIT), https://github.com/VRCWizard/TTS-Voice-Wizard — text-to-speech played into the virtual cable while the user hears a copy
- cpal `examples/feedback.rs` (Apache-2.0), https://github.com/RustAudio/cpal — input stream → buffer → output stream
- AudioDeviceCmdlets (MIT) and EarTrumpet (MIT) — the `IPolicyConfig` COM interface (`SetDefaultEndpoint`) used to change the Windows default recording device

## Sub-sessions, memory, SOUL, skills and the ear (ADR 021 / 022)

### Adapted designs (MIT; notices kept)

- OpenClaw, Copyright (c) 2026 OpenClaw Foundation, MIT License, https://github.com/openclaw/openclaw — workspace files (`SOUL.md`, `IDENTITY.md` with `- **Name:**` fields, `USER.md`, `MEMORY.md`), `sessions_spawn` / `sessions_send` semantics (a child reports back to its parent; children get a minimal prompt and an allow-listed tool set), identity set from settings overriding `IDENTITY.md`.
- Hermes Agent, Copyright (c) 2025 Nous Research, MIT License, https://github.com/NousResearch/hermes-agent — the `memory` tool (add / replace / remove on bounded `MEMORY.md` 2,200 and `USER.md` 1,375 character budgets, frozen snapshot per session, prompt-injection scan before writing) and skill management (`skill_view`, `skill_manage`).
- Agent Skills format, https://agentskills.io — `SKILL.md` with YAML frontmatter (`name`, `description`), one folder per skill; skills are shareable packs.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions: The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.

### Code dependencies and references

- wasapi-rs (`wasapi` crate), MIT License, https://github.com/HEnquist/wasapi-rs — process loopback capture (`AudioClient::new_application_loopback_client`), following its `examples/record_application.rs`.
- Microsoft ApplicationLoopback sample (MIT), https://github.com/microsoft/windows-classic-samples — `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` (Windows 10 build 20348 or later).
- auto-caption (MIT), https://github.com/HiMeditator/auto-caption — continuous captions with sherpa-onnx SenseVoice and endpoint detection (design reference; no code copied).
- Speech recognition of heard audio uses the SenseVoice and silero VAD models listed above.

## Prefix cache, context compaction and usage statistics (ADR 024)

### DeepSeek Harness (MIT)

- Project: https://github.com/deepseek-ai/deepseek-harness, commit `ca33fd6843b2` (0.2.2-rc.4); license text in `licenses/deepseek-harness-MIT.txt`.
- Adapted: the request layout for DeepSeek's prefix cache (a byte-stable system prompt and tool list per conversation, changing facts appended with the user message, append-only history, images dropped in batches, reasoning passed back), the compaction request shape (the conversation's own system prompt, tools and messages plus a final instruction, so the summary call reuses the cache; one checkpoint that later compactions merge; a proportional retained tail), the structure and rules of the compaction instruction (sections, "(none)" for empty sections, merge a previous checkpoint, no tool calls), and the cache hit-rate formula and display (hits / input, never shown as 100% unless it is). Kotone's text is Chinese and companion-specific. `crates/kotone-ai/src/agent/compact.rs`, `crates/kotone-agent/src/prompt.rs`, `crates/kotone-agent/src/port.rs`, `apps/desktop/src/lib/agentChat/timeline.ts`.

### goose (Apache-2.0)

- Project: https://github.com/aaif-goose/goose (formerly block/goose), commit `196698819392`, Copyright 2024 Block, Inc.; license text in `licenses/goose-APACHE-2.0.txt`. goose has no NOTICE file.
- Adapted and modified (Apache-2.0 §4(b)): the compaction trigger (a share of the context window, compact before the model call), keeping the original history visible while only the summary goes to the model, the rules of `crates/goose-context-management/src/prompts/compaction.md` (quote errors verbatim, keep the user's words, omit rather than invent, no new ideas) and the continuation sentence of `goose/src/context_mgmt/mod.rs` ("do not mention the summary, continue naturally"), rewritten in Chinese for a game companion in `crates/kotone-ai/src/agent/compact.rs`.

### Hermes Agent (MIT)

- The guard against repeated ineffective compactions (skip when compacting would free too little) — see the Hermes notice above.

## Switchable roles and custom wake words (ADR 025)

- Character Card V2 specification, https://github.com/malfoyslastname/character-card-spec-v2 (commit `8083fb38`; the repository has no license — Kotone implements the format only and copies no text): `roles/<id>/role.json` is a V2 card, Kotone's fields under `data.extensions.kotone`; import also reads V1 and V3 cards (`chara_card_v3`, https://github.com/kwaroran/character-card-spec-v3, MIT).
- SillyTavern (AGPL-3.0), https://github.com/SillyTavern/SillyTavern — concept only (the character card belongs to the character; the user's persona and world info are shared). No code copied.
- sherpa-onnx `text2token` (`tokens_type="ppinyin"`), Apache License 2.0, https://github.com/k2-fsa/sherpa-onnx (`sherpa_onnx/utils.py`, `sherpa-onnx/csrc/utils.cc` @ `4488718d`): how a Chinese phrase becomes the keyword spotter's pinyin tokens and how a keyword line is parsed, reimplemented in `crates/kotone-stt/src/wake_word.rs`. The model's `tokens.txt` (sherpa-onnx KWS zipformer WenetSpeech 3.3M, listed above) is embedded as `crates/kotone-stt/src/wake_tokens.txt` to check wake words before the model is downloaded.
- `pinyin` crate (rust-pinyin), MIT License, Copyright (c) 2016 mozillazg, https://github.com/mozillazg/rust-pinyin — Chinese characters to tone-marked pinyin (features `with_tone`, `heteronym`).

## Built-in role art: 大肥鱼 and Claude大小姐 (CC BY-NC-SA 4.0, non-commercial)

The avatar and expression images in `apps/desktop/src/assets/roles/` are **not** covered by Kotone's MIT license. They are licensed under Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International (full text in `licenses/CC-BY-NC-SA-4.0.txt`, shipped with the installer): no commercial use; adaptations must use the same license. Kotone cropped them to head shots, scaled them to 256×256 and converted them to WebP (details in `apps/desktop/src/assets/roles/README.md`). The personalities (SOUL texts) are Kotone's own writing.

- 大肥鱼 (`dafeiyu/`): frames from PC2005-cloud/dsh-pet (https://github.com/PC2005-cloud/dsh-pet; code MIT, Copyright (c) 2026 PC2005-cloud). Community fan character of DeepSeek's whale, **not an official DeepSeek image**; prototype 「溟月」 by @上善无形 (https://space.bilibili.com/4456176), DeepSeek maid version designed by @ZipZipPipe (https://space.bilibili.com/4168597); character license CC BY-NC-SA 4.0 (https://www.bilibili.com/opus/1247156596114456581).
- Claude大小姐 (`claude-ojou/`): 素材：Claude大小姐的日记本（github.com/opsCar/claude-daxiaojie-assets）· 角色设定：ZipZipPipe（胖次狐）· 设定图：lmy414/ai-chan-sheets · CC BY-NC-SA 4.0. Fan work, not affiliated with Anthropic; "Claude" is a trademark of Anthropic.

These notices describe third-party components and models; they do not grant a license to Kotone itself.

## DeepSeek API documentation (behaviour reference)

The DeepSeek connection client (`crates/kotone-ai/src/agent/deepseek.rs`) and web search (`crates/kotone-ai/src/web.rs`) follow DeepSeek's official guides — "构建 Chatbot" (`latest_reminder`, `title`, `safety_check`, thinking mode, error handling) and "网页搜索与抓取" (`/v1/tools/search`, `/v1/tools/fetch`) — at https://api-docs.deepseek.com/zh-cn/guides/build_a_chatbot. No code or text was copied; requests go to the player's own DeepSeek account under DeepSeek's API terms. See ADR 026.

## Voice calls: always listening, barge-in, echo cancellation (ADR 033)

- sonora (`sonora` crate 0.3), BSD-3-Clause License, https://github.com/dignifiedquire/sonora — a pure-Rust port of the WebRTC AudioProcessing module; Kotone uses its echo canceller (AEC3) and high-pass filter in `crates/kotone-platform-windows/src/echo_cancel.rs`; license text in `licenses/sonora-BSD-3-Clause.txt`. Copyright (c) 2011, The WebRTC Project Authors; Copyright (c) 2016, Arun Raghavan and contributors; Copyright (c) 2026, dignifiedquire. All rights reserved. Redistribution of binaries must reproduce the copyright notice, the list of conditions and the disclaimer of the BSD-3-Clause license.
- orb (dsh-orb-cordis), Copyright (c) 2026 mini-yifan, MIT License, https://github.com/mini-yifan/dsh-orb-cordis — design adapted, no code copied: echo cancellation on Windows with the default speaker's WASAPI loopback as the reference, a silent render stream that keeps loopback flowing, AEC3 with noise suppression and gain control off and the high-pass filter on, reference and microphone paired frame by frame by timestamp, not using the communications category (`packages/voice/native/windows-voice-capture.cpp`); the idle timer that pauses while she speaks, works or waits for an answer (`packages/voice/src/idle.ts`); the dismiss phrase that works while she speaks or works (`packages/voice/src/dismiss.ts`); and comparing a transcript with what she just said to drop her own echo (`packages/voice/src/echo.ts`). The MIT permission notice is reproduced in the sub-sessions section above.
- voice-call, Copyright (c) 2026 lw, MIT License, https://github.com/crease123/voice-call — design reference (call lifecycle, onset confirmation and pre-roll before a sentence); no code copied.

## Music: NetEase Cloud Music client and player (ADR 034, `crates/kotone-music`)

- NeteaseCloudMusicApi Enhanced (api-enhanced), Copyright (c) 2013-2022 Binaryify, MIT License, https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced — the weapi / eapi algorithms and constants (`util/crypto.js`), the QR login and endpoint parameters, the song-detail request shape and the two-minute response cache are adapted in `crates/kotone-music/src/netease/`; license text in `licenses/api-enhanced-MIT.txt`.
- ncmapi (ncmapi-rs), Copyright (c) 2021 TMs, MIT License, https://github.com/two-mountains/ncmapi-rs — per-endpoint encryption choices, the eapi header cookie set, the persisted 52-hex device id and the `type: 3` QR login flow are adapted in `crates/kotone-music/src/netease/`; license text in `licenses/ncmapi-MIT.txt`.
- YesPlayMusic, Copyright (c) 2020-2023 qier222, MIT License, https://github.com/qier222/YesPlayMusic — the `isTrackPlayable` rules (privilege / fee / copyright) are ported in `crates/kotone-music/src/netease/parse.rs`; license text in `licenses/YesPlayMusic-MIT.txt`.
- Symphonia (`symphonia` crate 0.5), Mozilla Public License 2.0, https://github.com/pdeljanov/Symphonia — decodes MP3 / FLAC / AAC for playback; the MPL-2.0 source code is available from that repository and crates.io.
- qrcode (`qrcode` crate 0.14), MIT OR Apache-2.0, https://github.com/kennytm/qrcode-rust — draws the login QR code.

## League of Legends plugin (ADR 035 / 038, `plugins/lol`)

- LeagueAkari (`dev` branch), Copyright (c) 2026 Hanxven, MIT License, https://github.com/LeagueAkari/LeagueAkari — the LCU command-line parsing and endpoint paths, the mode-haste rule for summoner spell timers, party detection from `teamParticipantId`, and the Tencent 101 response parser (`src/shared/data-adapter/champion-data/qq101-protocol.ts`, ported field by field in `plugins/lol/src/stats/qq101.rs`) are adapted; license text in `licenses/LeagueAkari-MIT.txt`. Only the MIT-licensed `dev` branch is used, not the GPL releases v1.3.x–v1.4.3.
- aramhex, Copyright (c) 2026 czgreat, MIT License, https://github.com/czgreat/aramhex — design reference for the overlay (show only when needed, small windows, below-normal recognition thread, not reading back its own drawing). The overlay rules are ported from its source (`main` at `17ef65cd`, read 2026-11-25) in `plugins/lol/src/overlays/rules.rs`: the S–D grade table and win-rate-only ranking (`lolhex/engine.py`: `GRADES`, `rank_grade`, `hero_brief`), the champ-select comparison of the bench against your champion (`lolhex/ui/overlay.py`: `ChampSelectLayer`), and the augment verdicts top pick / fine / consider a reroll with their thresholds (`lolhex/recommend.py`: `evaluate`, `REROLL_BELOW`, `REROLL_GAP`, `RELIABLE_GAMES`, `TOO_FEW_RATIO`, the rare-augment score). The rules are rewritten in Rust and the overlay look is our own (no artwork used); license text in `licenses/aramhex-MIT.txt`.
- aramhex-data, Creative Commons BY-NC 4.0, https://github.com/czgreat/aramhex-data — ARAM Mayhem champion × augment statistics. Not bundled: Kotone downloads it at run time when the player turns on “用网上的统计”, verifies the SHA-256 listed in `latest.json`, and credits the repository and license on every page that shows it. Kotone is free; the data is not used commercially.
- Tencent 101 / 掌盟 and `game.gtimg.cn` (hero list, items, runes, summoner spells) — undocumented public endpoints, requested on demand at run time; not bundled.
- OP.GG MCP (`mcp-api.op.gg/mcp`) — off by default until OP.GG answers https://github.com/opgginc/opgg-mcp/issues/26; when turned on, results are shown with OP.GG's name and a link to op.gg. Connected through Kotone's MCP client (`crates/kotone-mcp`, below), one session reused for all calls.

## MCP client (`crates/kotone-mcp`, MCP plan phase 1)

- rmcp, the official Model Context Protocol Rust SDK, version 3.11.0, https://github.com/modelcontextprotocol/rust-sdk — used as a library dependency, not modified. Copyright (c) 2024-2025 Model Context Protocol a Series of LF Projects, LLC. Apache License 2.0 (the project is moving from MIT to Apache-2.0; contributions not yet relicensed remain under the MIT License, see its `LICENSE`). Kotone uses its client, stdio (async read/write) and Streamable HTTP client transports; the test-only server in `crates/kotone-mcp/src/testing.rs` and `examples/stdio_server.rs` uses its server side.
- goose, Apache License 2.0 (see the goose notice above; license text in `licenses/goose-APACHE-2.0.txt`), commit `03a343f9fe30` — design followed, no code copied: wrapping rmcp's client service with a timeout on every request that sends `notifications/cancelled` when it expires (`crates/oi-kit-agent/src/mcp/client.rs`), and starting stdio servers without a console window while keeping their stderr for startup errors (`crates/oi-kit-agent/src/subprocess.rs`).
- OpenAI Codex, Copyright 2025 OpenAI, Apache License 2.0 (same text as `licenses/goose-APACHE-2.0.txt`), https://github.com/openai/codex, commit `98630438816` — adapted: the list of environment variable names a stdio server inherits on Windows (`WINDOWS_CORE_ENV_VARS` in `codex-rs/protocol/src/shell_environment.rs`), resolving the program on the child's `PATH` with `which::which_in` so `npx` finds `npx.cmd` (`codex-rs/rmcp-client/src/program_resolver.rs`), and the default timeouts (10 s startup, 60 s per tool call). Codex's NOTICE: "OpenAI Codex, Copyright 2025 OpenAI" (its Ratatui-derived code is not used).
- Hermes Agent (MIT, see the notice above), commit `2506f5b0a4d3` — adapted: stdio MCP servers get only an allow-list of non-secret environment variables plus what their configuration sets (`_build_safe_env` and `_SAFE_ENV_KEYS_CASE_INSENSITIVE` in `tools/mcp_tool_config.py`); the extra Windows names from that list are merged into Kotone's baseline in `crates/kotone-mcp/src/stdio.rs`.
- `which` crate, MIT License, https://github.com/harryfei/which-rs — finds the program on `PATH` / `PATHEXT`.

## MCP Apps in Kotone (ADR 039 D5, MCP plan phases 3 and 5)

- goose, Apache License 2.0 (see the goose notice above; license text in `licenses/goose-APACHE-2.0.txt`), commit `34adc70f13f2` — adapted and modified (Apache-2.0 §4(b)): the CSP source normalisation and validation of `crates/goose/src/acp/mcp_app_proxy.rs` (`normalize_csp_source`, `is_valid_csp_host_source`; now `normalize_csp_source` in `crates/kotone-plugin-api/src/resource.rs`) and the shape of its CSP (`build_outer_csp`: start from `default-src 'none'`, add only declared origins, block objects and form posts; now `app_csp` in `apps/desktop/src-tauri/src/app_proxy.rs`); the design of serving MCP Apps from a loopback server with a random secret, accepting only loopback peers.
- MCP Apps SDK (`@modelcontextprotocol/ext-apps`), Copyright (c) Model Context Protocol a Series of LF Projects, LLC, MIT License, https://github.com/modelcontextprotocol/ext-apps, commit `ea3e9d0` — the sandbox proxy relay of `examples/basic-host/src/sandbox.ts` is adapted in `apps/desktop/src-tauri/src/app_sandbox.html`; version 2.1.0 of the package (with `@modelcontextprotocol/core` 2.4.0, Apache-2.0, and `zod` 4.6.7, MIT) is bundled into the League of Legends App (`plugins/lol/app`, built into a single HTML file with `vite-plugin-singlefile`, MIT) together with Svelte (MIT); its `AppBridge` (with `@modelcontextprotocol/client` 2.4.0, Apache-2.0) is bundled into Kotone's main window, loaded only when an App is shown (`apps/desktop/src/lib/components/plugins/McpAppFrame.svelte`).
- CommunityDragon (https://raw.communitydragon.org), a community mirror of the game client's assets — champion icons are requested from it at run time when the local client cannot give them; nothing is bundled (ADR 039 D6). League of Legends assets belong to Riot Games.
