# Third-party notices

Kotone does not bundle speech model weights in the installer. The user chooses and downloads a model separately from within the application.

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

These notices describe third-party components and models; they do not grant a license to Kotone itself.
