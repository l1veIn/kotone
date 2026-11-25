"""MCP plan phase 3 (2026-11-25, ADR 039 D5): the League of Legends MCP App shows inside the real Kotone (WebView2).

The real desktop app (debug build) in an isolated KOTONE_HOME=target/mcp-app-real-ui/home, driven through WebView2's
DevTools port (Playwright connectOverCDP) — no OS input, nothing typed, no sound. A fake LoL client (the HTTPS replay
of one recorded game from lol_tools_real.py: LCU with a random token + in-game API, phase InProgress) is the client, so
the pages that need “读客户端数据” have real-shaped data; KOTONE_LOL_LCU / KOTONE_LOL_LIVE point the app at it.

1. 插件 → 英雄联盟: the whole page is the App, starting at “这局” (client data on). The frame is an iframe (McpAppFrame)
   pointing at the loopback proxy's sandbox page (http://127.0.0.1:<port>/<key>/sandbox/lol/match); the handshake
   finishes (data-phase=ready), which only happens if neither the main window's CSP (frame-src, in both
   tauri.conf.json and index.html), the App's CSP (HTTP header from the proxy) nor mixed content blocked anything; the
   inner frame is the App (/<key>/app/lol/match) and shows this game from the fake client;
   - the App's frames have no Tauri IPC (commands are refused in both the sandbox page and the App);
   - the proxy refuses requests without the key (a plain GET to the origin is 404).
2. Each page opened on its own, the way chat cards, the “这局总结” auto card and `ui.open` open them (2026-11-25 review:
   战绩 / 单局详情 / 英雄 were marked as Apps in the frontend but not in Rust, so they showed “页面没有” in the real
   Kotone while the browser preview passed): `kotone.exe tools call ui.open` with parameters → the right sidebar shows
   the App for 战绩 (the recorded player's puuid), 单局详情 (the recorded gameId), 英雄 (亚索, rift) and 强度榜 (rift, 中);
   for each: no “页面没有” (mcp-app-failed), handshake ready, the inner frame is /app/lol/<page>, and the page shows
   its data (战绩 / 单局详情 from the fake client must; 英雄 / 强度榜 need one public request to Tencent 101 — a network
   failure shows the App's status line instead and is reported, not failed).
   Before that (shared-resources audit §2 item 6, 2026-11-25): `ui.open` while the player is on the plugin page does not
   switch pages — the result says status "waiting" / opened false (the window answered over `ui_open_reply`), the plugin
   page stays, one in-window notice names the page, and on the chat page the right sidebar's Cards has exactly one
   waiting card for it (opening it twice counts once). Then the script goes to the chat page (DevTools click) and every
   `ui.open` above says status "shown": the first into a free sidebar, the rest stack on the same plugin's App.
- how many champion icons loaded (client assets → CommunityDragon; informational, needs the network);
- screenshots of the window and of each App go to target/plugin-shots/round7/real-*.png.

The window opens on screen (it is a normal start), but nothing is clicked with the OS — only DevTools. Stops before
starting if a game is running or in front. Takes target/tmp/real.lock (the shared rule for scripts that open a Kotone
window). Prerequisite (README 准备): `pnpm -C apps/desktop build:apps`, `pnpm -C apps/desktop build:web`, then
`cargo build -p kotone-tauri --features tauri/custom-protocol` (the web UI and the App embedded); the exe is copied to
target/mcp-app-real-ui/bin first (a running exe would block other builds). The recording (default
target/lol-probe/20261123-210745, see lol_tools_real.py) has real player names: it stays under target/.
usage: python -X utf8 -I scripts/manual-e2e/mcp_app_real.py [--probe DIR] [--gamedata DIR] [--keep]
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
import lol_tools_real as lt  # noqa: E402
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
    # env=ENV：脚本里用 child_process 调 kotone.exe tools call 时连的是这个隔离的实例
    proc = subprocess.run(["node", "--input-type=module", "-e", code], capture_output=True, text=True,
                          encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"), env=ENV)
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


# 等一个 McpAppFrame 握手完（ready / failed），记下它的样子。`frame` 是它的 locator。
FRAME_STATE = """
const deadline = Date.now() + 30000;
while (Date.now() < deadline && !["ready", "failed"].includes(await frame.getAttribute("data-phase"))) await page.waitForTimeout(250);
out.phase = await frame.getAttribute("data-phase");
out.uri = await frame.getAttribute("data-uri");
out.display = await frame.getAttribute("data-display");
out.plugin = await frame.getAttribute("data-plugin");
out.src = await frame.getByTestId("mcp-app-iframe").getAttribute("src").catch(() => null);
out.sandbox = await frame.getByTestId("mcp-app-iframe").getAttribute("sandbox").catch(() => null);
out.failed = await frame.getByTestId("mcp-app-failed").count() ? await frame.getByTestId("mcp-app-failed").getAttribute("title") : null;
"""

# 等 App（内层 iframe，路径 /app/lol/<页面>）画出 `want` 这个 testid，或者一句状态；记下文字、数量、图
APP_CONTENT = """
const appFrame = () => page.frames().filter((f) => f.url().endsWith("/app/lol/" + PAGE)).pop();
const until = Date.now() + 45000;
while (Date.now() < until) {
  const f = appFrame();
  if (f) {
    const n = await f.locator(`[data-testid="${WANT}"]`).count().catch(() => 0);
    const status = await f.locator('[data-testid="lol-app-status"]').count().catch(() => 0);
    if (n > 0 || status > 0) break;
  }
  await page.waitForTimeout(500);
}
const inner = appFrame();
out.appUrl = inner ? inner.url() : null;
out.count = inner ? await inner.locator(`[data-testid="${WANT}"]`).count() : 0;
out.status = inner && await inner.locator('[data-testid="lol-app-status"]').count()
  ? (await inner.locator('[data-testid="lol-app-status"]').first().innerText()).slice(0, 200) : null;
