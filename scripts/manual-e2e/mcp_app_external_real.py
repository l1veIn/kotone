"""MCP plan phase 5 (2026-11-25, handoff §11 items 15-16): the League of Legends MCP App opened by an external MCP host.

The player: the LoL App is "an independent MCP server + MCP App; Claude, Codex can use it directly". This proves it with
ext-apps' reference host `examples/basic-host` (the same AppBridge + double-iframe sandbox the spec describes; it knows
nothing about Kotone), talking to `kotone.exe mcp`:

  headless Chrome ─ basic-host (localhost:8080, sandbox on localhost:8081)
                      └ Streamable HTTP ─ bridge in this file (127.0.0.1:<free port>/mcp)
                                            └ stdio ─ `kotone.exe mcp` ─ tool service ─ isolated Kotone (--background)
                                                                                         └ fake LoL client (lol_tools_real.py)

- basic-host speaks HTTP and `kotone.exe mcp` is stdio, like Claude Desktop / Codex use it. The bridge here is stdlib only and
  passes every JSON-RPC message through unchanged (POST → one child's stdin, the response with the same id → the HTTP
  reply as application/json; GET → 405, so no server-initiated stream; one child per `initialize`). It refuses the tools
  that type / speak / control the client (lol.client, game_chat.send, voice_chat.say, kotone_call) without forwarding.
- isolated Kotone: KOTONE_HOME=target/mcp-app-external/home, tool service on, speech volume 0, hotkeys / wake word off,
  no AI connection; KOTONE_LOL_LCU / KOTONE_LOL_LIVE point at the fake client (lol_tools_real.py's recording: LCU and
  Live Client Data over HTTPS on 127.0.0.1 with a random token, phase InProgress).
- in basic-host (headless Chrome through apps/desktop's @playwright/test, channel chrome; no OS input): call
  `kotone_lol_tier` {mode: rift, lane: 中} (one public request to Tencent 101, no account) and `kotone_lol_scout` {}
  (the fake client), wait for the App to draw inside basic-host's sandbox, click the card's "open" button (fullscreen).
  Images in another host come from CommunityDragon (declared in the resource's CSP); counted in the page.

Checks: the server description basic-host received carries the LoL risk note (Tencent / Riot, LeagueAkari #523, client
control off by default); both tools link their App (`_meta.ui.resourceUri`); basic-host read the resources
(text/html;profile=mcp-app, CSP declares CommunityDragon); the tier list and the match App drew with data; the tier App
shows the mode the call asked for (basic-host sends no `toolInfo` in the host context, which is optional: the App must
still wait for the pushed result instead of loading its own default); champion icons loaded from CommunityDragon; the card's "open" asked basic-host for fullscreen; no refused tool was called.
Screenshots: target/plugin-shots/round7/basic-host-*.png (the recording has real player names: keep them on this machine).

Prerequisites: Node 20+ with npm (and git, if target/research/ext-apps is missing: shallow clone); network (npm install the
first time, Tencent 101, CommunityDragon); Chrome; `pnpm -C apps/desktop build:apps` then `cargo build -p kotone-tauri`;
a lol_probe.py recording (default target/lol-probe/20261123-210745 + target/lol-probe/gamedata). No uv needed.
basic-host is copied to target/research/basic-host-run (ext-apps pinned at 2.1.0, the version Kotone uses), npm-installed
and built once, then reused (`--fresh` redoes it). Ports 8080 / 8081 must be free (basic-host hard-codes the sandbox at
localhost:8081). Takes target/tmp/real.lock (starts Kotone) and target/tmp/pw.lock (browser, dev ports), waiting up to
30 minutes for each. Stops before starting if a game is running or in front. Sends no OS input, plays no sound.
Cleanup: stops basic-host, the bridge's children, Kotone and the fake client; deletes target/mcp-app-external
(`--keep` keeps it). The basic-host copy and the screenshots stay.
usage: python -u -X utf8 -I scripts/manual-e2e/mcp_app_external_real.py [--exe PATH] [--fresh] [--keep] [--probe DIR]
"""
import http.server
import json
import os
import queue
import shutil
import socket
import subprocess
import sys
import threading
import time
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lol_tools_real as lt  # noqa: E402  (fake client: Recording, Handler, serve, TOKEN)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "mcp-app-external")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
SRC_EXE = (os.path.abspath(sys.argv[sys.argv.index("--exe") + 1]) if "--exe" in sys.argv
           else os.path.join(REPO, "target", "debug", "kotone.exe"))
