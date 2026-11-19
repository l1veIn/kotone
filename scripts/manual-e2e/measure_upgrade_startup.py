"""
Measure first settings render and idle Windows working sets for two isolated release builds.

The 0.1.13 input must be built from the tag with the isolation shim. The script
seeds a separate old-format config and WebView2 profile for every run. It must
never be pointed at an installed Kotone package or run an installer. Requires
the workspace's Node.js and Python psutil. No microphone input or hotkeys.
"""

import argparse
import hashlib
import json
import mmap
import os
import socket
import statistics
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import psutil


MIB = 1024 * 1024
CONFIG = {
    "ui": {"firstRunCompleted": True, "autoStart": False, "updatePromptDisabled": True},
    "adminPromptDismissed": True,
    "autoAdminPromptDismissed": True,
    "runAsAdminOnStart": False,
    "hotkey": {"key": "", "mode": "hold"},
    "runtimeToggleHotkey": "",
    "channelCycleHotkey": "",
    "resendLastHotkey": "",
    "togglePostProcessingHotkey": "",
    "cyclePostProcessingHotkey": "",
    "automationStopHotkey": "",
    "assistant": {"hotkey": "", "wake": {"enabled": False}},
    "connections": [],
    "lowerSpeakerVolumeWhileListening": False,
}

# The Node process only accesses the report's loopback debugger. The selector
# is rendered after Kotone has queried the backend for the actual settings.
PROBE = r"""
const [port, expected] = process.argv.slice(1);
const origin = "http://127.0.0.1:" + port;
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const deadline = Date.now() + 30000;
const abort = setTimeout(() => { console.error("settings readiness timed out"); process.exit(1); }, 32000);

async function findSettings() {
  while (Date.now() < deadline) {
    try {
      const response = await fetch(origin + "/json/list", { signal: AbortSignal.timeout(800) });
      const targets = await response.json();
      const target = targets.find((target) => {
        if (target.type !== "page") return false;
        try {
          const url = new URL(target.url);
          const inAppOrigin = url.hostname === "tauri.localhost"
            || (url.protocol === "tauri:" && url.hostname === "localhost");
          return inAppOrigin && url.hash.startsWith("#/settings");
        } catch { return false; }
      });
      if (target) return target.webSocketDebuggerUrl;
    } catch {}
    await pause(40);
  }
  throw new Error("settings WebView target missing");
}

async function run() {
  const url = new URL(await findSettings());
  if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
    throw new Error("settings debugger is not local");
  }
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  let nextId = 0;
  function evaluate(expression) {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      function onMessage(event) {
        const message = JSON.parse(event.data);
        if (message.id !== id) return;
        socket.removeEventListener("message", onMessage);
        if (message.error || message.result?.exceptionDetails) {
          reject(new Error("settings DOM query failed"));
        } else {
          resolve(message.result?.result?.value);
        }
      }
      socket.addEventListener("message", onMessage);
      socket.send(JSON.stringify({
        id, method: "Runtime.evaluate", params: { expression, returnByValue: true },
      }));
    });
  }

  const firstPage = '(' + (() => {
    if (document.readyState !== "complete") return null;
    const notice = document.querySelector('[data-testid="legacy-upgrade-notice"]');
    if (notice?.getClientRects().length) return "upgradeNotice";
    const input = document.getElementById("hotkey-input");
    return input?.getClientRects().length ? "voiceSettings" : null;
  }).toString() + ')()';

  let signal;
  while ((signal = await evaluate(firstPage)) !== expected) await pause(40);
  console.log(JSON.stringify({ screen: signal }));
  if (signal === "upgradeNotice") {
    const dismissed = await evaluate('(' + (() => {
      const notice = document.querySelector('[data-testid="legacy-upgrade-notice"]');
      const button = [...(notice?.querySelectorAll("button") ?? [])].find(
        (button) => button.textContent.trim() === "继续只用语音输入法",
      );
      if (!button || button.disabled) return false;
      button.click();
      return true;
    }).toString() + ')()');
    if (!dismissed) throw new Error("cannot select the voice-only default in the test window");
    while (await evaluate(firstPage) !== "voiceSettings") await pause(40);
  }
  socket.close();
}

run().then(() => clearTimeout(abort)).catch((error) => {
  console.error(error.message ?? String(error));
  process.exit(1);
});
"""


class RejectProxy(BaseHTTPRequestHandler):
    """Block updater requests from the old binary without changing its source."""

    def reject(self):
        self.server.attempts += 1
        self.send_error(503)

    do_CONNECT = reject
    do_GET = reject
    do_POST = reject

    def log_message(self, *_):
        pass


def check_binary(path: Path, old: bool) -> Path:
    path = path.resolve(strict=True)
    if path.name.lower() != "kotone.exe" or "target" not in [part.lower() for part in path.parts]:
        raise ValueError(f"A locally built kotone.exe under target is required: {path}")
    with path.open("rb") as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as contents:
        if contents.find(b"KOTONE_HOME") < 0:
            raise ValueError(f"The exe lacks isolated KOTONE_HOME support: {path}")
        marker = "隔离实例不能更改系统自启动设置".encode("utf-8") if old else b"legacyUpgrade"
        if contents.find(marker) < 0:
            raise ValueError(f"The exe lacks the expected transition or isolation shim: {path}")
    return path


