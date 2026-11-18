import { expect, test, type Page } from "@playwright/test";

// 琴音的对话气泡页（#/companion）：浏览器预览里用 kotone-bubble-preview 事件喂状态。
const SHOTS = process.env.KOTONE_BUBBLE_SHOTS;

async function push(page: Page, snap: Record<string, unknown>) {
  await page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("kotone-bubble-preview", { detail }));
  }, {
    heard: "", askSeconds: null, askStartedMs: null, corner: "bottom-right", label: "", text: "", ...snap,
  });
}

test.describe("对话气泡", () => {
  test.use({ viewport: { width: 440, height: 260 } });

  test("依次显示 在听 → 在想 → 在说 → 追问 → 出错，隐藏后清空", async ({ page }) => {
    await page.goto("/#/companion");
    const stage = page.getByTestId("companion-bubble");
    await expect(stage).toBeAttached();
    await expect(page.getByRole("status")).toHaveCount(0);

    const steps: [string, Record<string, unknown>, string | null][] = [
      ["listening", { revision: 1, phase: "listening", label: "在听…", heard: "帮我看看队友说了" }, "“帮我看看队友说了”"],
      ["thinking", { revision: 2, phase: "thinking", label: "正在看屏幕", text: "帮我看看队友说了什么" }, "“帮我看看队友说了什么”"],
      ["speaking", { revision: 3, phase: "speaking", label: "琴音", text: "队友说中路的人消失了，让你注意下路。还有人在问要不要一起打小龙。" }, "队友说中路的人消失了，让你注意下路。还有人在问要不要一起打小龙。"],
      ["asking", { revision: 4, phase: "asking", label: "等你回答", text: "要我帮你回一句“马上来”吗？", askSeconds: 8, askStartedMs: Date.now() }, "要我帮你回一句“马上来”吗？"],
      ["error", { revision: 5, phase: "error", label: "出错了", text: "AI 连接没响应，检查一下网络或 API Key。" }, "AI 连接没响应，检查一下网络或 API Key。"],
    ];
    for (const [phase, snap, text] of steps) {
      await push(page, snap);
      const bubble = page.getByRole("status");
      await expect(bubble).toHaveAttribute("data-phase", phase);
      await expect(page.getByTestId("companion-label")).toHaveText(String(snap.label));
      if (text) await expect(page.getByTestId("companion-text")).toHaveText(text);
      if (SHOTS) {
        await page.waitForTimeout(400);
        await page.screenshot({ path: `${SHOTS}/bubble-${phase}.png`, omitBackground: true });
      }
    }

    // 迟到的旧序号不覆盖新内容
    await push(page, { revision: 2, phase: "thinking", label: "在想…" });
    await expect(page.getByRole("status")).toHaveAttribute("data-phase", "error");

    await push(page, { revision: 6, phase: "hidden" });
    await expect(page.getByRole("status")).toHaveCount(0);

    // 左上角：气泡在头像右边
    await push(page, { revision: 7, phase: "speaking", label: "琴音", text: "我在左上角。", corner: "top-left" });
    await expect(stage).toHaveClass(/left/);
    await expect(stage).toHaveClass(/top/);
    if (SHOTS) {
      await page.waitForTimeout(400);
      await page.screenshot({ path: `${SHOTS}/bubble-top-left.png`, omitBackground: true });
    }
  });

  test("设置页有对话气泡分区：默认开启、右下角，改位置会保存", async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1280, height: 820 });
    await page.goto("/#/settings?onboarding=never");
    await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-settings").click();
    await expect(page.locator("#bubble-settings")).toContainText("对话气泡");
    await expect(page.getByTestId("bubble-enabled")).toHaveAttribute("aria-checked", "true");
    await expect(page.getByTestId("bubble-danmaku")).toHaveAttribute("aria-checked", "false");
    await page.getByLabel("气泡位置").click();
    await page.getByRole("option", { name: "左下角" }).click();
    await page.getByTestId("bubble-enabled").scrollIntoViewIfNeeded();
    await page.screenshot({ path: SHOTS ? `${SHOTS}/bubble-settings.png` : testInfo.outputPath("bubble-settings.png") });
    const settings = await page.evaluate(async () => (await import("/src/lib/ipc.ts")).getSettings());
    expect(settings.assistant.bubble).toMatchObject({ enabled: true, corner: "bottom-left", offsetY: 0, alsoDanmaku: false });
  });
});