EXE = os.path.join(BIN, "kotone.exe")
CLONE = os.path.join(REPO, "target", "research", "ext-apps")
HOST_DIR = os.path.join(REPO, "target", "research", "basic-host-run")
SHOTS = os.path.join(REPO, "target", "plugin-shots", "round7")
LOCKS = [os.path.join(REPO, "target", "tmp", "real.lock"), os.path.join(REPO, "target", "tmp", "pw.lock")]
EXT_APPS = "2.1.0"
HOST_PORT, SANDBOX_PORT = 8080, 8081
APP_MIME = "text/html;profile=mcp-app"
CDRAGON = "https://raw.communitydragon.org"
TIER, MATCH = "ui://kotone/lol/tier", "ui://kotone/lol/match"
# 会打字、出声、操作客户端的：桥直接拒绝，不转给 Kotone
FORBIDDEN = {"kotone_lol_client", "kotone_game_chat_send", "kotone_voice_chat_say", "kotone_call"}
RESULTS = []
TRAFFIC = []  # (方向, 消息)：桥上走过的每一条
REFUSED = []
ENV = {}


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


def take_locks():
    """Both locks or none (holding one while waiting for the other could block a script that needs both)."""
    deadline = time.time() + 30 * 60
    while True:
        taken = []
        for lock in LOCKS:
            try:
                os.makedirs(lock)
                taken.append(lock)
            except FileExistsError:
                break
        if len(taken) == len(LOCKS):
            return taken
        release(taken)
        if time.time() > deadline:
            raise SystemExit(f"STOP: {LOCKS} held for 30 minutes (another script is running)")
        print("waiting for", ", ".join(os.path.basename(lock) for lock in LOCKS if os.path.isdir(lock)), "…")
        time.sleep(15)


def release(locks):
    for lock in locks:
        try:
            os.rmdir(lock)
        except OSError:
            pass


def port_free(port):
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", port)) != 0


def run(cmd, cwd, env=None, timeout=900):
    print("  $", " ".join(cmd))
    out = subprocess.run(cmd, cwd=cwd, env=env or os.environ, capture_output=True, text=True, encoding="utf-8",
                         errors="replace", timeout=timeout, stdin=subprocess.DEVNULL)
    if out.returncode != 0:
        raise SystemExit(f"failed: {' '.join(cmd)}\n{(out.stdout + out.stderr)[-2000:]}")
    return out.stdout


# ---------- basic-host ----------

def setup_host(fresh):
    """Copy ext-apps' basic-host out of the monorepo (so npm installs only its own dependencies), pin ext-apps, build."""
    node, npm = shutil.which("node"), shutil.which("npm")
    if not node or not npm:
        raise SystemExit("needs Node 20+ with npm on PATH")
    source = os.path.join(CLONE, "examples", "basic-host")
    if not os.path.isdir(source):
        git = shutil.which("git") or raise_missing("git")
        run([git, "clone", "--depth", "1", "https://github.com/modelcontextprotocol/ext-apps", CLONE],
            cwd=os.path.dirname(CLONE))
    built = all(os.path.exists(os.path.join(HOST_DIR, "dist", f)) for f in ("index.html", "sandbox.html"))
    if built and not fresh:
        print("basic-host: reusing", HOST_DIR)
        return node
    shutil.rmtree(HOST_DIR, ignore_errors=True)
    shutil.copytree(source, HOST_DIR, ignore=shutil.ignore_patterns("node_modules", "dist"))
    manifest = os.path.join(HOST_DIR, "package.json")
    with open(manifest, encoding="utf-8") as f:
        pkg = json.load(f)
    pkg["dependencies"]["@modelcontextprotocol/ext-apps"] = EXT_APPS
    with open(manifest, "w", encoding="utf-8") as f:
        json.dump(pkg, f, indent=2)
    run([npm, "install", "--no-audit", "--no-fund"], cwd=HOST_DIR)
    vite = os.path.join(HOST_DIR, "node_modules", "vite", "bin", "vite.js")
    for page in ("index.html", "sandbox.html"):
        run([node, vite, "build"], cwd=HOST_DIR, env=dict(os.environ, INPUT=page, NODE_ENV="development"))
    return node