out.appText = inner ? (await inner.locator("body").innerText()).slice(0, 600) : "";
// 图多给一点时间（素材层 → CommunityDragon）
await page.waitForTimeout(1500);
out.icons = inner ? await inner.evaluate(() => {
  const imgs = [...document.querySelectorAll("img")];
  return { total: imgs.length, loaded: imgs.filter((i) => i.complete && i.naturalWidth > 0).length, sample: imgs.slice(0, 2).map((i) => i.currentSrc.replace(/\\/[0-9a-f]{64}\\//, "/<key>/")) };
}) : null;
"""

SHOW_PLUGIN_PAGE = """
await page.getByTestId("settings-nav-plugins").click();
await page.getByTestId("plugins-card-lol-open").click();
const frame = page.getByTestId("plugin-page-lol").getByTestId("mcp-app-frame");
await frame.waitFor({ timeout: 15000 });
%s
const PAGE = "match", WANT = "lol-match-player";
%s
const sandboxFrame = page.frames().find((f) => /\\/sandbox\\/lol\\/match$/.test(f.url()));
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
await page.screenshot({ path: %s });
await frame.screenshot({ path: %s });
"""

# 在连着调试端口的时候再 ui.open（kotone.exe tools call，和她、外部 Agent 打开页面是同一条路）：连上之前就已经在的
# 跨源 iframe 里的子框架，connectOverCDP 后 Playwright 看不到（第一次跑时战绩、单局详情、强度榜都是“no App frame”，
# 截图里页面其实好好的）
SHOW_SIDEBAR_PAGE = """
const URI = %s;
const { spawnSync } = await import("node:child_process");
const opened = spawnSync(%s, ["tools", "call", "ui.open", "--args-file", "-", "--timeout-ms", "30000"],
  { input: JSON.stringify({ page: URI, params: %s }), encoding: "utf-8", timeout: 60000 });
try { out.opened = JSON.parse(opened.stdout); } catch { out.opened = { ok: false, raw: String(opened.stdout) + String(opened.stderr) }; }
const frame = page.locator(`[data-testid="sidebar-plugin-page"][data-uri="${URI}"] [data-testid="mcp-app-frame"][data-uri="${URI}"]`);
await frame.waitFor({ timeout: 20000 });
%s
const PAGE = %s, WANT = %s;
%s
await page.screenshot({ path: %s });
await frame.screenshot({ path: %s });
"""

# 玩家在插件页时 ui.open（共享资源评估第二节第 6 条）：不切页，放在聊天页右侧栏等玩家点；同一个页面开两次只算一次。
# 看完回到聊天页、把等着的卡片拿掉，后面每一页都直接显示在右侧栏
WAITING_PAGE = """
const { spawnSync } = await import("node:child_process");
const call = (uri, params) => {
  const r = spawnSync(%s, ["tools", "call", "ui.open", "--args-file", "-", "--timeout-ms", "30000"],
    { input: JSON.stringify({ page: uri, params }), encoding: "utf-8", timeout: 60000 });
  try { return JSON.parse(r.stdout); } catch { return { ok: false, raw: String(r.stdout) + String(r.stderr) }; }
};
out.first = call("ui://kotone/lol/player", %s);
out.again = call("ui://kotone/lol/player", %s);
await page.waitForTimeout(500);
out.stillPlugin = await page.getByTestId("plugin-page-lol").isVisible();
out.chatVisible = await page.getByTestId("assistant-chat").isVisible();
out.toasts = await page.locator("[data-sonner-toast]").allInnerTexts();
await page.screenshot({ path: %s });
// 隔离的 KOTONE_HOME 里还没有对话：左侧栏的“新对话”打开聊天页
await page.getByTestId("sidebar-new-chat").click();
await page.getByTestId("assistant-chat").waitFor({ timeout: 10000 });
out.waiting = await page.getByTestId("ui-open-waiting").evaluateAll((els) => els.map((e) => e.getAttribute("data-uri")));
out.tab = await page.getByTestId("right-sidebar").getAttribute("data-tab").catch(() => null);
await page.screenshot({ path: %s });
for (const el of await page.getByTestId("ui-open-waiting-dismiss").all()) await el.click();
out.left = await page.getByTestId("ui-open-waiting").count();
"""


# 单独打开的页面（照聊天卡片、“这局总结”自动卡片、ui.open 的路）：(页面, 参数, 有数据时出现的 testid, 要不要网)
def sidebar_pages(rec):
    return [
        ("player", {"puuid": rec.me["puuid"]}, "lol-player-game", False),
        ("game", {"gameId": (rec.game_detail or {}).get("gameId") or rec.session["gameData"]["gameId"]},
         "lol-game-player", False),
        ("champion", {"champion": "亚索", "mode": "rift"}, "lol-champion-name", True),
        ("tier", {"mode": "rift", "lane": "中"}, "lol-app-tier-row", True),
    ]


def frame_checks(where, r, page):
    check(f"{where}: an iframe on the loopback proxy's sandbox page for {page}",
          bool(r.get("src")) and r["src"].startswith("http://127.0.0.1:") and f"/sandbox/lol/{page}" in r["src"],
          (r.get("src") or "").split("/")[2] if r.get("src") else f"no iframe, failed: {r.get('failed')}")
    check(f"{where}: not “页面没有”, and the MCP Apps handshake finished in WebView2",
          r.get("phase") == "ready" and r.get("failed") is None, f"phase {r.get('phase')} {r.get('failed') or ''}")
    check(f"{where}: the inner frame is the App from the proxy's /app/lol/{page} path",
          bool(r.get("appUrl")) and r["appUrl"].endswith(f"/app/lol/{page}") and r["appUrl"].startswith("http://127.0.0.1:"),
          r.get("appUrl") or "no App frame")


def main():
    guard()
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"missing {SRC_EXE}: build it first (see the docstring)")
    try:
        os.makedirs(LOCK)
    except FileExistsError:
        raise SystemExit(f"STOP: {LOCK} exists (another real-machine script is running)")
    app = None
    server = None
    try:
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        os.makedirs(SHOTS, exist_ok=True)
        shutil.copy2(SRC_EXE, EXE)
        # 假客户端（lol_tools_real.py 的录像回放；证书写在这个脚本的工作目录里）
        lt.bt.WORK = WORK
        lt.REC = lt.Recording(lt.PROBE, lt.GAMEDATA)
        server, port = lt.serve()
        ENV.update(KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
        print(f"fake client on 127.0.0.1:{port} (recording {os.path.basename(lt.PROBE)}, queue {lt.REC.queue()})")
        with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
            json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                       "toolService": {"enabled": True}, "hotkey": {"key": "F24", "mode": "toggle"},
                       "runtimeToggleHotkey": "", "automationStopHotkey": "",
                       "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                     "wake": {"enabled": False}}}, f)
        app = subprocess.Popen([EXE], env=ENV)
        wait_ui()
        # 后台先连上假客户端、把 10 个人的战绩查好（“这局”要有人）
        time.sleep(8)

        # 1. 插件页：整页是 App，从“这局”开始
        r = ui(SHOW_PLUGIN_PAGE % (FRAME_STATE, APP_CONTENT,
                                   json.dumps(os.path.join(SHOTS, "real-plugin-page-window.png")),
                                   json.dumps(os.path.join(SHOTS, "real-match-app.png"))))
        print("  frame:", json.dumps({k: r.get(k) for k in ("phase", "uri", "display", "plugin", "failed")},
                                     ensure_ascii=False))
        print("  app text:", (r.get("appText") or "").replace("\n", " ")[:300])
        print("  icons:", json.dumps(r.get("icons"), ensure_ascii=False))
        check("插件 → 英雄联盟 starts the App at 这局 (client data on)", r.get("uri") == "ui://kotone/lol/match",
              str(r.get("uri")))
        frame_checks("plugin page", r, "match")
        check("sandbox attribute is allow-scripts allow-same-origin allow-forms",
              r.get("sandbox") == "allow-scripts allow-same-origin allow-forms")
        check("fullscreen, bound to the lol plugin", r.get("display") == "fullscreen" and r.get("plugin") == "lol")
        check("这局 shows the players of the recorded game (from the fake client)", r.get("count", 0) >= 10,
              f"{r.get('count')} players; status {r.get('status')}")
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
            urllib.request.urlopen(origin + "/sandbox/lol/match", timeout=5)
            status = 200
        except urllib.error.HTTPError as e:
            status = e.code
        except Exception as e:  # noqa: BLE001
            status = str(e)
        check("the proxy refuses a request without the key", status == 404, str(status))

        # 1.5 玩家还在插件页：ui.open 不切过去，放在聊天页右侧栏等玩家点（共享资源评估第二节第 6 条）
        player = {"puuid": lt.REC.me["puuid"]}
        r = ui(WAITING_PAGE % (json.dumps(EXE), json.dumps(player), json.dumps(player),
                               json.dumps(os.path.join(SHOTS, "real-ui-open-waiting-plugin-page.png")),
                               json.dumps(os.path.join(SHOTS, "real-ui-open-waiting-card.png"))))
        first, again = (r.get("first") or {}).get("result") or {}, (r.get("again") or {}).get("result") or {}
        check("ui.open while the player is on the plugin page: the result says waiting (the window answered)",
              (r.get("first") or {}).get("ok") and first.get("status") == "waiting" and first.get("opened") is False,
              json.dumps(r.get("first"), ensure_ascii=False)[:300])
        check("ui.open did not switch pages: still on 插件 → 英雄联盟, not the chat",
              r.get("stillPlugin") is True and r.get("chatVisible") is False,
              f"plugin page {r.get('stillPlugin')}, chat {r.get('chatVisible')}")
        check("one in-window notice names the page (the second open did not add one)",
              sum("英雄联盟 · 战绩" in t for t in (r.get("toasts") or [])) == 1,
              json.dumps(r.get("toasts"), ensure_ascii=False))
        check("opening the same page twice counts once: again waiting, one waiting card in the chat page's Cards",
              again.get("status") == "waiting" and r.get("waiting") == ["ui://kotone/lol/player"]
              and r.get("tab") == "cards", f"again {again.get('status')}, cards {r.get('waiting')}, tab {r.get('tab')}")
        check("the waiting card can be dismissed", r.get("left") == 0, str(r.get("left")))

        # 2. 每一页单独打开（ui.open → 右侧栏；聊天卡片和“这局总结”自动卡片走的是同一个 plugin_app）。
        #    玩家在聊天页（上一步点过去了）：第一页放进空着的右侧栏，后面的是同一个插件，往上叠，都直接显示
        for page, params, want, online in sidebar_pages(lt.REC):
            uri = f"ui://kotone/lol/{page}"
            try:
                r = ui(SHOW_SIDEBAR_PAGE % (json.dumps(uri), json.dumps(EXE), json.dumps(params, ensure_ascii=False),
                                            FRAME_STATE, json.dumps(page), json.dumps(want), APP_CONTENT,
                                            json.dumps(os.path.join(SHOTS, f"real-{page}-app-window.png")),
                                            json.dumps(os.path.join(SHOTS, f"real-{page}-app.png"))))
            except RuntimeError as e:
                check(f"ui.open {uri} shows the page in the right sidebar", False, str(e)[-300:])
                continue
            opened = r.get("opened") or {}
            check(f"ui.open {uri} is accepted and shown right away (chat page, sidebar free or the same plugin)",
                  opened.get("ok") and (opened.get("result") or {}).get("status") == "shown",
                  json.dumps(opened, ensure_ascii=False)[:200])
            print(f"  [{page}] frame:", json.dumps({k: r.get(k) for k in ("phase", "display", "plugin", "failed")},
                                                   ensure_ascii=False))
            print(f"  [{page}] app text:", (r.get("appText") or "").replace("\n", " ")[:240])
            print(f"  [{page}] icons:", json.dumps(r.get("icons"), ensure_ascii=False))
            frame_checks(f"right sidebar {page}", r, page)
            if r.get("count", 0) > 0 or not online:
                check(f"{page} shows its data ({want} × {r.get('count')})", r.get("count", 0) > 0,
                      f"status {r.get('status')}")
            else:
                check(f"{page} shows something (no data: network / Tencent 101?)",
                      bool(r.get("status") or (r.get("appText") or "").strip()), r.get("status") or "")
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
        if server is not None:
            server.shutdown()
            print("fake client hits:", dict(lt.HITS))
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
