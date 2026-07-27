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

These notices describe third-party components and models; they do not grant a license to Kotone itself.
