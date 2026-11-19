import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { realpath, writeFile } from "node:fs/promises";
import path from "node:path";
import { setTimeout as pause } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const env = process.env;
assert(process.platform === "win32"
  && env.GITHUB_ACTIONS === "true"
  && env.RUNNER_ENVIRONMENT === "github-hosted"
  && env.GITHUB_REPOSITORY?.toLowerCase() === "l1vein/kotone"
  && env.GITHUB_REF === "refs/heads/codex/smoke-0.1.14-isolated"
  && env.GITHUB_EVENT_NAME === "workflow_dispatch", "Expected the isolated GitHub-hosted smoke runner.");
const port = Number(process.argv[2]);
assert(Number.isInteger(port) && port > 1023 && port <= 65535, "Expected a local WebView2 debug port.");
const appPid = Number(process.argv[4]);
assert(Number.isSafeInteger(appPid) && appPid > 0, "Expected the installed test app's process ID.");
const temp = await realpath(env.RUNNER_TEMP);
const target = await realpath(process.argv[3]);
const relativeTarget = path.relative(temp, target);
assert(relativeTarget && !relativeTarget.startsWith("..") && !path.isAbsolute(relativeTarget),
  "Expected a scratch directory under RUNNER_TEMP.");

const rollbackUrl = "https://github.com/l1veIn/kotone/releases/download/v0.1.13/Kotone_0.1.13_x64-setup.exe";

async function until(read, label, duration = 10000) {
  const limit = Date.now() + duration;
  while (Date.now() < limit) {
    const value = await read();
    if (value) return value;
    await pause(100);
  }
  throw new Error(`Timed out waiting for ${label}.`);
}

const loopbacks = ["127.0.0.1", "[::1]"];
const discoveryErrors = new Map();

async function discoverSettings() {
  for (const host of loopbacks) {
    try {
      const response = await fetch(`http://${host}:${port}/json/list`, { signal: AbortSignal.timeout(1000) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const pages = await response.json();
      const page = pages.find((entry) => {
        if (entry.type !== "page") return false;
        const url = new URL(entry.url);
        return (url.origin === "http://tauri.localhost" || (url.protocol === "tauri:" && url.hostname === "localhost"))
          && url.hash.startsWith("#/settings");
      });
      if (page) return page;
      discoveryErrors.set(host, "settings target missing");
    } catch (error) {
      discoveryErrors.set(host, error.cause?.code ?? error.message);
    }
  }
  return null;
}

let pageTarget;
try {
  pageTarget = await until(discoverSettings, "installed Kotone settings", 15000);
} catch (error) {
  // A real runner can render the app without offering a CDP socket. Only
  // retry through UI Automation before any rollback click has been sent.
  const details = [...discoveryErrors].map(([host, message]) => `${host}: ${message}`).join("; ");
  console.log(`${error.message} (${details}) Using the installed app's Windows UI Automation tree.`);
  const fallback = fileURLToPath(new URL("./smoke_014_about_uia.ps1", import.meta.url));
  const result = spawnSync("powershell.exe", [
    "-NoProfile", "-NonInteractive", "-File", fallback,
    "-AppProcessId", String(appPid), "-TargetDirectory", target,
  ], { stdio: "inherit", windowsHide: true, timeout: 75000 });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, "Windows UI Automation did not verify the installed app's rollback link.");
  process.exit(0);
}
const wsUrl = new URL(pageTarget.webSocketDebuggerUrl);
assert(wsUrl.protocol === "ws:" && [...loopbacks, "localhost"].includes(wsUrl.hostname) && wsUrl.port === String(port),
  "Expected the debugger to remain on loopback.");

class DebuggerClient {
  nextId = 0;
  pending = new Map();
  events = new Map();

  constructor(url) {
    this.socket = new WebSocket(url);
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      const listener = message.id ? this.pending.get(message.id) : this.events.get(message.method);
      if (!listener) return;
      if (message.id) this.pending.delete(message.id);
      else this.events.delete(message.method);
      listener(message);
    });
  }

  async open() {
    const socket = this.socket;
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("WebView2 debugger did not open.")), 4000);
      socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
      socket.addEventListener("error", () => {
        clearTimeout(timer);
        reject(new Error("WebView2 debugger connection failed."));
      }, { once: true });
    });
  }

  send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`No response to ${method}.`));
      }, 5000);
      this.pending.set(id, (message) => {
        clearTimeout(timer);
        if (message.error) reject(new Error(`${method}: ${message.error.message}`));
        else resolve(message.result);
      });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  once(method) {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.events.delete(method);
        reject(new Error(`No ${method} event from WebView2.`));
      }, 5000);
      this.events.set(method, (message) => {
        clearTimeout(timer);
        resolve(message.params);
      });
    });
  }

  async evaluate(expression, returnByValue = true) {
    const reply = await this.send("Runtime.evaluate", { expression, returnByValue, awaitPromise: true });
    assert(!reply.exceptionDetails, reply.exceptionDetails?.exception?.description ?? "WebView2 evaluation failed.");
    return returnByValue ? reply.result.value : reply.result;
  }
}

