import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";

async function makeModelAvailable(page: Page) {
  await page.evaluate(async () => {
    const { downloadModel } = await import("/src/lib/ipc.ts");
    await downloadModel("x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05");
  });
}

for (const size of [
  { name: "desktop", width: 1060, height: 740 },
  { name: "narrow", width: 640, height: 720 },
]) {
  test(`the ${size.name} home puts the only recording hotkey editor inside the hero`, async ({ page }) => {
    await page.setViewportSize({ width: size.width, height: size.height });
    await page.goto("/#/settings?onboarding=never");
    await makeModelAvailable(page);

    const hero = page.getByTestId("general-hero");
    const key = hero.getByRole("textbox", { name: "录音热键", exact: true });
    await expect(hero.getByTestId("voice-input-status")).toHaveText("语音输入已就绪");
    await expect(key).toHaveValue("CapsLock");
    await expect(key).toBeInViewport();
    await expect(page.locator("#hotkey-input")).toHaveCount(1);
    await expect(page.getByRole("button", { name: /^(?:▶ )?启动(?: Kotone)?$/ })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "语音输入", exact: true })).toHaveAttribute("aria-current", "page");
    await expect(page.getByRole("button", { name: "提醒助手", exact: true })).toBeVisible();

    if (process.env.KOTONE_015_UI_SCREENSHOTS) {
      const outDir = process.env.KOTONE_015_UI_SCREENSHOTS;
      await mkdir(outDir, { recursive: true });
      await page.screenshot({ path: join(outDir, `home-${size.name}.png`), animations: "disabled" });
    }

    await key.fill("F9");
    await key.press("Enter");
    await expect(hero.getByText("当前：F9", { exact: true })).toBeVisible();
  });
}

test("the home distinguishes warming up while armed and an explicit pause", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await makeModelAvailable(page);
  const status = page.getByTestId("voice-input-status");
  await expect(status).toHaveText("语音输入已就绪");

  await page.evaluate(async () => {
    const { runtimeStore } = await import("/src/lib/stores/runtime.ts");
    runtimeStore.update((runtime) => runtime && { ...runtime, stage: "warmup" });
  });
  await expect(status).toHaveText("正在准备模型，可以按键录音");

  await page.evaluate(async () => (await import("/src/lib/ipc.ts")).stopRuntime());
  await expect(status).toHaveText("语音输入已暂停");
  await expect(page.getByTestId("general-hero")).toContainText("可从托盘菜单恢复");

  await page.evaluate(async () => (await import("/src/lib/ipc.ts")).startRuntime());
  await expect(status).toHaveText("语音输入已就绪");
});
