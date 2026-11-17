import { expect, test } from "@playwright/test";

// ADR 022 / 023：「琴音」页的“她是谁 / 记忆 / 技能”。浏览器预览用内存里的示例工作区。
test("rename her, edit SOUL, memory and game memory, manage skills", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  for (const tab of ["chat", "identity", "memory", "skills", "settings"]) {
    await expect(page.getByTestId(`assistant-nav-${tab}`)).toBeVisible();
  }

  // 她是谁：改名 → 页标题、侧栏都跟着变；恢复默认
  await page.getByTestId("assistant-nav-identity").click();
  const identity = page.getByTestId("identity-tab");
  await expect(identity).toBeVisible();
  await expect(page.getByTestId("identity-name-save")).toBeDisabled();
  await page.getByTestId("identity-name").fill("小雨");
  await page.getByTestId("identity-name-save").click();
  await expect(page.getByTestId("assistant-page-title")).toHaveText("小雨");
  await expect(page.getByTestId("settings-nav-assistant")).toHaveText("小雨");

  // SOUL：计字、超上限不能保存、保存后“还原”消失
  const soul = page.getByTestId("identity-soul-text");
  await expect(soul).toHaveValue(/游戏陪玩 Agent/);
  await soul.fill("# SOUL\n\n说话简短。");
  await expect(page.getByTestId("identity-soul-count")).toHaveText(/^\d+ \/ 4000 字$/);
  await page.getByTestId("identity-soul-save").click();
  await expect(page.getByTestId("identity-soul-save")).toBeDisabled();
  await soul.fill("长".repeat(4001));
  await expect(page.getByTestId("identity-soul-save")).toBeDisabled();
  await expect(page.getByTestId("identity-soul-count")).toHaveClass(/text-destructive/);
  await page.screenshot({ path: testInfo.outputPath("identity.png"), fullPage: true });

  // 头像：超过 2 MB 被拒
  await page.getByTestId("identity-avatar-input").setInputFiles({ name: "big.png", mimeType: "image/png", buffer: Buffer.alloc(2 * 1024 * 1024 + 1) });
  await expect(page.getByTestId("identity-avatar-error")).toHaveText("头像不能超过 2 MB");

  // 记忆：USER / MEMORY / 游戏记忆
  await page.getByTestId("assistant-nav-memory").click();
  await expect(page.getByTestId("memory-tab")).toBeVisible();
  await expect(page.getByTestId("memory-notes-text")).toHaveValue(/中文语音/);
  await page.getByTestId("memory-user-text").fill("- 叫我小林");
  await page.getByTestId("memory-user-save").click();
  await expect(page.getByTestId("memory-user-save")).toBeDisabled();
  await expect(page.getByTestId("memory-game")).toHaveText(["pubg"]);
  await expect(page.getByTestId("memory-game-editor-text")).toHaveValue(/P 城/);
  await page.screenshot({ path: testInfo.outputPath("memory.png"), fullPage: true });
  await page.getByTestId("memory-game-delete").click();
  await page.getByTestId("memory-game-delete-confirm").click();
  await expect(page.getByTestId("memory-games-empty")).toBeVisible();

  // 技能：列表与删除
  await page.getByTestId("assistant-nav-skills").click();
  await expect(page.getByTestId("skill")).toHaveCount(1);
  await expect(page.getByTestId("skill")).toContainText("pubg-callouts");
  await page.screenshot({ path: testInfo.outputPath("skills.png"), fullPage: true });
  await page.getByTestId("skill-delete").click();
  await page.getByTestId("skill-delete-confirm").click();
  await expect(page.getByTestId("skills-empty")).toBeVisible();

  // 恢复默认名字
  await page.getByTestId("assistant-nav-identity").click();
  await page.getByTestId("identity-name").fill("");
  await page.getByTestId("identity-name-save").click();
  await expect(page.getByTestId("assistant-page-title")).toHaveText("琴音");
});
