"""Probe a COPY of the player's BetterGI (ADR 047 phase 3, the genshin plugin): what Kotone can do without touching the game.

Safety rules (from the player, 2026-11-28):
- the real install (default D:\\BetterGI) is READ-ONLY: never written, never started;
- anything that runs is a copy under target/bgi-copy/ (big / private folders not copied: WebView2Data, Cache, log, User/Cache,
  User/Pulonia — the last one so no saved plan or schedule can fire in the copy);
- only things that don't touch the game: start / end the copy, /api/v1/health, the log WebSocket, single-instance forwarding
  with a harmless argument (none: a second plain start). NEVER `startOneDragon`, `--startGroups`, `--pulonia-run`, `start`;
- only the pid this script started is ended.

Pre-flight, in order (any failure = read-only checks only, nothing copied or started):
1. the player's BetterGI is not running (the copy would forward to it — single instance per user) and Genshin is not running;
2. the exe's manifest (`requestedExecutionLevel`): `requireAdministrator` / `highestAvailable` = don't start (nobody clicks UAC);
3. BetterGI 0.67 restarts itself elevated at startup (`RuntimeHelper.EnsureElevated` -> ShellExecute "runas", source
   Helpers/RuntimeHelper.cs). From a non-elevated script that means a UAC prompt, or — with ConsentPromptBehaviorAdmin = 0 — a
   silent elevation into a process this script cannot end. So the copy is only started when THIS script is elevated (then the
   copy inherits elevation, no runas, and the pid we get is the real one). Otherwise: don't start, say why.

usage: python -X utf8 -I scripts/manual-e2e/bgi_copy_probe.py [--source D:\\BetterGI] [--port 30748]
Results go to stdout; the copy's config and log stay under target/bgi-copy/ (never commit them).
"""
import argparse
import base64
import ctypes
import json
import os
import re
import secrets
import shutil
import socket
import subprocess
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
COPY = os.path.join(REPO, "target", "bgi-copy", "BetterGI")
SKIP = {"webview2data", "cache", "log"}
SKIP_USER = {"cache", "pulonia"}
GAME_EXES = ["yuanshen.exe", "genshinimpact.exe"]
FORBIDDEN = ("startonedragon", "--startgroups", "--pulonia-run", "start")
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def info(text):
    print("INFO " + text)


def running(exe):
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH", "/FI", f"IMAGENAME eq {exe}"],
                         capture_output=True, text=True, errors="replace").stdout
    return [int(row.split('","')[1]) for row in out.splitlines() if row.lower().startswith(f'"{exe.lower()}"')]


def manifest_level(exe):
    """requestedExecutionLevel from the exe bytes (the RT_MANIFEST resource is plain XML inside the PE)."""
    levels = set()
    pattern = re.compile(rb'requestedExecutionLevel\s+level="([A-Za-z]+)"')
    with open(exe, "rb") as f:
        tail = b""
        while True:
            chunk = f.read(8 * 1024 * 1024)
            if not chunk:
                break
            data = tail + chunk
            levels.update(m.group(1).decode() for m in pattern.finditer(data))
            tail = data[-200:]
    return levels


def elevated():
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:  # noqa: BLE001
        return False


def consent_behavior():
    try:
        import winreg
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System") as k:
            return winreg.QueryValueEx(k, "ConsentPromptBehaviorAdmin")[0]
    except OSError:
        return None


def file_version(exe):
    out = subprocess.run(["powershell", "-NoProfile", "-Command", f"(Get-Item -LiteralPath '{exe}').VersionInfo.FileVersion"],
                         capture_output=True, text=True, errors="replace").stdout.strip()
    return out or None


def copy_install(source):
    if os.path.exists(COPY):
        shutil.rmtree(COPY)
    def ignore(directory, names):
        rel = os.path.relpath(directory, source).replace("\\", "/").lower()
        if rel == ".":
            return [n for n in names if n.lower() in SKIP]
        if rel == "user":
            return [n for n in names if n.lower() in SKIP_USER]
        return []
    shutil.copytree(source, COPY, ignore=ignore)


def sanitize_copy_config(port, token):
    """In the COPY only: nothing starts by itself; the local external access service is on (another port)."""
    path = os.path.join(COPY, "User", "config.json")
    with open(path, encoding="utf-8-sig") as f:
        config = json.load(f)
    start = config.setdefault("genshinStartConfig", {})
    start["linkedStartEnabled"] = False      # 联动启动原神
    start["autoEnterGameEnabled"] = False    # 自动进游戏
    script = config.setdefault("scriptConfig", {})
    script["autoUpdateBeforeCommandLineRun"] = False
    script["autoUpdateSubscribedScripts"] = False
    config["nextScheduledTask"] = []
    config["externalAccessConfig"] = {"httpApiEnabled": True, "webSocketEnabled": True, "mcpEnabled": False,
                                      "port": port, "accessToken": token}
    notification = config.setdefault("notificationConfig", {})
    notification["webhookEnabled"] = False
    with open(path, "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False, indent=2)


def http_get(port, path, token):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", headers={"Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=3) as r:
        return r.status, json.loads(r.read().decode("utf-8"))


