import { expect, test } from "@playwright/test";

// 统一模型清单：ASR 原有列表不变，OCR / TTS 分栏可下载、切换并提示在线 OCR 会上传截图。
test("model page splits ASR / OCR / TTS and keeps the ASR list unchanged", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-models").click();

  await expect(page.getByTestId("model-category-asr")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("model-list")).toBeVisible();

  await page.getByTestId("model-category-ocr").click();
  const ocr = page.getByTestId("catalog-list-ocr");
  await expect(ocr).toBeVisible();
  await expect(page.getByTestId("model-list")).toHaveCount(0);
  await expect(ocr).toContainText("每次识别都会把截图上传");
  const korean = ocr.getByRole("radio").filter({ hasText: "Korean" });
  await korean.getByRole("button", { name: "下载" }).click();
  await expect(korean.getByRole("button", { name: "删除" })).toBeVisible();
  await korean.click();
  await expect(korean).toHaveAttribute("aria-checked", "true");
  await expect(korean).toContainText("使用中");

  await page.getByTestId("model-category-tts").click();
  const tts = page.getByTestId("catalog-list-tts");
  const system = tts.getByRole("radio").filter({ hasText: "Windows 系统语音" });
  await expect(system).toHaveAttribute("aria-checked", "true");
  await expect(system.getByRole("button", { name: "删除" })).toHaveCount(0);
  const kokoro = tts.getByRole("radio").filter({ hasText: "Kokoro" });
  await kokoro.click();
  await expect(kokoro).toHaveAttribute("aria-checked", "false");
  await kokoro.getByRole("button", { name: "下载" }).click();
  await kokoro.click();
  await expect(kokoro).toHaveAttribute("aria-checked", "true");

  await page.getByTestId("model-category-asr").click();
  await expect(page.getByTestId("model-list")).toBeVisible();
});

// 联网搜索和语音识别、文字识别、语音合成并列（ADR 026）：DeepSeek 是一个选项，也可以不联网。
test("web search is a model category: DeepSeek or off", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-advanced").click();
  await page.getByTestId("advanced-nav-models").click();
  await page.getByTestId("model-category-search").click();
  const search = page.getByTestId("catalog-list-search");
  await expect(search).toContainText("搜索词和要读的网址会发给服务商");
  const deepseek = search.getByRole("radio").filter({ hasText: "DeepSeek 联网搜索" });
  await expect(deepseek).toHaveAttribute("aria-checked", "true");
  await expect(deepseek.getByRole("button", { name: "下载" })).toHaveCount(0);
  const off = search.getByRole("radio").filter({ hasText: "不联网" });
  await off.click();
  await expect(off).toHaveAttribute("aria-checked", "true");
  await expect(deepseek).toHaveAttribute("aria-checked", "false");
});
