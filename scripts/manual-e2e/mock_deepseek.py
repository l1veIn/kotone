"""A local stand-in for DeepSeek (chat, ASR, TTS) for the DeepSeek-only happy-path test. No network, no cost.

POST /v1/chat/completions   scripted 琴音:
    "配置" in the last user message -> skill.view(kotone-config) -> models.activate(asr deepseek-asr)
                                       -> models.activate(ocr deepseek-ocr) -> "配好了"
    anything else                   -> "收到：<what the player said>"
POST /v1/tools/asr          -> {"text": MOCK_ASR_TEXT}; the uploaded WAV is saved as <log dir>/asr-<n>.wav
POST /v1/tools/tts          -> a short tone as 24 kHz WAV
GET  /v1/models             -> one model
Every request (minus audio bytes) is appended to <log dir>/requests.jsonl.
usage: python -X utf8 -I mock_deepseek.py <port> <log dir>
"""
import io
import json
import math
import os
import struct
import sys
import wave
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18767
LOG_DIR = sys.argv[2] if len(sys.argv) > 2 else "."
ASR_TEXT = os.environ.get("MOCK_ASR_TEXT", "帮我看看队友说了什么")
asr_count = 0


def log(entry):
    with open(os.path.join(LOG_DIR, "requests.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")


def chunk(delta, finish=None):
    return {"id": "x", "object": "chat.completion.chunk", "created": 0, "model": "deepseek-chat",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}


def text_of(message):
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(part.get("text", "") for part in content if isinstance(part, dict))
    return ""


def plan(body):
    messages = body.get("messages", [])
    last_user = max((i for i, m in enumerate(messages) if m.get("role") == "user"), default=-1)
    user = text_of(messages[last_user]) if last_user >= 0 else ""
    done = sum(1 for m in messages[last_user + 1:] if m.get("role") == "tool")
    if "配置" in user:
        steps = [("skill.view", {"name": "kotone-config"}),
                 ("models.activate", {"category": "asr", "engineId": "deepseek-asr", "modelId": "deepseek-asr"}),
                 ("models.activate", {"category": "ocr", "modelId": "deepseek-ocr"})]
        final = "配好了：语音识别和文字识别都用 DeepSeek。"
    else:
        steps = []
        final = "收到：" + user.strip()[:80]
    if done < len(steps):
        name, args = steps[done]
        return [chunk({"role": "assistant", "tool_calls": [{"index": 0, "id": f"call_{done}", "type": "function",
                                                               "function": {"name": name.replace(".", "_"), "arguments": json.dumps(args, ensure_ascii=False)}}]}),
                chunk({}, "tool_calls")]
    return [chunk({"role": "assistant", "content": final}, "stop")]


def tone_wav():
    buf = io.BytesIO()
    w = wave.open(buf, "wb")
    w.setnchannels(1); w.setsampwidth(2); w.setframerate(24000)
    w.writeframes(b"".join(struct.pack("<h", int(3000 * math.sin(i / 8))) for i in range(7200)))
    w.close()
    return buf.getvalue()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, data, content_type):
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.reply(200, json.dumps({"data": [{"id": "deepseek-chat"}]}).encode(), "application/json")

    def do_POST(self):
        global asr_count
        raw = self.rfile.read(int(self.headers.get("content-length", "0")))
        auth = self.headers.get("authorization", "")[:10]
        if self.path.endswith("/tools/asr"):
            asr_count += 1
            start = raw.find(b"RIFF")
            end = raw.rfind(b"\r\n--")
            audio = raw[start:end] if start >= 0 else b""
            with open(os.path.join(LOG_DIR, f"asr-{asr_count}.wav"), "wb") as f:
                f.write(audio)
            log({"path": self.path, "auth": auth, "bytes": len(audio), "multipartFileField": b'name="file"' in raw})
            self.reply(200, json.dumps({"request_id": f"r{asr_count}", "text": ASR_TEXT, "duration_ms": 1000}, ensure_ascii=False).encode(), "application/json")
            return
        if self.path.endswith("/tools/tts"):
            log({"path": self.path, "auth": auth, "body": json.loads(raw or b"{}")})
            self.reply(200, tone_wav(), "audio/wav")
            return
        body = json.loads(raw or b"{}")
        log({"path": self.path, "auth": auth, "body": body})
        events = plan(body)
        payload = "".join(f"data: {json.dumps(e, ensure_ascii=False)}\n\n" for e in events) + "data: [DONE]\n\n"
        self.reply(200, payload.encode(), "text/event-stream")


HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
