"""MCP plan phase 3 (2026-11-25, ADR 039 D5): the League of Legends MCP App shows inside the real Kotone (WebView2).

The real desktop app (debug build) in an isolated KOTONE_HOME=target/mcp-app-real-ui/home, driven through WebView2's
DevTools port (Playwright connectOverCDP) — no OS input, nothing typed, no sound:

- 插件 → 英雄联盟 → 强度榜: the tab is an iframe (McpAppFrame) pointing at the loopback proxy's sandbox page
  (http://127.0.0.1:<port>/<key>/sandbox/lol/tier); the handshake finishes (data-phase=ready), which only happens if
  neither the main window's CSP (frame-src, in both tauri.conf.json and index.html), the App's CSP (HTTP header from
  the proxy) nor mixed content blocked anything; the inner frame is the App (/<key>/app/lol/tier) and shows the tier
  list it fetched through the host (lol.tier → Tencent 101: one public request, no account; a network failure shows the
  App's status line instead and is reported, not failed);
- the App's frames have no Tauri IPC (`window.__TAURI_INTERNALS__` is undefined in both the sandbox page and the App);
- the proxy refuses requests without the key (a plain GET to the origin is 404);
- how many champion icons loaded (client assets → CommunityDragon; informational, needs the network);
- screenshots of the window and of the App go to target/plugin-shots/round7/.

The window opens on screen (it is a normal start), but nothing is clicked with the OS — only DevTools. Stops before
starting if a game is running or in front. Takes target/tmp/real.lock (the shared rule for scripts that open a Kotone
window). Prerequisite (README 准备): `pnpm -C apps/desktop build:apps`, `pnpm -C apps/desktop build:web`, then
`cargo build -p kotone-tauri --features tauri/custom-protocol` (the web UI and the App embedded); the exe is copied to
target/mcp-app-real-ui/bin first (a running exe would block other builds).
usage: python -X utf8 -I scripts/manual-e2e/mcp_app_real.py [--keep]
"""
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "mcp-app-real-ui")
HOME = os.path.join(WORK, "home")
SHOTS = os.path.join(REPO, "target", "plugin-shots", "round7")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
SRC_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
BIN = os.path.join(WORK, "bin")
EXE = os.path.join(BIN, "kotone.exe")
PORT = 9336
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def guard():
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, encoding="mbcs",
                         errors="replace").stdout.lower()
    running = [g for g in ("league of legends.exe", "leagueclient", "riotclient", "valorant", "cs2.exe",
                           "dota2.exe", "yuanshen", "genshinimpact") if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing while the player plays")
    path = foreground_process()
    if any(g in os.path.basename(path).lower() for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)


def ui(script):
    """Run Playwright code against the main window (`page`); whatever is put into `out` comes back."""
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
const out = {};
try {
%s
} finally { console.log(JSON.stringify(out)); await browser.close(); }
""" % (PORT, script)
    proc = subprocess.run(["node", "--input-type=module", "-e", code], capture_output=True, text=True,
                          encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    if proc.returncode != 0:
        raise RuntimeError((proc.stdout + proc.stderr)[-3000:])
    return json.loads(proc.stdout.strip().splitlines()[-1])


def wait_ui(timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            ui('await page.waitForSelector("[data-testid=app-sidebar]", { timeout: 2000 });')
            return
        except Exception:  # noqa: BLE001
            time.sleep(1)
    raise RuntimeError("the main window did not come up")


def tools_quit():
    subprocess.run([EXE, "tools", "call", "app.quit"], capture_output=True, env=ENV, stdin=subprocess.DEVNULL,
                   timeout=30)


SHOW_TIER = """
await page.getByTestId("settings-nav-plugins").click();
await page.getByTestId("plugins-card-lol-open").click();
await page.getByTestId("plugin-lol-tab-tier").click();
const frame = page.locator('[data-testid="mcp-app-frame"][data-uri="ui://kotone/lol/tier"]');
await frame.waitFor({ timeout: 15000 });
const deadline = Date.now() + 30000;
while (Date.now() < deadline && !["ready", "failed"].includes(await frame.getAttribute("data-phase"))) await page.waitForTimeout(250);
out.phase = await frame.getAttribute("data-phase");
out.display = await frame.getAttribute("data-display");
out.plugin = await frame.getAttribute("data-plugin");
out.src = await frame.getByTestId("mcp-app-iframe").getAttribute("src").catch(() => null);
out.sandbox = await frame.getByTestId("mcp-app-iframe").getAttribute("sandbox").catch(() => null);
out.failed = await frame.getByTestId("mcp-app-failed").getAttribute("title").catch(() => null);
// 等 App 把强度榜（或者一句状态）画出来
const app = () => page.frames().find((f) => /\\/app\\/lol\\/tier$/.test(f.url()));
const until = Date.now() + 40000;
while (Date.now() < until) {
  const f = app();
  const text = f ? await f.locator("body").innerText().catch(() => "") : "";
  if (/T0|T1|腾讯|没读到|查不到|连不上/.test(text)) break;
  await page.waitForTimeout(500);
}
const inner = app();
const sandboxFrame = page.frames().find((f) => /\\/sandbox\\/lol\\/tier$/.test(f.url()));
out.appUrl = inner ? inner.url() : null;
out.appText = inner ? (await inner.locator("body").innerText()).slice(0, 600) : "";
// Tauri 在 WebView2 里把它的初始化脚本也放进了 iframe（__TAURI_INTERNALS__ 在），要看的是命令调不调得通：
// 回环代理的源不在任何 capability 里（没有 remote.urls），Tauri 应该拒绝
// 回不来也可能是“跑了但回不到 iframe”：另外调一个会留痕的（log_frontend_error 往 kotone.log 写一行），看日志里有没有
const tryInvoke = (frame, probe) => frame.evaluate(async (probe) => {
  const ipc = window.__TAURI_INTERNALS__;
  if (!ipc) return { present: false };
  const out = { present: true };
  for (const [cmd, args] of [["plugins_list", {}], ["plugin_call", { plugin: "lol", tool: "lol.champions", arguments: {} }], ["get_settings", {}], ["log_frontend_error", { context: "ipc-probe", message: probe }]]) {
    try {
      const value = await Promise.race([ipc.invoke(cmd, args), new Promise((_, no) => setTimeout(() => no(new Error("timeout")), 5000))]);
      out[cmd] = "allowed: " + JSON.stringify(value).slice(0, 80);
    } catch (e) {
      out[cmd] = "refused: " + String(e).slice(0, 160);
    }
  }
  return out;
}, probe);
out.appIpc = inner ? await tryInvoke(inner, "IPC-PROBE-FROM-APP") : null;
out.sandboxIpc = sandboxFrame ? await tryInvoke(sandboxFrame, "IPC-PROBE-FROM-SANDBOX") : null;
// 对照：主窗口自己调同一个命令，日志里应该有
await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("log_frontend_error", { context: "ipc-probe", message: "IPC-PROBE-FROM-MAIN" }));
out.icons = inner ? await inner.evaluate(() => {
  const imgs = [...document.querySelectorAll("img")];
  return { total: imgs.length, loaded: imgs.filter((i) => i.complete && i.naturalWidth > 0).length, sample: imgs.slice(0, 2).map((i) => i.currentSrc.replace(/\\/[0-9a-f]{64}\\//, "/<key>/")) };
}) : null;
await page.screenshot({ path: %s });
await frame.screenshot({ path: %s });
"""


def main():
    guard()
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"missing {SRC_EXE}: build it first (see the docstring)")
    try:
        os.makedirs(LOCK)
    except FileExistsError:
        raise SystemExit(f"STOP: {LOCK} exists (another real-machine script is running)")
    app = None
    try:
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        os.makedirs(SHOTS, exist_ok=True)
        shutil.copy2(SRC_EXE, EXE)
        with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
            json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                       "toolService": {"enabled": True}, "hotkey": {"key": "F24", "mode": "toggle"},
                       "runtimeToggleHotkey": "", "automationStopHotkey": "",
                       "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                     "wake": {"enabled": False}}}, f)
        app = subprocess.Popen([EXE], env=ENV)
        wait_ui()
        r = ui(SHOW_TIER % (json.dumps(os.path.join(SHOTS, "real-tier-app-window.png")),
                            json.dumps(os.path.join(SHOTS, "real-tier-app.png"))))
        print("  frame:", json.dumps({k: r.get(k) for k in ("phase", "display", "plugin", "failed")}, ensure_ascii=False))
        print("  app text:", (r.get("appText") or "").replace("\n", " ")[:300])
        print("  icons:", json.dumps(r.get("icons"), ensure_ascii=False))
        check("the 强度榜 tab is an iframe on the loopback proxy's sandbox page",
              bool(r.get("src")) and r["src"].startswith("http://127.0.0.1:") and "/sandbox/lol/tier" in r["src"],
              (r.get("src") or "").split("/")[2] if r.get("src") else "no iframe")
        check("sandbox attribute is allow-scripts allow-same-origin allow-forms",
              r.get("sandbox") == "allow-scripts allow-same-origin allow-forms")
        check("the MCP Apps handshake finished in WebView2 (CSP / frame-src / mixed content did not block it)",
              r.get("phase") == "ready", f"phase {r.get('phase')} {r.get('failed') or ''}")
        check("fullscreen, bound to the lol plugin", r.get("display") == "fullscreen" and r.get("plugin") == "lol")
        check("the inner frame is the App from the proxy's /app/ path",
              bool(r.get("appUrl")) and r["appUrl"].endswith("/app/lol/tier") and r["appUrl"].startswith("http://127.0.0.1:"))
        text = r.get("appText") or ""
        if "T0" in text or "T1" in text:
            check("the App shows the tier list it fetched through the host (lol.tier)", True, text[:80].replace("\n", " "))
        else:
            check("the App shows something (no data: network / Tencent 101?)", bool(text.strip()), text[:200])
        print("  IPC from the App:", json.dumps(r.get("appIpc"), ensure_ascii=False))
        print("  IPC from the sandbox page:", json.dumps(r.get("sandboxIpc"), ensure_ascii=False))
        allowed = [f"{where}:{cmd}" for where in ("appIpc", "sandboxIpc") for cmd, v in (r.get(where) or {}).items()
                   if isinstance(v, str) and v.startswith("allowed")]
        log = ""
        try:
            with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
                log = f.read()
        except OSError:
            pass
        ran = [p for p in ("IPC-PROBE-FROM-APP", "IPC-PROBE-FROM-SANDBOX") if p in log]
        check("Kotone's commands (Tauri IPC) cannot be called from the App or the sandbox page",
              not allowed and not ran and "IPC-PROBE-FROM-MAIN" in log,
              f"answered: {allowed}, ran (in kotone.log): {ran}, main window probe logged: {'IPC-PROBE-FROM-MAIN' in log}")
        origin = "/".join((r.get("src") or "http://127.0.0.1:1/").split("/")[:3])
        try:
            urllib.request.urlopen(origin + "/sandbox/lol/tier", timeout=5)
            status = 200
        except urllib.error.HTTPError as e:
            status = e.code
        except Exception as e:  # noqa: BLE001
            status = str(e)
        check("the proxy refuses a request without the key", status == 404, str(status))
        print("  screenshots:", SHOTS)
    finally:
        try:
            tools_quit()
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        if app is not None:
            try:
                app.wait(15)
            except subprocess.TimeoutExpired:
                app.terminate()
                app.wait(10)
        os.rmdir(LOCK)
        if "--keep" not in sys.argv:
            for _ in range(10):
                shutil.rmtree(WORK, ignore_errors=True)
                if not os.path.exists(WORK):
                    break
                time.sleep(1)
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; FAILED: {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


main()
