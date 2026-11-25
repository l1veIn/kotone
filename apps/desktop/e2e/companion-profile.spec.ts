import { expect, test } from "@playwright/test";
import { openChat, openSettingsTab } from "./nav";
import { screenshot } from "./screenshot.cjs";

// ADR 022 / 023 / 028：角色页只有“角色 / 记忆 / 技能”（聊天从侧栏进入，设置在「设置」）。浏览器预览用内存里的示例工作区和示例角色。
test("rename her, edit SOUL, memory and game memory, manage skills", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 860 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  for (const tab of ["identity", "memory", "skills"]) {
    await expect(page.getByTestId(`assistant-nav-${tab}`)).toBeVisible();
  }
  for (const tab of ["chat", "settings"]) await expect(page.getByTestId(`assistant-nav-${tab}`)).toHaveCount(0);

  // 角色：给当前角色（琴音）改名 → 页标题、侧栏都跟着变；最后改回来
  await page.getByTestId("assistant-nav-identity").click();
  const identity = page.getByTestId("identity-tab");
  await expect(identity).toBeVisible();
  await expect(page.getByTestId("identity-name-save")).toBeDisabled();
  await page.getByTestId("identity-name").fill("小星");
  await page.getByTestId("identity-name-save").click();
  await expect(page.getByTestId("assistant-page-title")).toHaveText("小星");
  await expect(page.getByTestId("settings-nav-assistant")).toHaveText("小星");

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
  await screenshot(page, { path: testInfo.outputPath("identity.png"), fullPage: true });

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
  await screenshot(page, { path: testInfo.outputPath("memory.png"), fullPage: true });
  await page.getByTestId("memory-game-delete").click();
  await page.getByTestId("memory-game-delete-confirm").click();
  await expect(page.getByTestId("memory-games-empty")).toBeVisible();

  // 技能：列表与删除
  await page.getByTestId("assistant-nav-skills").click();
  await expect(page.getByTestId("assistant-nav-skills")).toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("assistant-nav-memory")).not.toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("skill")).toHaveCount(1);
  await expect(page.getByTestId("skill")).toContainText("pubg-callouts");
  await screenshot(page, { path: testInfo.outputPath("skills.png"), fullPage: true });
  await page.getByTestId("skill-delete").click();
  await page.getByTestId("skill-delete-confirm").click();
  await expect(page.getByTestId("skills-empty")).toBeVisible();

  // 改回原来的名字
  await page.getByTestId("assistant-nav-identity").click();
  await page.getByTestId("identity-name").fill("琴音");
  await page.getByTestId("identity-name-save").click();
  await expect(page.getByTestId("assistant-page-title")).toHaveText("琴音");
});

test("roles: switch, create, wake word check, placeholder avatar", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 900 });
  await page.goto("/#/settings?onboarding=never");
  await page.getByTestId("settings-nav-assistant").click();
  await page.getByTestId("assistant-nav-identity").click();
  const cards = page.getByTestId("role-card");
  // 内置：琴音（当前）、大肥鱼、Claude大小姐
  await expect(cards).toHaveCount(3);
  // 自带形象（社区二创）：用自己的表情头像，编辑区写着署名和授权
  await expect(cards.filter({ hasText: "大肥鱼" }).getByTestId("avatar-face")).toHaveCount(1);
  await cards.filter({ hasText: "Claude大小姐" }).click({ position: { x: 8, y: 8 } });
  await expect(page.getByTestId("role-face-credit")).toContainText("CC BY-NC-SA 4.0");
  await expect(page.getByTestId("role-face-credit")).toContainText("与 Anthropic 无关");
  await expect(page.getByTestId("role-wake")).toHaveValue("你好大小姐");
  await expect(cards.filter({ hasText: "琴音" }).getByTestId("role-current")).toBeVisible();
  // 换成大肥鱼：名字跟着变，她成了当前角色；聊天页用她的表情头像，界面上的说明也用她的名字
  await cards.filter({ hasText: "大肥鱼" }).getByTestId("role-switch").click();
  await expect(page.getByTestId("assistant-page-title")).toHaveText("大肥鱼");
  await expect(page.getByTestId("settings-nav-assistant")).toHaveText("大肥鱼");
  await expect(cards.filter({ hasText: "大肥鱼" }).getByTestId("role-current")).toBeVisible();
  await openChat(page);
  await expect(page.getByTestId("avatar-face").first()).toBeVisible();
  await expect(page.getByLabel("跟大肥鱼说")).toBeVisible();
  // 语音唤醒用她自己的唤醒词，不再给琴音的预置选项（「设置 → 怎么叫她」）
  await openSettingsTab(page, "call");
  await expect(page.getByTestId("wake-role-word")).toHaveText("你好大肥鱼");
  await expect(page.getByTestId("settings-call")).toContainText("说“你好大肥鱼”叫醒大肥鱼");
  await expect(page.getByRole("combobox", { name: "唤醒词" })).toHaveCount(0);
  await page.getByTestId("settings-nav-assistant").click();
  await expect(page.getByTestId("assistant-nav-identity")).toHaveAttribute("aria-current", "page");

  // 新建角色：没有头像时显示名字的第一个字（占位）
  await page.getByTestId("role-new-name").fill("星野");
  await page.getByTestId("role-create").click();
  await expect(cards).toHaveCount(4);
  await expect(page.getByTestId("identity-name")).toHaveValue("星野");
  await expect(cards.filter({ hasText: "星野" }).getByTestId("avatar-letter")).toHaveText("星");

  // 唤醒词：默认是名字重复两遍；太短、不是汉字都提示
  const wake = page.getByTestId("role-wake");
  await expect(wake).toHaveAttribute("placeholder", "星野星野");
  await wake.fill("星野");
  await expect(page.getByTestId("role-wake-problem")).toContainText("至少 3 个字");
  await expect(page.getByTestId("role-wake-save")).toBeEnabled();
  await wake.fill("Hoshino");
  await expect(page.getByTestId("role-wake-problem")).toContainText("只能用汉字");
  await wake.fill("星野银行");
  await expect(page.getByTestId("role-wake-check")).toHaveAttribute("data-ok", "true");
  // 多音字给下拉框
  await page.getByTestId("role-wake-reading").selectOption("háng");
  await page.getByTestId("role-wake-save").click();
  await expect(page.getByTestId("role-wake-save")).toBeDisabled();
  await screenshot(page, { path: testInfo.outputPath("roles.png"), fullPage: true });

  // 删掉自己建的角色；换回琴音
  await page.getByTestId("role-delete").click();
  await expect(cards).toHaveCount(3);
  await cards.filter({ hasText: "琴音" }).getByTestId("role-switch").click();
  await expect(page.getByTestId("assistant-page-title")).toHaveText("琴音");
});
