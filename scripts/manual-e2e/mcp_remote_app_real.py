"""MCP plan phase 2, UI part (2026-11-25): someone else's MCP App shows inside the real Kotone (WebView2).

The official ext-apps example `@modelcontextprotocol/server-basic-vanillajs` (MIT, `--stdio`) is added as a
third-party plugin through the plugins page — the real desktop app (debug build) in an isolated
KOTONE_HOME=target/mcp-remote-app-real/home, driven through WebView2's DevTools port (Playwright connectOverCDP) — no
OS input, nothing typed into other windows, no sound:

1. 插件 → 加 MCP 服务器 → paste `{"mcpServers": {"basic": {"command": "npx", "args": ["-y", <pkg>, "--stdio"]}}}`
   → 看看里面有什么 → 下一步 → the trust prompt shows the program and the package → 信任并添加;
2. its card shows up under 第三方插件 (badge, the server's own icon or the first-letter block), it connects;
3. click the card → its MCP App: an iframe on the loopback proxy (this plugin's own origin), handshake finished,
   the App's page came from the server (`resources/read`), with the CSP the server declared;
4. click the App's own button (inside the iframe, through DevTools) → the App calls its server's tool by the
   server's name (`get-time`) → Kotone maps it to `basic.get-time` and the result shows in the App;
5. Kotone's commands can't be called from the App's frames;
6. screenshots: target/plugin-shots/round7/mcp-remote-app*.png.

Stops before starting if a game is running or in front. Takes target/tmp/real.lock. Prerequisites:
`pnpm -C apps/desktop build:web`, then `cargo build -p kotone-tauri --features tauri/custom-protocol`; the exe is
copied to target/mcp-remote-app-real/bin. Needs node/npx and the network the first time (npm download).
usage: python -X utf8 -I scripts/manual-e2e/mcp_remote_app_real.py [--keep]
"""
import json
import re
import os
import shutil
import subprocess
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "mcp-remote-app-real")
HOME = os.path.join(WORK, "home")
SHOTS = os.path.join(REPO, "target", "plugin-shots", "round7")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
SRC_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
BIN = os.path.join(WORK, "bin")
EXE = os.path.join(BIN, "kotone.exe")
PORT = 9359
PACKAGE = "@modelcontextprotocol/server-basic-vanillajs"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
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


def ui(script, timeout=240):
    """Run Playwright code against the main window (`page`); whatever is put into `out` comes back."""
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
const out = {};
try {
%s
} catch (e) { out.error = String(e).slice(0, 600); } finally { console.log(JSON.stringify(out)); await browser.close(); }
""" % (PORT, script)
    proc = subprocess.run(["node", "--input-type=module", "-e", code], capture_output=True, text=True,
                          encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"), timeout=timeout)
    lines = proc.stdout.strip().splitlines()
    if not lines:
        raise RuntimeError((proc.stdout + proc.stderr)[-3000:])
    return json.loads(lines[-1])


def wait_ui(timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if "error" not in ui('await page.waitForSelector("[data-testid=app-sidebar]", { timeout: 2000 });', 30):
                return
        except Exception:  # noqa: BLE001
            pass
        time.sleep(1)
    raise RuntimeError("the main window did not come up")


def tools_quit():
    subprocess.run([EXE, "tools", "call", "app.quit"], capture_output=True, env=ENV, stdin=subprocess.DEVNULL,
                   timeout=30)


ADD = """
const paste = %s;
await page.getByTestId("settings-nav-plugins").click();
await page.getByTestId("plugins-add-mcp").click();
const dialog = page.getByTestId("mcp-add-dialog");
await dialog.getByTestId("mcp-add-paste").fill(paste);
await dialog.getByTestId("mcp-add-read").click();
await dialog.getByTestId("mcp-add-candidate").first().waitFor({ timeout: 10000 });
out.candidateId = await dialog.getByTestId("mcp-add-candidate-id").first().inputValue();
await dialog.getByTestId("mcp-add-next").click();
out.trustCommand = await dialog.getByTestId("mcp-trust-facts-command").first().innerText();
out.trustPackage = await dialog.getByTestId("mcp-trust-facts-package").first().innerText();
out.risk = await dialog.getByTestId("mcp-trust-risk").innerText();
await page.screenshot({ path: %s });
await dialog.getByTestId("mcp-trust-confirm").click();
const card = page.getByTestId("plugins-grid-third-party").getByTestId("plugins-card-basic");
await card.waitFor({ timeout: 15000 });
// 等它连上（开着、还不知道有哪些工具：加进来就去连；第一次 npx 要下载）
const until = Date.now() + 120000;
while (Date.now() < until) {
  const status = await card.getByTestId("plugins-card-basic-status").innerText().catch(() => "");
  out.status = status;
  if (status === "连上了") break;
  await page.waitForTimeout(1000);  // 插件页加完以后隔 2 秒重读一次清单
}
out.badge = await card.getByTestId("plugins-card-basic-third-party").innerText();
out.logo = await card.getByTestId("plugins-card-basic-logo").getAttribute("data-logo");
const list = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("plugins_list"));
const basic = list.find((p) => p.manifest.id === "basic");
out.remote = basic ? { status: basic.remote.status, error: basic.remote.error, pages: basic.remote.pages, trusted: basic.remote.trusted,
  tools: basic.tools.map((t) => t.name), title: basic.remote.server && basic.remote.server.title } : null;
