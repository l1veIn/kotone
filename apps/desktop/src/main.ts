import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { installDesktopGuards } from "./lib/desktopGuards";

installDesktopGuards();

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