def raise_missing(what):
    raise SystemExit(f"needs {what} on PATH")


def start_host(node, bridge_url):
    """basic-host's own serve.ts (Node 22.6+ strips the types itself): host on 8080, sandbox with the CSP header on 8081."""
    env = dict(os.environ, SERVERS=json.dumps([bridge_url]), HOST_PORT=str(HOST_PORT), SANDBOX_PORT=str(SANDBOX_PORT))
    log = open(os.path.join(WORK, "basic-host.log"), "w", encoding="utf-8")
    proc = subprocess.Popen([node, "serve.ts"], cwd=HOST_DIR, env=env, stdout=log, stderr=subprocess.STDOUT,
                            stdin=subprocess.DEVNULL)
    deadline = time.time() + 30
    while time.time() < deadline:
        if not port_free(HOST_PORT) and not port_free(SANDBOX_PORT):
            return proc
        if proc.poll() is not None:
            break
        time.sleep(0.5)
    log.close()
    raise SystemExit("basic-host did not start:\n" + open(os.path.join(WORK, "basic-host.log"), encoding="utf-8").read())


# ---------- stdio ↔ Streamable HTTP ----------

class Child:
    """One `kotone.exe mcp` process = one MCP session (like Claude Desktop starting the server)."""

    def __init__(self):
        self.proc = subprocess.Popen([EXE, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, env=ENV)
        self.waiting = {}
        self.lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            TRAFFIC.append(("<-", msg))
            key = json.dumps(msg.get("id")) if ("result" in msg or "error" in msg) else None
            with self.lock:
                waiter = self.waiting.pop(key, None) if key else None
            if waiter:
                waiter.put(msg)
            # 别的（notifications/*/list_changed）没有 GET 流可推，丢掉
        with self.lock:
            for waiter in self.waiting.values():
                waiter.put(None)
            self.waiting.clear()

    def exchange(self, msg, timeout=90):
        waiter = None
        if "method" in msg and "id" in msg:
            waiter = queue.Queue()
            with self.lock:
                self.waiting[json.dumps(msg["id"])] = waiter
        TRAFFIC.append(("->", msg))
        self.proc.stdin.write((json.dumps(msg, ensure_ascii=False) + "\n").encode("utf-8"))
        self.proc.stdin.flush()
        if waiter is None:
            return None
        reply = waiter.get(timeout=timeout)
        if reply is None:
            return {"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32603, "message": "kotone.exe mcp exited"}}
        return reply

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(10)
        except Exception:  # noqa: BLE001
            self.proc.kill()


SESSIONS = {}
SESSIONLESS = []


class Bridge(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def reply(self, code, body=None, session=None, extra=None):
        data = b"" if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "*")
        self.send_header("Access-Control-Expose-Headers", "Mcp-Session-Id")
        if body is not None:
            self.send_header("Content-Type", "application/json")
        if session:
            self.send_header("Mcp-Session-Id", session)
        for key, value in (extra or {}).items():
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_OPTIONS(self):  # noqa: N802
        self.reply(204)

    def do_GET(self):  # noqa: N802
        self.reply(405, extra={"Allow": "POST, DELETE"})  # 不开服务器推送的流

    def do_DELETE(self):  # noqa: N802
        child = SESSIONS.pop(self.headers.get("Mcp-Session-Id") or "", None)
        if child:
            child.close()
        self.reply(200)

    def do_POST(self):  # noqa: N802
        if self.path.partition("?")[0] != "/mcp":
            return self.reply(404, {"error": "not here"})
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"null")
        messages = body if isinstance(body, list) else [body]
        sid = self.headers.get("Mcp-Session-Id")
        new_sid = None
        if any(isinstance(m, dict) and m.get("method") == "initialize" for m in messages):
            new_sid = uuid.uuid4().hex
            child = SESSIONS[new_sid] = Child()
        elif sid:
            child = SESSIONS.get(sid)
            if child is None:
                return self.reply(404, {"jsonrpc": "2.0", "id": None,
                                        "error": {"code": -32001, "message": "Session not found"}})
        else:
            # 握手前的探测（例如新版客户端的 server/discover）：交给一个不算会话的进程，照 stdio 原样回
            if not SESSIONLESS:
                SESSIONLESS.append(Child())
            child = SESSIONLESS[0]
        replies = []
        for m in messages:
            params = m.get("params") or {}
            if m.get("method") == "tools/call" and params.get("name") in FORBIDDEN:
                REFUSED.append(params.get("name"))
                replies.append({"jsonrpc": "2.0", "id": m.get("id"),
                                "error": {"code": -32603, "message": "refused by the test bridge"}})
                continue
            out = child.exchange(m)
            if out is not None:
                replies.append(out)
        if not replies:
            return self.reply(202, session=new_sid)
        self.reply(200, replies if isinstance(body, list) else replies[0], session=new_sid or sid)

    def log_message(self, *args):
        pass


def start_bridge():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Bridge)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_address[1]}/mcp"