await page.screenshot({ path: %s });
"""

OPEN = """
await page.getByTestId("settings-nav-plugins").click();
await page.getByTestId("plugins-card-basic-open").click();
const frame = page.locator('[data-testid="remote-plugin-app"] [data-testid="mcp-app-frame"]');
await frame.waitFor({ timeout: 20000 });
const deadline = Date.now() + 40000;
while (Date.now() < deadline && !["ready", "failed"].includes(await frame.getAttribute("data-phase"))) await page.waitForTimeout(250);
out.phase = await frame.getAttribute("data-phase");
out.plugin = await frame.getAttribute("data-plugin");
out.failed = await frame.getByTestId("mcp-app-failed").getAttribute("title").catch(() => null);
out.src = await frame.getByTestId("mcp-app-iframe").getAttribute("src").catch(() => null);
const app = () => page.frames().find((f) => /\\/app\\/basic\\/r[0-9a-f]{16}$/.test(f.url()));
const until = Date.now() + 20000;
while (Date.now() < until && !app()) await page.waitForTimeout(250);
const inner = app();
out.appUrl = inner ? inner.url().replace(/\\/[0-9a-f]{64}\\//, "/<key>/") : null;
out.before = inner ? (await inner.locator("body").innerText()).slice(0, 400) : "";
// App 自己的按钮（经 DevTools 点，不发系统按键）：它调自己服务器的工具（get-time）
if (inner) {
  const button = inner.locator("button").first();
  out.button = await button.innerText().catch(() => null);
  await button.click({ timeout: 5000 }).catch((e) => (out.clickError = String(e).slice(0, 200)));
  await page.waitForTimeout(3000);
  out.after = (await inner.locator("body").innerText()).slice(0, 400);
  out.appIpc = await inner.evaluate(async () => {
    const ipc = window.__TAURI_INTERNALS__;
    if (!ipc) return "absent";
    try {
      await Promise.race([ipc.invoke("plugins_list"), new Promise((_, no) => setTimeout(() => no(new Error("timeout")), 4000))]);
      return "allowed";
    } catch (e) { return "refused: " + String(e).slice(0, 80); }
  });
}
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
        # npx 先把包下载好（连接的启动超时是 10 秒）
        npx = shutil.which("npx") or "npx"
        subprocess.run([npx, "-y", "--package", PACKAGE, "node", "-e", "0"], capture_output=True, timeout=300)
        app = subprocess.Popen([EXE], env=ENV)
        wait_ui()
        paste = json.dumps({"mcpServers": {"basic": {"command": "npx", "args": ["-y", PACKAGE, "--stdio"]}}})
        r = ui(ADD % (json.dumps(paste), json.dumps(os.path.join(SHOTS, "mcp-remote-trust-prompt.png")),
                      json.dumps(os.path.join(SHOTS, "mcp-remote-card.png"))))
        if r.get("error"):
            print("  error:", r["error"])
        check("paste → candidate id basic", r.get("candidateId") == "basic", str(r.get("candidateId")))
        check("trust prompt shows the program and the package",
              PACKAGE in (r.get("trustCommand") or "") and PACKAGE in (r.get("trustPackage") or ""),
              f"{r.get('trustCommand')} / {r.get('trustPackage')}")
        check("trust prompt says what the risk is", "别人写的程序" in (r.get("risk") or ""))
        remote = r.get("remote") or {}
        print("  remote:", json.dumps(remote, ensure_ascii=False)[:400])
        check("its card is under 第三方插件 with the badge, and it connected",
              r.get("badge") == "第三方" and remote.get("status") == "connected" and remote.get("trusted") is True,
              f"badge={r.get('badge')} status={r.get('status')} logo={r.get('logo')}")
        check("it has an MCP App page (ui://) and its tools are named basic.<tool>",
              remote.get("pages") and all(t.startswith("basic.") for t in remote.get("tools") or []),
              json.dumps(remote.get("pages"), ensure_ascii=False))
        r = ui(OPEN % (json.dumps(os.path.join(SHOTS, "mcp-remote-app-window.png")),
                       json.dumps(os.path.join(SHOTS, "mcp-remote-app.png"))))
        if r.get("error"):
            print("  error:", r["error"])
        print("  frame:", json.dumps({k: r.get(k) for k in ("phase", "plugin", "failed", "appUrl", "button", "clickError")},
                                     ensure_ascii=False))
        print("  before:", (r.get("before") or "").replace("\n", " ")[:200])
        print("  after: ", (r.get("after") or "").replace("\n", " ")[:200])
        # App 的 CSP 在代理给的 HTTP 头里（主窗口的 fetch 被它自己的 CSP 挡着，这里直接问代理；代理只认本机）
        csp = ""
        if r.get("src"):
            try:
                with urllib.request.urlopen(r["src"].replace("/sandbox/", "/app/"), timeout=10) as resp:
                    csp = resp.headers.get("content-security-policy") or ""
            except Exception as e:  # noqa: BLE001
                csp = f"fetch: {e}"
        r["csp"] = csp
        print("  App CSP:", re.sub(r"/[0-9a-f]{64}/", "/<key>/", csp)[:300])
        check("its App is an iframe on the loopback proxy, handshake finished, bound to the basic plugin",
              r.get("phase") == "ready" and r.get("plugin") == "basic"
              and (r.get("src") or "").startswith("http://127.0.0.1:") and "/sandbox/basic/r" in (r.get("src") or ""),
              f"phase={r.get('phase')} {r.get('failed') or ''}")
        check("the inner frame is the server's page from the proxy's /app/ path", bool(r.get("appUrl")), str(r.get("appUrl")))
        check("the App's CSP starts from default-src 'none'", (r.get("csp") or "").startswith("default-src 'none'"))
        check("the App called its server's tool through Kotone (its text changed after the click)",
              r.get("after") and r.get("after") != r.get("before") and not r.get("clickError"),
              (r.get("after") or "")[:120].replace("\n", " "))
        check("Kotone's commands cannot be called from the App", r.get("appIpc") in ("absent", None)
              or str(r.get("appIpc")).startswith("refused"), str(r.get("appIpc")))
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
