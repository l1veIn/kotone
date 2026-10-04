import { expect, test, type Page } from "@playwright/test";

const message = (page: Page) => page.getByTestId("danmaku-message");

async function fixture(page: Page, patch: Record<string, unknown>) {
  await page.evaluate(async (patch) => {
    const { showDanmakuPreview } = await import("/src/lib/danmaku.ts");
    showDanmakuPreview({
      id: 2, text: "该打小龙了", startedAtMs: Date.now(), expiresAtMs: Date.now() + 60_000,
      speedPxPerSec: 160, fontSize: 32, topOffsetPx: 120, color: "#FFFFFF", emote: "none", ...patch,
    });
  }, patch);
}

async function seek(page: Page, timeMs: number) {
  await expect(message(page)).toHaveAttribute("data-duration-ms", /\d/);
  await message(page).evaluate((element, time) => {
    const animation = element.getAnimations()[0];
    animation.pause();
    animation.currentTime = time;
  }, timeMs);
}

test("danmaku moves right to left at the configured speed without a plate or pointer interception", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("/#/danmaku");
  await seek(page, 1000);
  const first = await message(page).boundingBox();
  await seek(page, 2000);
  const second = await message(page).boundingBox();
  expect(first!.x - second!.x).toBeCloseTo(160, 0);
  expect(second!.height).toBeLessThan(60);
  expect(second!.y).toBeLessThan(1080 * .25);
  await expect(page.locator(".stage")).toHaveCSS("pointer-events", "none");
  await expect(message(page)).toHaveCSS("pointer-events", "none");
  await expect(message(page)).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(message(page)).toHaveCSS("box-shadow", "none");
  await expect(message(page)).toHaveCSS("white-space", "nowrap");
  await expect(page.locator("body")).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(page.getByTestId("settings-nav-general")).toHaveCount(0);

  await fixture(page, { id: 3, emote: "pointing", color: "#FFE082", fontSize: 36 });
  await expect(message(page).locator("img")).toHaveCount(1);
  await expect(message(page)).toHaveCSS("font-size", "36px");
  await expect(message(page)).toHaveCSS("color", "rgb(255, 224, 130)");
  await expect(message(page)).toHaveAttribute("data-duration-ms", /\d/);
  const duration = Number(await message(page).getAttribute("data-duration-ms"));
  await seek(page, duration / 2);
  await message(page).locator("img").evaluate((image: HTMLImageElement) => image.decode());
  await page.screenshot({ path: testInfo.outputPath("danmaku-floating.png"), omitBackground: true });

  await page.evaluate(() => {
    (window as Window & { finishedDanmaku?: number }).finishedDanmaku = undefined;
    window.addEventListener("kotone:preview-danmaku-finished", (event) => {
      (window as Window & { finishedDanmaku?: number }).finishedDanmaku = (event as CustomEvent<number>).detail;
    }, { once: true });
  });
  await message(page).evaluate((element) => element.getAnimations()[0].finish());
  await expect(message(page)).toHaveCount(0);
  expect(await page.evaluate(() => (window as Window & { finishedDanmaku?: number }).finishedDanmaku)).toBe(3);
});

test("restored snapshots resume elapsed time and stale or cancelled presentations do not replay", async ({ page }) => {
  await page.goto("/#/danmaku");
  const start = Date.now() - 3000;
  await fixture(page, { id: 10, startedAtMs: start });
  await expect(message(page)).toHaveAttribute("data-presentation-id", "10");
  await expect(message(page)).toHaveAttribute("data-duration-ms", /\d/);
  const elapsed = await message(page).evaluate((element) => Number(element.getAnimations()[0].currentTime));
  expect(elapsed).toBeGreaterThanOrEqual(2900);
  await fixture(page, { id: 10, text: "重复快照不应重播" });
  await expect(message(page)).not.toContainText("重复快照");
  await page.evaluate(async () => (await import("/src/lib/danmaku.ts")).showDanmakuPreview(null));
  await expect(message(page)).toHaveCount(0);
  await fixture(page, { id: 9, text: "过期事件" });
  await fixture(page, { id: 10, text: "已取消快照" });
  await expect(message(page)).toHaveCount(0);
  await fixture(page, { id: 11, startedAtMs: Date.now() - 46_000 });
  await expect(message(page)).toHaveCount(0);
});

test("long content stays complete on one line with bounded duration and smaller text", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.clock.install();
  await page.goto("/#/danmaku");
  const text = "记得检查装备和队友状态，".repeat(24);
  await fixture(page, { id: 2, text, fontSize: 72, speedPxPerSec: 40, emote: "hello" });
  await expect(message(page)).toHaveAttribute("data-duration-ms", /\d/);
  await expect(message(page)).toHaveCSS("font-size", "24px");
  expect(Number(await message(page).getAttribute("data-duration-ms"))).toBeLessThanOrEqual(45_000);
  await expect(message(page).locator("span")).toHaveText(text);
  await expect(message(page)).toHaveCSS("white-space", "nowrap");
  await page.clock.fastForward(60_000);
  await expect(message(page)).toHaveCount(0);
});

