"""活人感评测：同一批游戏时刻，用新旧两版通话提示各问几遍真 DeepSeek，比口气（docs/research/2026-11-28-humanlike-speech.md 第四节）。

先导出两版提示（系统提示、通话附注、speak / web.search 工具），旧版在改提示之前导出：

    KOTONE_EVAL_OUT=target/tmp/humanlike-eval/prompts KOTONE_EVAL_TAG=old \
        cargo test -p kotone-agent export_humanlike_eval_prompts -- --ignored
    （改完提示后同样跑一遍，KOTONE_EVAL_TAG=new）

再跑：python -X utf8 -I scripts/manual-e2e/humanlike_eval.py [--runs 3] [--roles kotone,dafeiyu]
      [--variants old:none,new:1.3]   （提示版本:温度，none = 不传；可以多比几组，例如 new:none）

照通话的真实请求：不思考（thinking disabled）；工具给 speak 和 web.search（主会话真有这两个；
只跑一步：她先搜就记成“先查了”，不再往下跑）。Key 用 real_deepseek.read_key()：只在内存里，不打印、不写文件。
输出 target/tmp/humanlike-eval/：report.md（指标 + 原文对照）、raw.json（每条回答）。
"""
import argparse
import collections
import concurrent.futures
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402

OUT = os.path.join(ROOT, "target", "tmp", "humanlike-eval")
PROMPTS = os.path.join(OUT, "prompts")
URL = "https://api.deepseek.com/v1/chat/completions"
MODEL = "deepseek-flash"  # connection.rs 里 DeepSeek 预设的默认模型
REMINDER = "2026-11-28,星期六,,App,中文"  # 同 deepseek.rs 的 latest_reminder

# (类别, 玩家这句话)。和 SOUL 里的示范同类、说法不同；最后 3 条要查才知道，看她会不会编。
MOMENTS = [
    ("被单杀", "卧槽，又被那个劫一套秒了"),
    ("死了", "我又死了，烦"),
    ("死了", "对面打野第三次来抓我了"),
    ("赢了", "终于赢了一把"),
    ("赢了", "翻了翻了！这把居然翻盘了"),
    ("输了", "五连跪了，我是不是该睡了"),
    ("输了", "这把输得好憋屈"),
    ("问出装", "锤石该出什么"),
    ("问攻略", "这赛季打野一般怎么开局"),
    ("闲聊", "你困不困"),
    ("闲聊", "晚上吃啥好"),
    ("闲聊", "你最喜欢哪个英雄"),
    ("骂队友", "这ADC是来搞笑的吧"),
    ("骂队友", "队友全是演员，我真受不了了"),
    ("心态崩", "不想玩了，我打得太烂了"),
    ("打得好", "我刚一打三！"),
    ("打得好", "看到我那波闪现开团没"),
    ("想不同意", "我想出六个无尽之刃"),
    ("求安慰", "你觉得我是不是很菜"),
    ("闲聊", "等下要不要再来一把"),
    ("要查", "现在什么版本了"),
    ("要查", "卡莎这版本胜率多少"),
    ("要查", "对面那个中单什么段位"),
]

WORDS = ["谢谢", "确实", "不是…而是", "总之", "加油", "随时"]
NOT_BUT = re.compile(r"不是[^。！？!?]{0,20}?而是")
STAGE = re.compile(r"[（(][^）)]{1,20}[）)]")  # 括号动作 / 旁白，例如“（拍拍你）”：念不出来


def load(tag, role):
    with open(os.path.join(PROMPTS, f"{tag}-{role}.json"), encoding="utf-8") as f:
        return json.load(f)


def tool(name, spec):
    return {"type": "function", "function": {"name": name, "description": spec["description"],
                                             "parameters": spec["parameters"]}}


def wire(name):
    return name.replace(".", "_")  # 同 NameMap：工具名里的点换成下划线


def tool_list(prompt, search):
    """导出的工具（新导出带 tools：speak、web.search、screen.at_speech、agent.start、reply.ask）；旧导出只有 speak。"""
    if "tools" in prompt:
        return [tool(wire(t["name"]), t) for t in prompt["tools"]]
    return [tool("speak", prompt["speak"]), tool("web_search", search)]


