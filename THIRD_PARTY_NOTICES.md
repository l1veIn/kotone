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

These notices describe third-party components and models; they do not grant a license to Kotone itself.