test("top offset keeps the old default, accepts zero and fits large text plus emotes on different screens", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/#/danmaku");
  await fixture(page, { id: 2, topOffsetPx: undefined }); // Event from an older backend.
  await seek(page, 2000);
  expect((await message(page).boundingBox())!.y).toBe(120);
  await fixture(page, { id: 3, topOffsetPx: 0, fontSize: 72, emote: "hello" });
  await seek(page, 2000);
  expect((await message(page).boundingBox())!.y).toBe(0);

  let id = 4;
  for (const viewport of [{ width: 638, height: 360 }, { width: 320, height: 80 }, { width: 2560, height: 1440 }]) {
    await page.setViewportSize(viewport);
    await fixture(page, { id: id++, topOffsetPx: 2160, fontSize: 72, emote: "cheering" });
    await seek(page, 2000);
    const bounds = (await message(page).boundingBox())!;
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.height).toBeLessThanOrEqual(viewport.height);
    expect(bounds.y + bounds.height).toBeCloseTo(viewport.height, 1);
    const emote = (await message(page).locator("img").boundingBox())!;
    expect(emote.y).toBeGreaterThanOrEqual(0);
    expect(emote.y + emote.height).toBeLessThanOrEqual(viewport.height + .1);
  }
  await page.setViewportSize({ width: 1280, height: 720 });
  await fixture(page, { id: id++, topOffsetPx: 280, fontSize: 48, emote: "pointing", color: "#FFE082" });
  await expect(message(page)).toHaveAttribute("data-duration-ms", /\d/);
  await seek(page, Number(await message(page).getAttribute("data-duration-ms")) / 2);
  await message(page).locator("img").evaluate((image: HTMLImageElement) => image.decode());
  await page.screenshot({ path: testInfo.outputPath("danmaku-position.png"), omitBackground: true });
});

test("resize updates vertical bounds without restarting travel and removes position observers on destroy", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/#/danmaku");
  await fixture(page, { id: 2, topOffsetPx: 600, fontSize: 72, emote: "hello" });
  await seek(page, 2000);
  await message(page).evaluate((element) => {
    const animation = element.getAnimations()[0];
    Object.assign(window, { positionTest: { element, animation, duration: animation.effect!.getTiming().duration } });
  });
  await page.setViewportSize({ width: 638, height: 180 });
  await expect.poll(async () => {
    const bounds = (await message(page).boundingBox())!;
    return Math.round(bounds.y + bounds.height);
  }).toBe(180);
  expect(await page.evaluate(() => {
    const { element, animation, duration } = (window as any).positionTest;
    return { same: element.getAnimations()[0] === animation, time: animation.currentTime, durationUnchanged: animation.effect.getTiming().duration === duration };
  })).toEqual({ same: true, time: 2000, durationUnchanged: true });

  // A CSS-pixel viewport change also covers the layout change caused by moving between DPIs.
  await page.setViewportSize({ width: 320, height: 80 });
  await expect.poll(async () => Math.round((await message(page).boundingBox())!.y)).toBe(0);
  expect(await message(page).evaluate((element) => Number(element.getAnimations()[0].currentTime))).toBe(2000);
  await page.evaluate(async () => {
    const saved = (window as any).positionTest;
    saved.lastTop = saved.element.style.top;
    (await import("/src/lib/danmaku.ts")).showDanmakuPreview(null);
  });
  await expect(message(page)).toHaveCount(0);
  await page.setViewportSize({ width: 1280, height: 720 });
  expect(await page.evaluate(() => {
    const saved = (window as any).positionTest;
    return { top: saved.element.style.top, previousTop: saved.lastTop, animation: saved.animation.playState };
  })).toEqual({ top: "0px", previousTop: "0px", animation: "idle" });
});

