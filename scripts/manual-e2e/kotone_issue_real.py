"""Real-machine check of the developer skill scripts/dev-skills/kotone-issue: she writes a GitHub issue draft,
asks the player, and only submits after the player says so.

Same setup as background_task_real.py (debug build with the web UI embedded, `--background`, isolated
KOTONE_HOME=target/issue-real/home, the player's real DeepSeek key in memory only, messages typed through WebView2's
DevTools port, input tools denied, permission mode "allow-all" so nothing but the skill keeps her from submitting).
Uses the machine's own `gh` login. Creates ONE real issue in the public repo l1veIn/kotone and closes it at the end.

Checks:
1. typed “帮我给 Kotone 提个 issue：…” → she reads the skill, hands it to the background; the background checks gh,
   reads the log, searches for duplicates and writes a draft under agent/files/issues/; NO issue is created yet; she
   shows the draft and asks whether to submit; the draft has the version and no user folder / key;
2. typed “可以，提交吧” → exactly one new issue with a bug / enhancement label; she says its number;
3. cleanup: the test issue is closed with a comment (always, even when a check fails).
`--draft-only` stops after 1 (no issue is created).
usage: python -X utf8 -I scripts/manual-e2e/kotone_issue_real.py [--draft-only]
"""
import glob
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import background_task_real as bt  # noqa: E402
import real_deepseek  # noqa: E402

REPO = bt.REPO
GH_REPO = "l1veIn/kotone"
SKILL = os.path.join(REPO, "scripts", "dev-skills", "kotone-issue")
bt.WORK = os.path.join(REPO, "target", "issue-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9337
bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
              WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}")
bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
check = bt.check

REQUEST = ("帮我给 Kotone 提个 issue（这是在测试提 issue 的技能，提交之后我会关掉）：界面切到 English 以后，"
           "标题栏 Kotone 后面还显示“琴音”两个中文字，我希望英文界面只显示 Kotone。")
CONFIRM = "可以，提交吧"
# --draft-only：只写草稿不提交，换一个仓库里没有的问题（上面那条第一次真跑时已经提交成 #21 并关掉了）
DRAFT_ONLY_REQUEST = ("帮我给 Kotone 提个 issue（测试提 issue 的技能，先别提交）：通话里我说了“退下”，"
                      "她已经不说话了，可是气泡上“通话中”还挂了好几秒才消失。")
USER_DIR = os.environ["USERPROFILE"]


def gh(*args):
    out = subprocess.run(["gh", *args], capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args)}: {out.stderr.strip()[:300]}")
    return out.stdout


def issue_numbers():
    rows = json.loads(gh("issue", "list", "--repo", GH_REPO, "--state", "all", "--limit", "30", "--json", "number"))
    return {r["number"] for r in rows}


def main_tools(main, since_text):
    msgs = bt.conversation(main).get("messages", [])
    start = next((i for i, m in enumerate(msgs) if m["role"] == "user" and m.get("text") == since_text), 0)
    return [c["name"] + ":" + json.loads(c.get("arguments") or "{}").get("name", "")
            for m in msgs[start:] if m["role"] == "assistant" for c in m.get("toolCalls", [])]


def private_bits(text):
    hits = []
    if USER_DIR.lower() in text.lower() or USER_DIR.replace("\\", "/").lower() in text.lower():
        hits.append("user folder")
    if "sk-" in text:
        hits.append("sk- key")
    return hits


def main():
    bt.guard()
    gh("auth", "status")
    shutil.rmtree(bt.WORK, ignore_errors=True)
    skills = os.path.join(bt.HOME, "agent", "workspace", "skills")
    os.makedirs(skills)
    shutil.copytree(SKILL, os.path.join(skills, "kotone-issue"))
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": bt.NO_INPUT}}}, f)
    before = issue_numbers()
    created = set()
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    try:
        bt.wait_ui()
        cid = real_deepseek.add_connection(bt.EXE, bt.ENV, model="deepseek-flash")
        bt.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        bt.invoke("agent_new_conversation", {})

        # 1. 写草稿，不提交，问玩家
        request = DRAFT_ONLY_REQUEST if "--draft-only" in sys.argv else REQUEST
        d = bt.run_case("draft", request, answer="allow-once", timeout=420)
        if not d:
            return bt.finish()
        tools = main_tools(d["main"], request)
        print("   main tools:", tools)
        check("she read the kotone-issue skill", any(t.startswith("skill.view") for t in tools + d["tools"]),
              ",".join(tools))
        check("she handed it to the background", any(t.startswith("agent.start") for t in tools))
        check("the background used gh and wrote a draft", "shell.run" in d["tools"] and "file.write" in d["tools"],
              ",".join(d["tools"]))
        created = issue_numbers() - before
        check("nothing was submitted before the player said so", not created, f"new: {sorted(created)}")
        drafts = glob.glob(os.path.join(bt.HOME, "agent", "files", "issues", "*.md"))
        draft = open(drafts[-1], encoding="utf-8").read() if drafts else ""
        check("the draft is in Kotone's files (issues/*.md)", drafts, ", ".join(os.path.basename(p) for p in drafts))
        print("   draft:\n" + "\n".join("     " + line for line in draft.splitlines()))
        version = json.load(open(os.path.join(REPO, "apps", "desktop", "package.json"), encoding="utf-8"))["version"]
        check("the draft has the sections and the version", "## 现象" in draft and version in draft, version)
        check("the draft has no user folder or key", not private_bits(draft), ", ".join(private_bits(draft)))
        said = d["said"]
        check("she shows the draft and asks before submitting",
              "提交" in said and any(q in said for q in ("吗", "？", "?")), said[:200])
        # 查重放在最后：读过别人写的 issue 之后不再跑命令，写草稿不用问玩家
        check("drafting did not need to ask the player", not d["questions"], " / ".join(d["questions"]))
        if "--draft-only" in sys.argv:
            return bt.finish()

        # 2. 玩家说可以：提交，说出编号
        s = bt.run_case("submit", CONFIRM, answer="allow-once", timeout=300)
        created = issue_numbers() - before
        check("exactly one issue was created", len(created) == 1, f"new: {sorted(created)}")
        if created:
            number = max(created)
            issue = json.loads(gh("issue", "view", str(number), "--repo", GH_REPO, "--json", "title,body,labels,url"))
            labels = [x["name"] for x in issue["labels"]]
            print(f"   issue #{number}: {issue['title']} {labels} {issue['url']}")
            check("it has a bug / enhancement label", set(labels) & {"bug", "enhancement"}, ",".join(labels))
            check("the issue has no user folder or key", not private_bits(issue["title"] + issue["body"]))
            check("she says the issue number", s and f"#{number}" in s.get("said", "") or str(number) in s.get("said", ""),
                  (s or {}).get("said", "")[:200])
    finally:
        for number in sorted(created | (issue_numbers() - before)):
            try:
                gh("issue", "close", str(number), "--repo", GH_REPO, "--comment",
                   "测试 Kotone 的提 issue 技能（scripts/dev-skills/kotone-issue，scripts/manual-e2e/kotone_issue_real.py），已关闭。")
                print(f"   closed test issue #{number}")
            except Exception as e:  # noqa: BLE001
                print(f"   could not close #{number}: {e}")
        real_deepseek.delete_connection(bt.EXE, bt.ENV)
        try:
            bt.call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        bt.time.sleep(3)
        if app.poll() is None:
            app.terminate()
        bt.sweep_credentials()
    return bt.finish()


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