def ws_first_message(port, path, timeout=10):
    """Minimal WebSocket client (RFC 6455): handshake, then read one text frame (server frames are unmasked)."""
    key = base64.b64encode(secrets.token_bytes(16)).decode()
    s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    s.sendall((f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
               f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
    head = b""
    while b"\r\n\r\n" not in head:
        head += s.recv(1)
    if b" 101 " not in head.split(b"\r\n")[0]:
        s.close()
        return None
    def read(n):
        buf = b""
        while len(buf) < n:
            buf += s.recv(n - len(buf))
        return buf
    b1, b2 = read(2)
    length = b2 & 0x7F
    if length == 126:
        length = int.from_bytes(read(2), "big")
    elif length == 127:
        length = int.from_bytes(read(8), "big")
    payload = read(length)
    s.close()
    return payload.decode("utf-8", errors="replace") if (b1 & 0x0F) == 1 else None


def start_copy(args):
    lowered = [a.lower() for a in args]
    assert not any(a in FORBIDDEN or a.startswith("--pulonia") for a in lowered), f"forbidden argument: {args}"
    exe = os.path.join(COPY, "BetterGI.exe")
    return subprocess.Popen([exe, *args], cwd=COPY, creationflags=0x00000008)  # DETACHED_PROCESS


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", default=r"D:\BetterGI")
    parser.add_argument("--port", type=int, default=30748)
    a = parser.parse_args()
    source = os.path.realpath(a.source)
    exe = os.path.join(source, "BetterGI.exe")
    if not check("the real install exists (read-only)", os.path.isfile(exe), exe):
        return False
    version = file_version(exe)
    info(f"BetterGI version: {version}")
    level = manifest_level(exe)
    info(f"manifest requestedExecutionLevel: {sorted(level) or 'none'}")
    consent = consent_behavior()
    info(f"this script elevated: {elevated()}; ConsentPromptBehaviorAdmin: {consent} "
         f"({'elevates admins without asking' if consent == 0 else 'asks' if consent is not None else 'unknown'})")

    # 1. nothing of the player's is running
    player_bgi = running("BetterGI.exe")
    games = [g for g in GAME_EXES if running(g)]
    ok_quiet = check("the player's BetterGI is not running", not player_bgi, str(player_bgi))
    ok_quiet = check("Genshin is not running", not games, str(games)) and ok_quiet
    # 2. manifest
    ok_manifest = check("the manifest doesn't ask for admin", not ({"requireAdministrator", "highestAvailable"} & level),
                        str(sorted(level)))
    # 3. self-elevation at startup (0.67.0 source: RuntimeHelper.EnsureElevated)
    ok_elevation = check("the copy can be started and ended by this script (this script is elevated, so BetterGI won't runas)",
                         elevated(),
                         "BetterGI restarts itself elevated at startup; from here that is a UAC prompt or a silent elevation "
                         "into a process this script can't end — not starting it")
    if not (ok_quiet and ok_manifest and ok_elevation):
        info("read-only checks only: the copy was NOT made or started")
        return summary()

    token = secrets.token_hex(16)
    copy_install(source)
    sanitize_copy_config(a.port, token)
    check("copied to target/bgi-copy (without WebView2Data, Cache, log, User/Cache, User/Pulonia)",
          os.path.isfile(os.path.join(COPY, "BetterGI.exe")))
    first = start_copy([])
    pid = first.pid
    info(f"started the copy, pid {pid}")
    try:
        def health():
            try:
                return http_get(a.port, "/api/v1/health", token)
            except Exception:  # noqa: BLE001
                return None
        got = None
        for _ in range(60):
            got = health()
            if got:
                break
            time.sleep(1)
        check("/api/v1/health answers with the token", got and got[0] == 200, json.dumps(got[1] if got else None))
        try:
            http_get(a.port, "/api/v1/health", "wrong")
            check("a wrong token is refused", False)
        except urllib.error.HTTPError as e:
            check("a wrong token is refused", e.code == 401, str(e.code))
        message = ws_first_message(a.port, "/ws/v1/logs")
        check("/ws/v1/logs sends log events", message and '"type"' in message, (message or "")[:200])
        # single-instance forwarding with a harmless argument: a second plain start forwards and exits
        second = start_copy([])
        try:
            code = second.wait(timeout=60)
        except subprocess.TimeoutExpired:
            code = None
            second.kill()
        check("a second start forwards to the running copy and exits", code is not None and first.poll() is None,
              f"exit code {code}")
    finally:
        # no stop API: end only the pid this script started
        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True)
        time.sleep(2)
        check("ended only our own pid", pid not in running("BetterGI.exe"), f"pid {pid}")
    return summary()


def summary():
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
