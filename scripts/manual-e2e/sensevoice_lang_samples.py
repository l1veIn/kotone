"""Make the speech samples for the SenseVoice language probe (crates/kotone-stt/tests/interpreter_sensevoice.rs).

Fixed test sentences only (game call-outs in Chinese, English, Japanese, Korean) → DeepSeek speech synthesis
(`POST https://api.deepseek.com/v1/tools/tts`, the player's DeepSeek key read in memory by real_deepseek.py, never
printed or written; costs a few cents) → ffmpeg → 16 kHz mono 16-bit WAV, with the reference text next to it:
<out>/<lang>-<n>.wav + <lang>-<n>.txt. Kept between runs (skips files that exist). Only these sentences leave the
machine. (edge-tts was the first choice; on 2026-11-25 Microsoft answered its handshake with 403.)
Needs: the DeepSeek key (credential or KOTONE_REAL_DEEPSEEK_KEY), ffmpeg on PATH, network.
usage: python -X utf8 -I scripts/manual-e2e/sensevoice_lang_samples.py [out dir, default target/sensevoice-lang]
"""
import json
import os
import subprocess
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "target", "sensevoice-lang")

SAMPLES = {
    "zh": [
        "对面打野在下路",
        "快来上路支援，我被抓了",
        "大龙刷了，先别打",
        "中路消失了，小心一点",
        "闪现交了，我们打团吗",
        "帮我拿一下蓝",
    ],
    "en": [
        "Enemy jungler is top, be careful.",
        "Push mid, they lost two.",
        "Can you help me take dragon?",
        "Behind you, behind you!",
    ],
    "ja": [
        "敵が後ろにいます、気をつけて。",
        "ドラゴンを取りに行こう。",
        "助けてください、ジャングラーが来ています。",
        "ミッドに敵がいない、注意して。",
    ],
    "ko": [
        "뒤에 적이 있어요 조심하세요.",
        "드래곤 먹으러 가자.",
        "정글러가 탑에 있어요.",
        "도와주세요 저 지금 죽어요.",
    ],
}


def synthesize(key, text, path):
    body = json.dumps({"text": text, "voice_id": "tide", "format": "wav"}).encode()
    request = urllib.request.Request("https://api.deepseek.com/v1/tools/tts", data=body, method="POST",
                                     headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read()
    if not data.startswith(b"RIFF"):
        raise RuntimeError(f"not a WAV for {text!r}: {data[:200]!r}")
    with open(path, "wb") as f:
        f.write(data)


def main():
    os.makedirs(OUT, exist_ok=True)
    key = None
    for lang, sentences in SAMPLES.items():
        for i, text in enumerate(sentences, 1):
            wav = os.path.join(OUT, f"{lang}-{i}.wav")
            with open(os.path.join(OUT, f"{lang}-{i}.txt"), "w", encoding="utf-8") as f:
                f.write(text)
            if os.path.exists(wav):
                continue
            key = key or real_deepseek.read_key()
            if not key:
                raise SystemExit("no DeepSeek key (credential not found and KOTONE_REAL_DEEPSEEK_KEY not set)")
            raw = wav[:-4] + ".raw.wav"
            synthesize(key, text, raw)
            subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", raw, "-ac", "1", "-ar", "16000",
                            "-sample_fmt", "s16", wav], check=True)
            os.remove(raw)
            print(wav, flush=True)
    del key
    print(f"samples in {OUT}")


if __name__ == "__main__":
    main()
