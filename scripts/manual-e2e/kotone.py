"""Drive an isolated Kotone instance for manual real-machine tests (see README.md).

usage:
  python -X utf8 -I kotone.py call <tool> [json-args]     # tools call; {"source_file": "x.lua"} reads a Luau file
  python -X utf8 -I kotone.py runs                        # automation.runs, one line per run
  python -X utf8 -I kotone.py wait <run-id> [wait-ms]     # wait for automation.finished of that run
  python -X utf8 -I kotone.py ui <names|invoke|setvalue> [name] [value] [index]   # UI Automation on the settings window

Always use `python -X utf8 -I`: `-I` ignores PYTHONIOENCODING, so without `-X utf8` pipes are read as GBK.
"""
import base64
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
# The repo carries a Low integrity label: an exe started from inside it cannot send input to normal windows,
# so the test build is copied outside the repo (see README.md).
EXE = os.path.expandvars(r"%LOCALAPPDATA%\Temp\kotone-automation-e2e\kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=os.path.join(REPO, "target", "automation-e2e", "home"))


def cli(args, stdin=None):
    out = subprocess.run([EXE, *args], input=stdin, capture_output=True, text=True, encoding="utf-8", env=ENV)
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"raw": out.stdout, "err": out.stderr}


def call(tool, args=None):
    return cli(["tools", "call", tool, "--args-file", "-"], json.dumps(args if args is not None else {}))


def brief(run):
    a = run["action"]
    return {
        "id": run["id"], "kind": a["kind"], "title": a.get("name") or a.get("text"),
        "status": run["status"], "message": run.get("message"),
        "dueIn_s": round((run["dueAtMs"] - time.time() * 1000) / 1000, 1),
        "keys": (run.get("report") or {}).get("keys"),
    }


def ui(argv):
    b = lambda t: base64.b64encode(t.encode()).decode()
    args = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", os.path.join(HERE, "uia.ps1"), "-Action", argv[0]]
    if len(argv) > 1:
        args += ["-NameB64", b(argv[1])]
    if len(argv) > 2:
        args += ["-ValueB64", b(argv[2])]
    if len(argv) > 3:
        args += ["-Index", argv[3]]
    text = subprocess.run(args, capture_output=True).stdout.decode("utf-8", errors="replace")
    if argv[0] == "names":
        text = "\n".join(l for l in text.splitlines() if not l.startswith(("Group|", "Pane|")))
    print(text)


def main():
    cmd = sys.argv[1]
    if cmd == "call":
        args = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
        if "source_file" in args:
            args["source"] = open(args.pop("source_file"), encoding="utf-8").read()
        print(json.dumps(call(sys.argv[2], args), ensure_ascii=False))
    elif cmd == "runs":
        env = call("automation.runs")
        r = env.get("result") or env
        print(json.dumps({k: r.get(k) for k in ("running", "pending", "maxRunning")}, ensure_ascii=False))
        for run in r.get("runs", []):
            print(json.dumps(brief(run), ensure_ascii=False))
    elif cmd == "wait":
        run_id = sys.argv[2]
        remaining = int(sys.argv[3]) if len(sys.argv) > 3 else 20000
        cur = cli(["tools", "events", "--from", "oldest", "--types", "automation.finished", "--wait-ms", "0"])["result"]["cursor"]
        while remaining > 0:
            step = min(remaining, 30000)
            result = cli(["tools", "events", "--cursor", cur, "--types", "automation.finished", "--wait-ms", str(step)]).get("result", {})
            cur = result.get("cursor", cur)
            for event in result.get("events", []):
                if event["data"].get("id") == run_id:
                    print("FINISHED", json.dumps(brief(event["data"]), ensure_ascii=False))
                    return
            remaining -= step
        print("TIMEOUT")
    elif cmd == "ui":
        ui(sys.argv[2:])
    else:
        raise SystemExit(__doc__)


main()