def local_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def working_sets(root: psutil.Process) -> dict:
    app = root.memory_info().rss
    children = []
    for child in root.children(recursive=True):
        try:
            children.append(child.memory_info().rss)
        except psutil.NoSuchProcess:
            pass
    return {"mainMiB": app / MIB, "treeMiB": (app + sum(children)) / MIB}


def close_app(app: subprocess.Popen) -> None:
    try:
        owned = psutil.Process(app.pid).children(recursive=True)
    except psutil.NoSuchProcess:
        owned = []
    if app.poll() is None:
        app.terminate()
        app.wait(timeout=5)
    _, survivors = psutil.wait_procs(owned, timeout=2)
    for child in survivors:
        try:
            child.terminate()
        except psutil.NoSuchProcess:
            pass


def measure(binary: Path, directory: Path, proxy: ThreadingHTTPServer, expected: str) -> dict:
    home = directory / "home"
    home.mkdir(parents=True)
    (home / "config.json").write_text(json.dumps(CONFIG), encoding="utf-8")
    port = local_port()
    reject = f"http://127.0.0.1:{proxy.server_port}"
    env = dict(
        os.environ,
        KOTONE_HOME=str(home),
        WEBVIEW2_USER_DATA_FOLDER=str(directory / "webview"),
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=(
            f"--remote-debugging-port={port} --disable-background-networking "
            f"--proxy-server={reject} --proxy-bypass-list=localhost;127.0.0.1;tauri.localhost"
        ),
        HTTP_PROXY=reject,
        HTTPS_PROXY=reject,
        ALL_PROXY=reject,
        NO_PROXY="localhost,127.0.0.1,tauri.localhost",
    )
    before_attempts = proxy.attempts
    start = time.perf_counter()
    app = subprocess.Popen([str(binary)], cwd=directory, env=env)
    try:
        with subprocess.Popen(
            ["node", "--input-type=module", "-e", PROBE, str(port), expected],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            creationflags=subprocess.CREATE_NO_WINDOW,
        ) as probe:
            assert probe.stdout is not None
            first = probe.stdout.readline()
            ready_ms = round((time.perf_counter() - start) * 1000)
            _, error = probe.communicate(timeout=33)
        if probe.returncode != 0 or not first:
            raise RuntimeError(f"Could not open the isolated settings window: {error.strip()} ({directory})")
        signal = json.loads(first)["screen"]
        time.sleep(8)
        process = psutil.Process(app.pid)
        samples = []
        for _ in range(5):
            samples.append(working_sets(process))
            time.sleep(0.5)
        return {
            "firstScreen": signal,
            "startupMs": ready_ms,
            "mainMiB": round(statistics.median(sample["mainMiB"] for sample in samples), 2),
            "treeMiB": round(statistics.median(sample["treeMiB"] for sample in samples), 2),
            "proxyRequestsRejected": proxy.attempts - before_attempts,
            "evidenceDirectory": str(directory),
        }
    finally:
        close_app(app)


def item(binary: Path, installer: Path | None) -> dict:
    with binary.open("rb") as source:
        checksum = hashlib.file_digest(source, "sha256").hexdigest()
    return {
        "exe": str(binary),
        "exeBytes": binary.stat().st_size,
        "exeSha256": checksum,
        "installerBytes": installer.resolve(strict=True).stat().st_size if installer else None,
        "runs": [],
    }


def main() -> None:
    if os.name != "nt":
        raise SystemExit("This measurement requires Windows and WebView2.")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline-installer", type=Path)
    parser.add_argument("--candidate-installer", type=Path)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--target", type=Path, default=Path(__file__).resolve().parents[2] / "target")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    try:
        old = check_binary(args.baseline, old=True)
        new = check_binary(args.candidate, old=False)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    target = args.target.resolve()
    if "target" not in [part.lower() for part in target.parts]:
        parser.error("--target must be under an isolated target directory")
    target.mkdir(parents=True, exist_ok=True)
    report_dir = Path(tempfile.mkdtemp(prefix="upgrade-bench-", dir=target))
    result = {
        "baseline": item(old, args.baseline_installer),
        "candidate": item(new, args.candidate_installer),
    }
    with ThreadingHTTPServer(("127.0.0.1", 0), RejectProxy) as proxy:
        proxy.attempts = 0
        worker = threading.Thread(target=proxy.serve_forever, daemon=True)
        worker.start()
        try:
            pairs = [("baseline", old, "voiceSettings"), ("candidate", new, "upgradeNotice")]
            for run in range(args.runs):
                for label, binary, screen in (pairs if run % 2 == 0 else reversed(pairs)):
                    sample = measure(binary, report_dir / f"{label}-{run + 1}", proxy, screen)
                    result[label]["runs"].append(sample)
                    print(label, run + 1, json.dumps(sample, ensure_ascii=False), flush=True)
        finally:
            proxy.shutdown()
            worker.join()
    for version in result.values():
        version["medians"] = {
            key: statistics.median(run[key] for run in version["runs"])
            for key in ("startupMs", "mainMiB", "treeMiB")
        }
    path = report_dir / "results.json"
    path.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(path)


if __name__ == "__main__":
    main()
