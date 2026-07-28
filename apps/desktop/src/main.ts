import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { installDesktopGuards } from "./lib/desktopGuards";
import { logFrontendError } from "./lib/ipc";

installDesktopGuards();

window.addEventListener("error", (event) => {
  void logFrontendError("window", event.message || "unknown window error").catch(() => {});
});
window.addEventListener("unhandledrejection", (event) => {
  const message =
    event.reason instanceof Error ? `${event.reason.name}: ${event.reason.message}` : String(event.reason);
  void logFrontendError("unhandledrejection", message).catch(() => {});
});

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