for (const cancel of [false, true]) {
  test(`native event wins over a late ${cancel ? "active" : "null"} snapshot${cancel ? " after cancellation" : " and completes through IPC"}`, async ({ page }) => {
    await page.addInitScript(() => {
      const callbacks = new Map<number, (event: unknown) => void>();
      const listeners = new Map<string, number>();
      let sequence = 0;
      const bridge = {
        snapshotRequested: false,
        resolveSnapshot: (_payload: unknown) => {},
        finished: [] as number[],
        send(payload: unknown) {
          const handler = listeners.get("kotone://danmaku");
          if (handler !== undefined) callbacks.get(handler)?.({ payload });
        },
      };
      Object.assign(window, {
        nativeDanmakuTest: bridge,
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
        __TAURI_INTERNALS__: {
          transformCallback(callback: (event: unknown) => void) {
            const id = ++sequence;
            callbacks.set(id, callback);
            return id;
          },
          invoke(command: string, args: { event?: string; handler?: number; id?: number }) {
            if (command === "plugin:event|listen") {
              listeners.set(args.event!, args.handler!);
              return Promise.resolve(args.handler);
            }
            if (command === "get_danmaku") {
              bridge.snapshotRequested = true;
              return new Promise((resolve) => { bridge.resolveSnapshot = resolve; });
            }
            if (command === "finish_danmaku") bridge.finished.push(args.id!);
            return Promise.resolve(null);
          },
        },
      });
    });
    await page.goto("/#/danmaku");
    await page.waitForFunction(() => (window as Window & { nativeDanmakuTest: { snapshotRequested: boolean } }).nativeDanmakuTest.snapshotRequested);
    await page.evaluate((cancel) => {
      const bridge = (window as Window & { nativeDanmakuTest: { send(value: unknown): void; resolveSnapshot(value: unknown): void } }).nativeDanmakuTest;
      const presentation = { id: 11, text: "正在滑动", startedAtMs: Date.now() - 2000, expiresAtMs: Date.now() + 60_000, speedPxPerSec: 160, fontSize: 32, topOffsetPx: 310, color: "#FFFFFF", emote: "none" };
      bridge.send(presentation);
      if (cancel) bridge.send(null);
      bridge.resolveSnapshot(cancel ? { ...presentation, id: 10, text: "旧快照" } : null);
    }, cancel);
    if (cancel) {
      await expect(message(page)).toHaveCount(0);
    } else {
      await expect(message(page)).toHaveText("正在滑动");
      await expect(message(page)).toHaveAttribute("data-duration-ms", /\d/);
      await expect(message(page)).toHaveCSS("top", "310px");
      await message(page).evaluate((element) => element.getAnimations()[0].finish());
      await expect(message(page)).toHaveCount(0);
      expect(await page.evaluate(() => (window as Window & { nativeDanmakuTest: { finished: number[] } }).nativeDanmakuTest.finished)).toEqual([11]);
    }
  });
}

test("appearance settings persist speed, font, exact top offset, color and built-in emote", async ({ page }, testInfo) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
  const settings = page.getByTestId("danmaku-settings");
  await expect(settings).toBeVisible();
  await settings.getByLabel("弹幕速度", { exact: true }).fill("240");
  await settings.getByLabel("弹幕速度", { exact: true }).dispatchEvent("change");
  await expect(settings.getByText("240 px/s", { exact: true })).toBeVisible();
  await settings.getByLabel("弹幕字号", { exact: true }).fill("40");
  await settings.getByLabel("弹幕字号", { exact: true }).dispatchEvent("change");
  await expect(settings.getByText("40 px", { exact: true })).toBeVisible();
  const topSlider = settings.getByLabel("弹幕距顶部", { exact: true });
  const topNumber = settings.getByLabel("弹幕距顶部像素", { exact: true });
  await expect(topNumber).toHaveValue("120");
  await topSlider.fill("432");
  await topSlider.dispatchEvent("change");
  await expect(topNumber).toHaveValue("432");
  await topNumber.fill("73");
  await topNumber.press("Enter");
  await expect(topSlider).toHaveValue("73");
  for (const invalid of ["", "1.5", "-1", "2161"]) {
    await topNumber.fill(invalid);
    await topNumber.press("Enter");
    await expect(settings.getByRole("alert")).toHaveText("请输入 0 到 2160 之间的整数像素。");
    expect((await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings())).assistant.danmakuTopOffsetPx).toBe(73);
  }
  await topNumber.fill("0");
  await topNumber.press("Enter");
  await expect(topSlider).toHaveValue("0");
  await topNumber.fill("345");
  await topNumber.press("Enter");
  await expect(topSlider).toHaveValue("345");
  await expect(settings.getByRole("alert")).toHaveCount(0);
  await settings.getByRole("button", { name: "弹幕颜色霓虹青", exact: true }).click();
  await expect(settings.getByRole("button", { name: "弹幕颜色霓虹青", exact: true })).toHaveAttribute("aria-pressed", "true");
  await settings.getByRole("button", { name: "弹幕表情加油", exact: true }).click();
  await expect(settings.getByRole("button", { name: "弹幕表情加油", exact: true })).toHaveAttribute("aria-pressed", "true");
  const saved = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
  expect(saved.assistant).toMatchObject({ danmakuSpeedPxPerSec: 240, danmakuFontSize: 40, danmakuTopOffsetPx: 345, danmakuColor: "#00E5FF", danmakuEmote: "cheering" });
  await page.getByTestId("assistant-nav-tasks").click();
  await page.getByTestId("assistant-nav-settings").click();
  await expect(topNumber).toHaveValue("345");
  await expect(page.getByRole("status").filter({ hasText: "弹幕样式已保存" })).toHaveCount(0);
  await settings.screenshot({ path: testInfo.outputPath("danmaku-settings.png") });
  await page.setViewportSize({ width: 1040, height: 900 });
  await page.getByRole("main").evaluate((element) => { element.scrollTop = element.scrollHeight; });
  await page.screenshot({ path: testInfo.outputPath("danmaku-settings-fullscreen-hint.png"), animations: "disabled" });
});
