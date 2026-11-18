"""Custom wake words: hit rate and false wakes against the real wake model (offline, no microphone).

Synthesizes every wake word with several voices — Windows SAPI (Huihui, two speeds) and the real DeepSeek voices
(stella / tide; key from real_deepseek.py, memory only) — alone and followed by a command, plus a few minutes of
ordinary talk and game commentary without any wake word (negatives, all wake words listening at once). Then runs
`cargo run -p kotone-stt --features engine-sherpa --example wake_eval` (the desktop's own WakeDetector) and prints
hits per word / voice and false wakes per hour of negative audio.

usage: python -X utf8 -I scripts/manual-e2e/wake_eval.py [--sensitivity normal] [--no-deepseek]
Needs the downloaded wake model (default: target/agent-smoke/home/models/wake/…, override KOTONE_KWS_DIR).
"""
import json
import os
import subprocess
import sys
import wave

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "wake-eval")
MODEL = os.environ.get("KOTONE_KWS_DIR", os.path.join(
    REPO, "target", "agent-smoke", "home", "models", "wake", "sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01"))

# 预置（基线）+ 内置角色的默认唤醒词 + 一个自定义名字
WORDS = ["琴音琴音", "你好琴音", "小雨小雨", "小林小林", "星野星野"]
NEGATIVE_TEXT = [
    "今天这把打得还行，下路小林被抓了两次，不过后面小龙我们拿到了。",
    "我先去刷一下野，你们中路注意一下对面打野的位置，等会儿一起去拿先锋。",
    "对面那个琴女一直在骚扰，我们辅助的音量是不是开太大了，雨天的地图看不清。",
    "好的收到，我这边还有二十秒就能回城，买完装备直接去河道，记得插眼。",
    "这个英雄的技能前摇有点长，你们看到他抬手就往旁边走，别站在一条线上。",
    "我觉得这局可以打，他们经济虽然领先一点，但是我们后期强，拖一拖就好。",
    "刚才那波团战太乱了，下次开团之前先说一声，大家一起上，不要一个一个送。",
    "野区的小怪刷新是两分钟，你记一下时间，下一波我们一起去对面野区反。",
]


def sapi(text, path, rate=0):
    script = (
        "Add-Type -AssemblyName System.Speech;"
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;"
        "$s.SelectVoice('Microsoft Huihui Desktop');"
        f"$s.Rate = {rate};"
        "$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono);"
        f"$s.SetOutputToWaveFile('{path}', $f);"
        f"$s.Speak('{text}');"
        "$s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def read_wav(path):
    with wave.open(path) as w:
        rate, channels, width = w.getframerate(), w.getnchannels(), w.getsampwidth()
        data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16 if width == 2 else np.int32)
    data = data.astype(np.float32) / (32768.0 if width == 2 else 2147483648.0)
    if channels > 1:
        data = data.reshape(-1, channels).mean(axis=1)
    if rate != 16000:
        from scipy.signal import resample_poly
        from math import gcd
        g = gcd(rate, 16000)
        data = resample_poly(data, 16000 // g, rate // g).astype(np.float32)
    return data


def write_wav(path, data):
    pcm = (np.clip(data, -1, 1) * 32767).astype(np.int16)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(pcm.tobytes())


def deepseek_tts(key, text, voice, path):
    import urllib.request
    body = json.dumps({"text": text, "voice_id": voice, "format": "wav", "filter_markdown": True}).encode()
    request = urllib.request.Request("https://api.deepseek.com/v1/tools/tts", data=body, method="POST", headers={
        "Authorization": f"Bearer {key}", "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        raw = response.read()
    tmp = path + ".raw.wav"
    with open(tmp, "wb") as f:
        f.write(raw)
    write_wav(path, read_wav(tmp))
    os.remove(tmp)


def pad(data, lead=1.0, tail=1.0):
    return np.concatenate([np.zeros(int(16000 * lead), np.float32), data, np.zeros(int(16000 * tail), np.float32)])


def main():
    sensitivity = sys.argv[sys.argv.index("--sensitivity") + 1] if "--sensitivity" in sys.argv else "normal"
    os.makedirs(WORK, exist_ok=True)
    key = None if "--no-deepseek" in sys.argv else real_deepseek.read_key()
    voices = [("sapi", 0), ("sapi-fast", 3)] + ([("deepseek-stella", "stella"), ("deepseek-tide", "tide")] if key else [])
    manifest = []
    for word in WORDS:
        for variant, text in [("alone", word), ("command", f"{word}，帮我看看小地图")]:
            for voice, arg in voices:
                name = f"{word}-{variant}-{voice}.wav"
                path = os.path.join(WORK, name)
                if not os.path.exists(path):
                    raw = path + ".src.wav"
                    if voice.startswith("sapi"):
                        sapi(text, raw, arg)
                    else:
                        deepseek_tts(key, text, arg, raw)
                    write_wav(path, pad(read_wav(raw)))
                    os.remove(raw)
                manifest.append({"wav": name, "keyword": word, "voice": voice, "expect": 1})
    for voice, arg in voices:
        name = f"negative-{voice}.wav"
        path = os.path.join(WORK, name)
        if not os.path.exists(path):
            parts = []
            for i, text in enumerate(NEGATIVE_TEXT):
                raw = os.path.join(WORK, f"neg-{voice}-{i}.wav")
                if voice.startswith("sapi"):
                    sapi(text, raw, arg)
                else:
                    deepseek_tts(key, text, arg, raw)
                parts.append(pad(read_wav(raw), 0.3, 0.3))
                os.remove(raw)
            write_wav(path, np.concatenate(parts))
        manifest.append({"wav": name, "keyword": "", "keywords": WORDS, "voice": voice, "expect": 0})
    del key
    with open(os.path.join(WORK, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)
    env = dict(os.environ, TMP=os.path.join(REPO, "target", "tmp"), TEMP=os.path.join(REPO, "target", "tmp"))
    out = subprocess.run(["cargo", "run", "-q", "-p", "kotone-stt", "--features", "engine-sherpa", "--example", "wake_eval", "--",
                          MODEL, os.path.join(WORK, "manifest.json"), sensitivity],
                         capture_output=True, text=True, encoding="utf-8", cwd=REPO, env=env)
    if out.returncode != 0:
        print(out.stderr[-3000:])
        raise SystemExit(1)
    rows = [json.loads(line) for line in out.stdout.splitlines() if line.startswith("{")]
    print(f"sensitivity={sensitivity}")
    print(f"{'wake word':<10} " + " ".join(f"{v:>16}" for v, _ in voices) + "   total")
    for word in WORDS:
        cells, got, want = [], 0, 0
        for voice, _ in voices:
            r = [x for x in rows if x["keyword"] == word and x["voice"] == voice]
            hit = sum(1 for x in r if x["hits"] >= 1)
            cells.append(f"{hit}/{len(r)}")
            got += hit
            want += len(r)
        print(f"{word:<10} " + " ".join(f"{c:>16}" for c in cells) + f"   {got}/{want} ({got / max(1, want) * 100:.0f}%)")
    neg = [x for x in rows if not x["keyword"]]
    seconds = sum(x["seconds"] for x in neg)
    false = sum(x["hits"] for x in neg)
    print(f"negatives: {seconds / 60:.1f} min of talk, {false} false wakes "
          f"({false / max(seconds, 1) * 3600:.1f} per hour) {[w for x in neg for w in x['words']]}")
    with open(os.path.join(WORK, f"result-{sensitivity}.json"), "w", encoding="utf-8") as f:
        json.dump(rows, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
