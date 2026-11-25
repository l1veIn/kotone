import { expect, test } from "@playwright/test";
import { screenshot } from "./screenshot.cjs";

// 外观（设置 → 外观，ADR 023 从“通用”搬来）：只作用于设置窗口，偏好存在 localStorage，读坏了回退默认。

test("appearance defaults to Kotone dark and leaves old pages untouched", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-appearance").click();
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-palette", "kotone");
  await expect(html).toHaveAttribute("data-theme", "dark");
  const panel = page.getByTestId("appearance-settings");
  await expect(panel.getByTestId("appearance-mode-dark")).toHaveAttribute("aria-pressed", "true");
  await expect(panel.getByTestId("appearance-palette-kotone")).toHaveAttribute("aria-pressed", "true");
  // 默认外观：内容区仍是 Kotone 的蓝黑底，卡片仍是 Kotone 的面板色
  await expect(page.getByRole("main")).toHaveCSS("background-color", "rgb(26, 26, 46)");
  const card = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--card").trim());
  expect(card).toBe("#212140");
});

test("appearance switches theme, palette, zoom and font, and persists across reloads", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-appearance").click();
  const panel = page.getByTestId("appearance-settings");
  await panel.getByTestId("appearance-mode-light").click();
  await panel.getByTestId("appearance-palette-green").click();
  await panel.getByTestId("appearance-zoom").click();
  await page.getByRole("option", { name: "110%" }).click();
  // 缩放会让整页重新排版：等它生效、下拉收起后再开字体
  await expect(page.locator("body")).toHaveCSS("zoom", "1.1");
  await expect(page.getByRole("option", { name: "110%" })).toHaveCount(0);
  await panel.getByTestId("appearance-font").click();
  await page.getByRole("option", { name: "等线" }).click();

  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "light");
  await expect(html).toHaveAttribute("data-palette", "green");
  await expect(page.locator("body")).toHaveCSS("zoom", "1.1");
  expect(await page.evaluate(() => getComputedStyle(document.body).fontFamily)).toContain("DengXian");
  await screenshot(panel, { path: testInfo.outputPath("appearance-light-green.png") });

  await page.reload();
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-appearance").click();
  await expect(html).toHaveAttribute("data-theme", "light");
  await expect(html).toHaveAttribute("data-palette", "green");
  expect(JSON.parse(await page.evaluate(() => localStorage.getItem("kotone.appearance.v1") ?? "null")))
    .toMatchObject({ mode: "light", palette: "green", zoom: 110, uiFont: "dengxian" });

  await page.getByTestId("appearance-reset").click();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(html).toHaveAttribute("data-palette", "kotone");
  await expect(page.locator("body")).toHaveCSS("zoom", "1");
});

test("broken or unavailable appearance storage falls back to the default look", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-appearance").click();
  await page.evaluate(() => localStorage.setItem("kotone.appearance.v1", "{not json"));
  await page.reload();
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-appearance").click();
  await expect(page.locator("html")).toHaveAttribute("data-palette", "kotone");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  await page.evaluate(() => localStorage.setItem("kotone.appearance.v1", JSON.stringify({ palette: "nope", mode: "light", zoom: 9999, uiFont: "custom", uiCustomFont: "x;}body{color:red" })));
  await page.reload();
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-appearance").click();
  await expect(page.locator("html")).toHaveAttribute("data-palette", "kotone");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("body")).toHaveCSS("zoom", "1.5");
  expect(await page.evaluate(() => getComputedStyle(document.body).fontFamily)).not.toContain("red");

  // 写入失败（例如存储被禁用）时，本次仍然生效并提示。
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw new Error("存储不可用"); }; });
  await page.getByTestId("appearance-mode-dark").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.getByTestId("appearance-settings").getByRole("alert")).toContainText("存储不可用");
});

test("overlay window never picks up the settings appearance", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.evaluate(() => localStorage.setItem("kotone.appearance.v1", JSON.stringify({ mode: "light", palette: "red", zoom: 125 })));
  await page.goto("/#/overlay");
  await page.reload();
  const html = page.locator("html");
  await expect(html).not.toHaveAttribute("data-theme", /.*/);
  await expect(page.locator("body")).toHaveCSS("zoom", "1");
});