def answers(method):
    """Responses (from Kotone) to the requests with this method that went over the bridge."""
    ids = {json.dumps(m.get("id")): m for d, m in TRAFFIC if d == "->" and m.get("method") == method}
    return [(ids[json.dumps(m.get("id"))], m) for d, m in TRAFFIC
            if d == "<-" and json.dumps(m.get("id")) in ids and ("result" in m or "error" in m)]


# ---------- Kotone + fake client ----------

def cli(*args, timeout=30):
    out = subprocess.run([EXE, *args], capture_output=True, text=True, encoding="utf-8", env=ENV,
                         stdin=subprocess.DEVNULL, timeout=timeout)
    try:
        return out.returncode, json.loads(out.stdout)
    except json.JSONDecodeError:
        return out.returncode, {"raw": out.stdout[-500:]}


def wait_service(timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        code, value = cli("tools", "call", "app.status", "--timeout-ms", "3000")
        if code == 0 and value.get("ok"):
            return value["result"]
        time.sleep(1)
    raise RuntimeError("the isolated instance's tool service did not come up")


def wait_log(text, timeout):
    path = os.path.join(HOME, "kotone.log")
    deadline = time.time() + timeout
    while time.time() < deadline:
        if os.path.exists(path) and text in open(path, encoding="utf-8", errors="replace").read():
            return True
        time.sleep(1)
    return False


# ---------- the browser ----------

DRIVER = r"""
import { chromium } from "@playwright/test";
const cfg = JSON.parse(process.env.DRIVER_CONFIG);
const out = { console: [], cdragon: { ok: 0, failed: 0, samples: [] }, calls: {} };
const browser = await chromium.launch({ channel: "chrome", headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1180, height: 1300 }, colorScheme: "light" });
  page.on("console", (m) => { if (out.console.length < 300) out.console.push(`${m.type()}: ${m.text().slice(0, 300)}`); });
  page.on("response", (r) => {
    if (!r.url().startsWith(cfg.cdragon)) return;
    if (r.ok()) out.cdragon.ok++; else out.cdragon.failed++;
    if (out.cdragon.samples.length < 3) out.cdragon.samples.push(`${r.status()} ${r.url()}`);
  });
  page.on("requestfailed", (r) => {
    if (r.url().startsWith(cfg.cdragon)) { out.cdragon.failed++; out.cdragon.samples.push(`failed ${r.failure()?.errorText} ${r.url()}`); }
  });
  await page.goto(cfg.host + "/index.html?theme=hide");
  const select = page.locator("select");
  await page.waitForFunction(() => (document.querySelectorAll("select")[1]?.options.length ?? 0) > 0, null, { timeout: 60000 });
  out.servers = await select.first().locator("option").allInnerTexts();
  out.tools = await select.nth(1).locator("option").allInnerTexts();
  for (const call of cfg.calls) {
    const r = (out.calls[call.tool] = {});
    if (!out.tools.includes(call.tool)) { r.error = "not in basic-host's tool list"; continue; }
    const before = new Set(page.frames());
    await select.nth(1).selectOption(call.tool);
    await page.locator("textarea").fill(JSON.stringify(call.args));
    await page.getByRole("button", { name: "Call Tool" }).click();
    const panel = page.locator('[class*="toolCallInfoPanel"]').last();
    // App 在 basic-host 的沙盒页（8081）里面那层 iframe（document.write 进 about:blank）
    let app = null;
    const until = Date.now() + call.waitMs;
    while (Date.now() < until) {
      const sandbox = page.frames().find((f) => !before.has(f) && f.url().startsWith(cfg.sandbox));
      app = sandbox?.childFrames()[0] ?? null;
      if (app && (await app.locator(call.ready).count().catch(() => 0)) > 0) break;
      await page.waitForTimeout(500);
    }
    r.sandboxUrl = page.frames().find((f) => !before.has(f) && f.url().startsWith(cfg.sandbox))?.url() ?? null;
    if (!app) { r.error = "no App frame"; await panel.screenshot({ path: call.shot }).catch(() => {}); continue; }
    // 等图片：都加载完（或者失败）为止
    const imgUntil = Date.now() + 20000;
    while (Date.now() < imgUntil) {
      const pending = await app.evaluate(() => [...document.images].filter((i) => !i.complete).length).catch(() => 0);
      if (pending === 0) break;
      await page.waitForTimeout(500);
    }
    await page.waitForTimeout(800);
    const probe = (counts) => app.evaluate(({ counts, cdragon }) => {
      const imgs = [...document.images];
      return {
        href: location.href,
        display: document.documentElement.dataset.display ?? null,
        page: document.querySelector('meta[name="kotone:page"]')?.getAttribute("content") ?? null,
        tierMode: document.querySelector('[data-testid="lol-app-tier"]')?.dataset.mode ?? null,
        text: document.body.innerText.slice(0, 900),
        counts: Object.fromEntries(counts.map((s) => [s, document.querySelectorAll(`[data-testid="${s}"]`).length])),
        images: {
          total: imgs.length,
          loaded: imgs.filter((i) => i.complete && i.naturalWidth > 0).length,
          cdragon: imgs.filter((i) => i.currentSrc.startsWith(cdragon) && i.naturalWidth > 0).length,
          sample: imgs.slice(0, 2).map((i) => i.currentSrc),
          elsewhere: imgs.filter((i) => i.currentSrc && !i.currentSrc.startsWith(cdragon) && !i.currentSrc.startsWith("data:")).map((i) => i.currentSrc).slice(0, 3),
        },
      };
    }, { counts, cdragon: cfg.cdragon });
    Object.assign(r, await probe(call.counts));
    r.iframeHeight = await panel.locator("iframe").first().evaluate((e) => Math.round(e.getBoundingClientRect().height));
    await panel.screenshot({ path: call.shot });
    if (call.fullShot) {
      // 卡片右上角的“打开”：App 请宿主换成 fullscreen（basic-host 支持 inline / fullscreen）
      const open = app.getByTestId("lol-card-open");
      if ((await open.count()) > 0) {
        await open.first().click();
        const fsUntil = Date.now() + 10000;
        while (Date.now() < fsUntil && (await app.evaluate(() => document.documentElement.dataset.display)) !== "fullscreen")
          await page.waitForTimeout(250);
        await page.waitForTimeout(1500);
        r.fullscreen = await probe(call.fullCounts);
        await page.screenshot({ path: call.fullShot });
      } else r.fullscreen = { error: "no lol-card-open button" };
    }
  }
  await page.screenshot({ path: cfg.pageShot, fullPage: true }).catch(() => {});
} finally {
  console.log(JSON.stringify(out));
  await browser.close();
}
"""


def drive(calls, page_shot):
    config = {"host": f"http://localhost:{HOST_PORT}", "sandbox": f"http://localhost:{SANDBOX_PORT}/sandbox.html",
              "cdragon": CDRAGON, "calls": calls, "pageShot": page_shot}
    proc = subprocess.run(["node", "--input-type=module", "-e", DRIVER], capture_output=True, text=True,
                          encoding="utf-8", errors="replace", timeout=600,
                          cwd=os.path.join(REPO, "apps", "desktop"), env=dict(os.environ, DRIVER_CONFIG=json.dumps(config)))
    lines = [line for line in proc.stdout.strip().splitlines() if line.startswith("{")]
    if proc.returncode != 0 or not lines:
        raise RuntimeError((proc.stdout + proc.stderr)[-3000:])
    return json.loads(lines[-1])


# ---------- checks ----------

def session(enemies):
    os.makedirs(SHOTS, exist_ok=True)
    shot = lambda name: os.path.join(SHOTS, f"basic-host-{name}.png")  # noqa: E731
    calls = [
        {"tool": "kotone_lol_tier", "args": {"mode": "rift", "lane": "中"}, "waitMs": 90000,
         "ready": '[data-testid="lol-app-tier-row"], [data-testid="lol-card-row"], [data-testid="lol-app-status-banner"]',
         "counts": ["lol-app-tier", "lol-app-tier-row", "lol-card-row", "lol-card-open", "lol-app-status-banner"],
         "shot": shot("tier")},
        {"tool": "kotone_lol_scout", "args": {}, "waitMs": 60000,
         "ready": '[data-testid="lol-match-player"], [data-testid="lol-card-row"], [data-testid="lol-match-card"]',
         "counts": ["lol-app-match", "lol-match-card", "lol-card-row", "lol-match-player", "lol-card-open"],
         "shot": shot("scout"),
         "fullShot": shot("scout-fullscreen"),
         "fullCounts": ["lol-app-match", "lol-match-allies", "lol-match-enemies", "lol-match-player"]},
    ]
    r = drive(calls, shot("page"))
    tier, scout = r["calls"].get("kotone_lol_tier", {}), r["calls"].get("kotone_lol_scout", {})
    for name, c in (("tier", tier), ("scout", scout)):
        print(f"  [{name}] page={c.get('page')} mode={c.get('tierMode')} display={c.get('display')} "
              f"iframe={c.get('iframeHeight')}px "
              f"counts={json.dumps(c.get('counts'))} images={json.dumps(c.get('images'), ensure_ascii=False)}")
        print(f"  [{name}] text:", (c.get("text") or c.get("error") or "").replace("\n", " ")[:300])
    if scout.get("fullscreen"):
        f = scout["fullscreen"]
        print(f"  [scout fullscreen] display={f.get('display')} counts={json.dumps(f.get('counts'))} "
              f"images={json.dumps(f.get('images'), ensure_ascii=False)}")
    print("  CommunityDragon responses:", json.dumps(r["cdragon"], ensure_ascii=False))
    noisy = [line for line in r["console"] if "error" in line.lower() or "content security policy" in line.lower()]
    for line in noisy[:12]:
        print("  console:", line)

    # 服务器说明（basic-host 的 initialize 拿到的）
    init = answers("initialize")
    result = init[-1][1].get("result", {}) if init else {}
    instructions = result.get("instructions", "")
    check("basic-host connected to `kotone.exe mcp` over the bridge (Streamable HTTP ↔ stdio)",
          r.get("servers") == ["kotone"] and result.get("serverInfo", {}).get("name") == "kotone",
          f"servers {r.get('servers')}, protocol {result.get('protocolVersion')}")
    check("the server description carries the LoL risk note (Tencent / Riot, LeagueAkari #523, client control off)",
          all(k in instructions for k in ("腾讯", "拳头", "issues/523", "kotone_lol_client", "默认关")),
          instructions[instructions.find("英雄联盟的工具"):][:120] if "英雄联盟的工具" in instructions
          else "old exe? " + instructions[:80])

    # 工具和资源
    listed = {t["name"]: t for _, m in answers("tools/list") for t in m.get("result", {}).get("tools", [])}
    for tool, uri in (("kotone_lol_tier", TIER), ("kotone_lol_scout", MATCH)):
        meta = listed.get(tool, {}).get("_meta", {})
        check(f"{tool} links its App ({uri}) and basic-host lists it",
              meta.get("ui", {}).get("resourceUri") == uri and tool in (r.get("tools") or []),
              json.dumps(meta, ensure_ascii=False))
    reads = {req["params"]["uri"]: resp for req, resp in answers("resources/read")}
    listing = {res["uri"]: res for _, m in answers("resources/list") for res in m.get("result", {}).get("resources", [])}
    for uri in (TIER, MATCH):
        content = ((reads.get(uri) or {}).get("result", {}).get("contents") or [{}])[0]
        csp = ((content.get("_meta") or listing.get(uri, {}).get("_meta") or {}).get("ui") or {}).get("csp") or {}
        check(f"basic-host read {uri}: one HTML document, {APP_MIME}, CSP declares CommunityDragon",
              content.get("mimeType") == APP_MIME and "<html" in content.get("text", "").lower()
              and CDRAGON in csp.get("resourceDomains", []),
              f"{content.get('mimeType')}, {len(content.get('text', ''))} chars, csp {csp}")
        if "build:apps" in content.get("text", ""):
            print(f"  NOTE: {uri} is the placeholder page: run `pnpm -C apps/desktop build:apps` before cargo build")

    # 强度榜
    rows = (tier.get("counts") or {}).get("lol-app-tier-row", 0) + (tier.get("counts") or {}).get("lol-card-row", 0)
    banner = (tier.get("counts") or {}).get("lol-app-status-banner", 0)
    if rows == 0 and banner:
        print("SKIP the tier list rows: the App says", (tier.get("text") or "")[:160].replace("\n", " "),
              "(network / Tencent 101 down?)")
    else:
        check("kotone_lol_tier: the tier list App drew inside basic-host's sandbox, with rows",
              tier.get("page") == "tier" and rows > 0 and (tier.get("sandboxUrl") or "").startswith(
                  f"http://localhost:{SANDBOX_PORT}/sandbox.html"),
              f"page {tier.get('page')}, {rows} rows, display {tier.get('display')}")
        # basic-host 不在宿主上下文里带 toolInfo（规范里是可选的）：App 要照样等宿主推来的结果，不能自己用缺省参数
        # 再调一次、把模型要的（峡谷 · 中路）盖成别的
        check("tier list: the App shows what the call asked for (mode rift), not its own default",
              tier.get("tierMode") == "rift", f"data-mode {tier.get('tierMode')}")
        images = tier.get("images") or {}
        check("tier list: champion icons came from CommunityDragon (no Kotone proxy in this host)",
              images.get("cdragon", 0) > 0 and not images.get("elsewhere"),
              f"{images.get('cdragon')} of {images.get('total')} from CommunityDragon; elsewhere {images.get('elsewhere')}")

    # 这局
    counts = scout.get("counts") or {}
    text = scout.get("text") or ""
    named = [n for n in enemies if n and n in text]
    check("kotone_lol_scout: the match App drew the fake client's game inside basic-host (inline card)",
          scout.get("page") == "match" and (counts.get("lol-match-card", 0) or counts.get("lol-match-player", 0)
                                            or counts.get("lol-card-row", 0)),
          f"counts {counts}; enemy champions named {named}")
    images = scout.get("images") or {}
    check("match: icons (champions, profile icons, spells …) came from CommunityDragon",
          images.get("cdragon", 0) >= 5 and not images.get("elsewhere"),
          f"{images.get('cdragon')} of {images.get('total')} from CommunityDragon; elsewhere {images.get('elsewhere')}")
    full = scout.get("fullscreen") or {}
    fcounts = full.get("counts") or {}
    check("match: the card's “open” asked basic-host for fullscreen and the full page drew 10 players",
          full.get("display") == "fullscreen" and fcounts.get("lol-match-player", 0) >= 10,
          f"display {full.get('display')}, counts {fcounts}, {full.get('error') or ''}")

    check("no refused tool reached the bridge (lol.client, game_chat.send, voice_chat.say, kotone_call)",
          not REFUSED, str(REFUSED))
    called = [f"{m['params'].get('name')} {json.dumps(m['params'].get('arguments') or {}, ensure_ascii=False)}"
              for d, m in TRAFFIC if d == "->" and m.get("method") == "tools/call"]
    print("  tools/call over the bridge (basic-host's own, then the App's):", called)
    print("  screenshots:", ", ".join(os.path.basename(c["shot"]) for c in calls), "+ basic-host-scout-fullscreen.png,"
          " basic-host-page.png in", SHOTS)


def main():
    global ENV
    guard()
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"missing {SRC_EXE}: build it first (see the docstring)")
    for need in (os.path.join(lt.PROBE, "lcu-gameflow-session.json"), os.path.join(lt.GAMEDATA, "champion-summary.json")):
        if not os.path.exists(need):
            raise SystemExit(f"missing recording: {need} (record one with lol_probe.py)")
    node = setup_host("--fresh" in sys.argv)
    locks = take_locks()
    procs, fake, bridge = [], None, None
    try:
        for port in (HOST_PORT, SANDBOX_PORT):
            if not port_free(port):
                raise SystemExit(f"STOP: port {port} is in use (basic-host needs 8080 and 8081)")
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        shutil.copy2(SRC_EXE, EXE)
        print(f"app: {SRC_EXE}")
        # 假客户端（lol_tools_real.py 的录像：LCU + 局内接口，HTTPS，随机令牌）
        lt.bt.WORK = WORK
        lt.REC = lt.Recording(lt.PROBE, lt.GAMEDATA)
        enemies, _ = lt.REC.champion_names()
        fake, port = lt.serve()
        ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
                   KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
        ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
        ENV.pop("KOTONE_EXE", None)
        with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
            json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                       "toolService": {"enabled": True}, "hotkey": {"key": "F24", "mode": "toggle"},
                       "runtimeToggleHotkey": "", "automationStopHotkey": "",
                       "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                     "wake": {"enabled": False}}}, f)
        procs.append(subprocess.Popen([EXE, "--background"], env=ENV))
        status = wait_service()
        print("isolated instance up:", status.get("appVersion"), "pid", status.get("pid"))
        fetched = wait_log("个人的战绩查完了", 60)
        print("fake client:", "every history fetched" if fetched else "histories not fetched yet (scout may be partial)")
        bridge, url = start_bridge()
        print("bridge:", url)
        procs.append(start_host(node, url))
        print(f"basic-host: http://localhost:{HOST_PORT} (sandbox :{SANDBOX_PORT})")
        session(enemies)
    finally:
        if bridge:
            bridge.shutdown()
        for child in list(SESSIONS.values()) + SESSIONLESS:
            child.close()
        if len(procs) > 1:
            procs[1].terminate()
            procs[1].wait(10)
        if procs:
            cli("tools", "call", "app.quit")
            try:
                procs[0].wait(15)
            except subprocess.TimeoutExpired:
                procs[0].terminate()
                procs[0].wait(10)
        if fake:
            fake.shutdown()
        release(locks)
        if "--keep" not in sys.argv:
            for _ in range(10):
                shutil.rmtree(WORK, ignore_errors=True)
                if not os.path.exists(WORK):
                    break
                time.sleep(1)
            print("cleaned up", WORK if not os.path.exists(WORK) else f"(could not delete {WORK})")
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; FAILED: {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