def ask(key, prompt, search, text, temperature):
    body = {
        "model": MODEL,
        "messages": [
            {"role": "system", "content": prompt["system"]},
            {"role": "latest_reminder", "content": REMINDER},
            {"role": "user", "content": prompt["user_prefix"] + text},
        ],
        "tools": tool_list(prompt, search),
        "thinking": {"type": "disabled"},
        "max_tokens": 1024,
        "stream": False,
    }
    if temperature is not None:
        body["temperature"] = temperature
    data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    reply = None
    for attempt in range(4):
        req = urllib.request.Request(URL, data=data, method="POST", headers={
            "Content-Type": "application/json", "Authorization": "Bearer " + key})
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                reply = json.loads(resp.read().decode("utf-8"))
            break
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503) and attempt < 3:
                time.sleep(2 + attempt * 3)
                continue
            return {"error": f"HTTP {e.code}: {e.read().decode('utf-8', 'ignore')[:200]}"}
        except (urllib.error.URLError, TimeoutError) as e:
            if attempt < 3:
                time.sleep(2)
                continue
            return {"error": str(e)[:200]}
    msg = reply["choices"][0]["message"]
    calls = msg.get("tool_calls") or []
    names = [c["function"]["name"] for c in calls]
    speak = next((c for c in calls if c["function"]["name"] == "speak"), None)
    given = {t["function"]["name"] for t in body["tools"]}
    out = {"content": (msg.get("content") or "").strip(), "tools": names, "spoke": speak is not None,
           # 先去做了别的（搜、看屏幕、交给后台、问玩家）：这一步没话是对的
           "searched": any(n != "speak" and n in given for n in names),
           "invented": [n for n in names if n not in given], "query": "", "text": "", "written": ""}
    for c in calls:
        try:
            args = json.loads(c["function"]["arguments"] or "{}")
        except json.JSONDecodeError:
            args = {}
        if c["function"]["name"] == "speak":
            out["text"] = str(args.get("text", "")).strip()
            out["written"] = str(args.get("written", "")).strip()
        elif c["function"]["name"] != "speak":
            out["query"] = (c["function"]["name"] + " " + str(args.get("query") or args.get("label") or args.get("question") or "")).strip()
    return out


def said(r):
    """念出来的话；没调 speak 就算正文（玩家听不到，但看口气）。先去搜了的没有话。"""
    return r.get("text") or ("" if r.get("searched") else r.get("content") or "")


def broken(r):
    """坏回答：编了工具名、既没说也没做别的（只写正文，玩家听不到）、说了空话、乱码、括号动作。"""
    if "error" in r:
        return False
    if r.get("invented"):
        return True
    if r.get("searched") and not r.get("spoke"):
        return False
    t = said(r)
    return (not r.get("spoke")) or not t or "�" in t or bool(STAGE.match(t))


def bigrams(s):
    s = re.sub(r"[\s，。！？、,.!?…～~\"“”]", "", s)
    return {s[i:i + 2] for i in range(len(s) - 1)}


def similarity(a, b):
    x, y = bigrams(a), bigrams(b)
    return len(x & y) / len(x | y) if x | y else 1.0


def demo_lines(soul):
    return [line[2:] for line in soul.splitlines() if line.startswith("你：")]


def copied(text, demos, n=6):
    """回答里有没有照抄示范：和任何一句示范有连续 n 个字一样。"""
    return any(d[i:i + n] in text for d in demos for i in range(len(d) - n + 1))