const cdp = new DebuggerClient(wsUrl);
try {
  await cdp.open();
  await until(() => cdp.evaluate(`(() => {
    const item = document.querySelector('[data-testid="settings-nav-about"]');
    if (!item || item.offsetParent === null) return false;
    item.click();
    return true;
  })()`), "About navigation");
  const href = await until(() => cdp.evaluate(`(() => {
    const details = document.querySelector('[data-testid="rollback-section"]');
    if (!details || details.offsetParent === null) return null;
    if (!details.open) details.querySelector('summary').click();
    const anchor = details.querySelector('a');
    return details.open && anchor ? anchor.href : null;
  })()`), "visible rollback details");
  assert.equal(href, rollbackUrl);

  // Both Tauri IPC transports use postMessage. Observe the actual click's
  // success/error callbacks without replacing its payload or native command.
  await cdp.send("Debugger.enable");
  const postMessage = await cdp.evaluate("window.__TAURI_INTERNALS__.postMessage", false);
  assert.equal(postMessage.type, "function");
  const { breakpointId } = await cdp.send("Debugger.setBreakpointOnFunctionCall", {
    objectId: postMessage.objectId,
    condition: `arguments[0]?.cmd === "open_external" && arguments[0]?.payload?.url === ${JSON.stringify(rollbackUrl)}`,
  });
  const paused = cdp.once("Debugger.paused");
  const clicked = cdp.evaluate(`(() => {
    const anchor = document.querySelector('[data-testid="rollback-section"][open] a');
    if (!anchor || anchor.offsetParent === null) return false;
    anchor.click();
    return true;
  })()`);
  // Runtime.evaluate waits for the click handler. Finish observing the paused
  // call before awaiting that command.
  clicked.catch(() => {});
  try {
    const event = await paused;
    assert(event.hitBreakpoints.includes(breakpointId), "The rollback click did not reach the native opener.");
    const observation = await cdp.send("Debugger.evaluateOnCallFrame", {
      callFrameId: event.callFrames[0].callFrameId,
      expression: `(() => {
        const request = arguments[0];
        const callbacks = window.__TAURI_INTERNALS__.callbacks;
        const observe = (id, ok) => {
          const original = callbacks.get(id);
          if (typeof original !== 'function') return false;
          callbacks.set(id, (result) => {
            window.__kotone014Rollback = { ok, command: request.cmd, url: request.payload.url };
            return original(result);
          });
          return true;
        };
        return observe(request.callback, true) && observe(request.error, false);
      })()`,
      returnByValue: true,
    });
    assert.equal(observation.result.value, true, "Could not observe the real native opener response.");
  } finally {
    await cdp.send("Debugger.removeBreakpoint", { breakpointId });
    await cdp.send("Debugger.resume");
  }
  assert.equal(await clicked, true);
  const observed = await until(() => cdp.evaluate("window.__kotone014Rollback ?? null"), "native open_external response");
  assert.deepEqual(observed, { ok: true, command: "open_external", url: rollbackUrl });
  await writeFile(path.join(target, "about-rollback.json"), `${JSON.stringify(observed, null, 2)}\n`, "utf8");
  console.log(`PASS About rollback link invoked the native opener successfully: ${rollbackUrl}`);
} finally {
  cdp.socket.close();
}
