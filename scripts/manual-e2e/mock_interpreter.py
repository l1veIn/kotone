"""OpenAI-compatible mock for the interpreter (同传) real-machine test (ADR 021 / ADR 029 phase 2). No network, no cost.

Routes by the system prompt: a background task's prompt says "在后台替玩家做事的帮手", the main conversation's does not.

main        "帮我盯着队友"  -> agent.start(name=interpreter, label=同传) -> final "好，我在听"
main        "停止同传"      -> agent.stop(interpreter) -> final "已经停了"
main        other           -> final "收到"
background  task            -> voice.listen.start(app = MOCK_LISTEN_APP) -> final "开始听了，翻译会显示成弹幕"
background  heard sentence  -> interpreter.show({text: "（译）<text>", heard, app}) when the interpreter plugin offers it (ADR 043
                               D11: the plugin pushes the line and the “这局的同传” card), else danmaku("（译）<text>")
                               (the background cannot speak) -> final "好的"
background  “监听……停了” note (voice_listen.rs, e.g. local recognition stopped) -> final "同传停了"
POST …/tools/asr (DeepSeek speech recognition, multipart) -> {"text": MOCK_ASR_TEXT} (default "Enemy behind the red house.")

Every request body is appended to <log dir>/requests.jsonl.
usage: python -X utf8 -I mock_interpreter.py <port> <log dir>
"""
import json
import os
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18766
LOG_DIR = sys.argv[2] if len(sys.argv) > 2 else "."
APP = os.environ.get("MOCK_LISTEN_APP", "python.exe")
ASR_TEXT = os.environ.get("MOCK_ASR_TEXT", "Enemy behind the red house.")


def chunk(delta, finish=None):
    return {"id": "x", "object": "chat.completion.chunk", "created": 0, "model": "mock",
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
    system = text_of(messages[0]) if messages and messages[0].get("role") == "system" else ""
    last_user = max((i for i, m in enumerate(messages) if m.get("role") == "user"), default=-1)
    user = text_of(messages[last_user]) if last_user >= 0 else ""
    done = sum(1 for m in messages[last_user + 1:] if m.get("role") == "tool")
    if "在后台替玩家做事的帮手" in system:
        if "监听" in user and "停了" in user and "（听到" not in user:
            steps = []
            final = "同传停了"
        elif "（听到" in user:
            heard = re.findall(r"KOTONE_DATA（[^）]*）\n(.*?)\nKOTONE_DATA", user, re.S)
            said = heard[-1].strip() if heard else user[-80:]
            offered = {t.get("function", {}).get("name") for t in body.get("tools", []) if isinstance(t, dict)}
            if "interpreter_show" in offered:
                steps = [("interpreter.show", {"text": "（译）" + said[:200], "heard": said[:200], "app": APP})]
            else:
                steps = [("danmaku", {"text": "（译）" + said[:200]})]
            final = "好的"
        else:
            steps = [("voice.listen.start", {"app": APP})]
            final = "开始听了，翻译会显示成弹幕"
    else:
        if "帮我盯着队友" in user:
            steps = [("agent.start", {"name": "interpreter", "label": "同传",
                                      "task": "你是同传。先 voice.listen.start 听 " + APP + "；每听到一句外语就翻成中文，用 danmaku 的弹幕显示。"})]
            final = "好，我在听"
        elif "停止同传" in user:
            steps = [("agent.stop", {"name": "interpreter"})]
            final = "已经停了"
        else:
            steps = []
            final = "收到"
    if done < len(steps):
        name, args = steps[done]
        return [chunk({"role": "assistant", "tool_calls": [{"index": 0, "id": f"call_{done}", "type": "function",
                                                               "function": {"name": name.replace(".", "_"), "arguments": json.dumps(args, ensure_ascii=False)}}]}),
                chunk({}, "tool_calls")]
    return [chunk({"role": "assistant", "content": final}, "stop")]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        body = json.dumps({"data": [{"id": "mock"}]}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length)
        if self.path.endswith("/tools/asr"):
            with open(os.path.join(LOG_DIR, "asr.jsonl"), "a", encoding="utf-8") as f:
                f.write(json.dumps({"bytes": len(raw)}) + "\n")
            data = json.dumps({"text": ASR_TEXT}).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        body = json.loads(raw or b"{}")
        with open(os.path.join(LOG_DIR, "requests.jsonl"), "a", encoding="utf-8") as f:
            f.write(json.dumps(body, ensure_ascii=False) + "\n")
        events = plan(body)
        payload = "".join(f"data: {json.dumps(e, ensure_ascii=False)}\n\n" for e in events) + "data: [DONE]\n\n"
        data = payload.encode()
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