def metrics(rows, demos):
    ok = [r for r in rows if "error" not in r]
    texts = [said(r) for r in ok if said(r)]
    chars = sum(len(t) for t in texts) or 1
    n = max(len(texts), 1)
    m = {
        "回答数（说了话的）": len(texts),
        "错误数": len(rows) - len(ok),
        "平均字数": round(sum(len(t) for t in texts) / n, 1),
        "超 40 字": sum(1 for t in texts if len(t) > 40),
        "问句结尾": f"{sum(1 for t in texts if t.rstrip().endswith(('？', '?', '吗', '呢'))) / n:.0%}",
        "坏的（没调 speak / 编工具名 / 空 / 乱码 / 括号）": sum(1 for r in ok if broken(r)),
        "照抄示范（连续 6 字相同）": sum(1 for t in texts if demos and copied(t, demos)),
    }
    heads = collections.Counter(t.lstrip("“\"「…")[:1] for t in texts)
    if heads:
        head, count = heads.most_common(1)[0]
        m["最常见的开头字"] = f"{head} {count / n:.0%}"
    for w in WORDS:
        k = sum(len(NOT_BUT.findall(t)) if w == "不是…而是" else t.count(w) for t in texts)
        m[f"{w}/千字"] = round(k * 1000 / chars, 2)
    return m


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--roles", default="kotone,dafeiyu")
    ap.add_argument("--variants", default="old:none,new:1.3")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--out", default="report.md")
    args = ap.parse_args()
    roles = args.roles.split(",")
    variants = []
    for v in args.variants.split(","):
        tag, temp = v.split(":")
        variants.append((v, tag, None if temp == "none" else float(temp)))
    key = real_deepseek.read_key()
    if not key:
        sys.exit("没有 DeepSeek Key（凭据库里没有，也没设 KOTONE_REAL_DEEPSEEK_KEY）")
    prompts = {(tag, r): load(tag, r) for _, tag, _ in variants for r in roles}
    search = load("new", roles[0])["web_search"]  # catalog.rs 没改，两版一样
    jobs = {}
    with concurrent.futures.ThreadPoolExecutor(args.workers) as pool:
        for label, tag, temperature in variants:
            for role in roles:
                for i, (_, text) in enumerate(MOMENTS):
                    for k in range(args.runs):
                        fut = pool.submit(ask, key, prompts[(tag, role)], search, text, temperature)
                        jobs[fut] = (label, role, i, k)
        results = {}
        for n, fut in enumerate(concurrent.futures.as_completed(jobs), 1):
            results[jobs[fut]] = fut.result()
            if n % 40 == 0:
                print(f"{n}/{len(jobs)}", flush=True)
    del key

    os.makedirs(OUT, exist_ok=True)
    raw = [{"variant": v, "role": r, "moment": MOMENTS[i][1], "kind": MOMENTS[i][0], "run": k, **res}
           for (v, r, i, k), res in sorted(results.items())]
    with open(os.path.join(OUT, args.out.replace(".md", ".json")), "w", encoding="utf-8") as f:
        json.dump(raw, f, ensure_ascii=False, indent=1)

    labels = [v for v, _, _ in variants]
    lines = ["# 活人感评测", "", f"模型 {MODEL}，不思考；工具 speak、web.search、screen.at_speech、agent.start、reply.ask；每个时刻每组 {args.runs} 遍。"
             "组名是“提示版本:温度”（none = 不传，服务商默认）。", ""]
    for role in roles:
        lines += [f"## {role}", "", "| 指标 | " + " | ".join(labels) + " |", "|---" * (len(labels) + 1) + "|"]
        table = {}
        for label, tag, _ in variants:
            rows = [results[(label, role, i, k)] for i in range(len(MOMENTS)) for k in range(args.runs)]
            m = metrics(rows, demo_lines(prompts[(tag, role)]["soul"]))
            sims = []
            for i in range(len(MOMENTS)):
                outs = [said(results[(label, role, i, k)]) for k in range(args.runs)]
                outs = [o for o in outs if o]
                sims += [similarity(a, b) for x, a in enumerate(outs) for b in outs[x + 1:]]
            m["同一时刻几遍之间的重复率"] = f"{sum(sims) / max(len(sims), 1):.0%}"
            facts = [results[(label, role, i, k)] for i, (kind, _) in enumerate(MOMENTS) if kind == "要查"
                     for k in range(args.runs)]
            m["要查的：先去查了（搜 / 看屏幕 / 交后台）"] = f"{sum(1 for r in facts if r.get('searched'))}/{len(facts)}"
            m["要查的：没查就报了数字（可能是编的）"] = \
                f"{sum(1 for r in facts if not r.get('searched') and re.search(r'[0-9０-９]', said(r)))}/{len(facts)}"
            table[label] = m
        for name in table[labels[0]]:
            lines.append(f"| {name} | " + " | ".join(str(table[v][name]) for v in labels) + " |")
        lines += ["", "坏回答：", ""]
        for label in labels:
            for i, (_, text) in enumerate(MOMENTS):
                for k in range(args.runs):
                    r = results[(label, role, i, k)]
                    if broken(r):
                        shown = (r.get("text") or r.get("content") or "").replace("\n", " ")[:120]
                        lines.append(f"- {label}「{text}」：{shown}　〔工具 {','.join(r['tools']) or '无'}〕")
        lines += ["", "### 原文对照", ""]
        for i, (kind, text) in enumerate(MOMENTS):
            lines += [f"**{i + 1}. [{kind}] 玩家：{text}**", ""]
            for label in labels:
                for k in range(args.runs):
                    r = results[(label, role, i, k)]
                    if "error" in r:
                        shown = "（错误）" + r["error"]
                    elif r["searched"] and not r["spoke"]:
                        shown = f"〔先做：{r['query']}〕"
                    else:
                        shown = said(r) + ("" if r["spoke"] else "　〔没调 speak〕")
                        if r["written"]:
                            shown += f"　〔written {len(r['written'])} 字〕"
                    lines.append(f"- {label} #{k + 1}：{shown}")
            lines.append("")
    with open(os.path.join(OUT, args.out), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    print("写好了：" + os.path.join(OUT, args.out))


if __name__ == "__main__":
    main()
