"""OpenAI-compatible mock for the sub-session / listen real-machine test (ADR 021 / 022). No network, no cost.

Routes by the system prompt: a sub-session's prompt says "子会话", the main session's does not.

main   "帮我盯着队友"  -> session.spawn(同传, tools: listen/say/send) -> final "好，我在听"
main   "停止同传"      -> session.stop(<id from the earlier spawn result>) -> final
main   other           -> final
child  task             -> voice.listen.start(app = MOCK_LISTEN_APP) -> final "已开始监听"
child  heard sentence   -> reply.say(danmaku, "（译）<text>") -> session.send(parent, "队友说：<text>") -> final
child  other            -> final

Every request body is appended to <log dir>/requests.jsonl.
usage: python -X utf8 -I mock_sessions.py <port> <log dir>
"""
import json
import os
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18766
LOG_DIR = sys.argv[2] if len(sys.argv) > 2 else "."
APP = os.environ.get("MOCK_LISTEN_APP", "python.exe")


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
    all_text = "\n".join(text_of(m) for m in messages)
    if "子会话" in system and "派出的子会话" in system:
        if user.startswith("（听到"):
            heard = re.findall(r"KOTONE_DATA（[^）]*）\n(.*?)\nKOTONE_DATA", user, re.S)
            said = heard[-1].strip() if heard else user[-80:]
            steps = [("reply.say", {"text": "（译）" + said[:200], "delivery": ["danmaku"]}),
                     ("session.send", {"id": "parent", "text": "队友说：" + said[:200]})]
        elif user.startswith("停止"):
            steps = [("voice.listen.stop", {})]
        else:
            steps = [("voice.listen.start", {"app": APP})]
        final = "已开始监听" if not steps or steps[0][0] == "voice.listen.start" else "好的"
    else:
        if "帮我盯着队友" in user:
            steps = [("session.spawn", {"task": "你是同传。先 voice.listen.start 听 " + APP + "；每听到一句外语就翻成中文，用 reply.say 的弹幕显示；队友在叫玩家时 session.send 告诉我。",
                                         "label": "同传", "tools": ["voice.listen.start", "voice.listen.stop", "reply.say", "session.send"]})]
            final = "好，我在听"
        elif "停止同传" in user:
            ids = re.findall(r'"id"\s*:\s*"(c-[0-9-]+)"', all_text)
            steps = [("session.stop", {"id": ids[-1] if ids else "missing"})]
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
        body = json.loads(self.rfile.read(length) or b"{}")
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
