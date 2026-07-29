import { expect, test } from "@playwright/test";

test("WebView hotkey fallback recognizes combos and paired releases", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");

  const result = await page.evaluate(async () => {
    const { keyboardEventCombo, releasesHotkey } = await import("/src/lib/hotkeyCombo.ts");
    return {
      combo: keyboardEventCombo(
        new KeyboardEvent("keydown", {
          code: "KeyV",
          key: "v",
          ctrlKey: true,
          altKey: true,
        }),
      ),
      mainReleasedAfterModifiers: releasesHotkey(
        new KeyboardEvent("keyup", { code: "KeyV", key: "v" }),
        "Ctrl+Alt+V",
      ),
      modifierReleasedFirst: releasesHotkey(
        new KeyboardEvent("keyup", { code: "ControlLeft", key: "Control" }),
        "Ctrl+Alt+V",
      ),
      unrelatedRelease: releasesHotkey(
        new KeyboardEvent("keyup", { code: "KeyX", key: "x" }),
        "Ctrl+Alt+V",
      ),
    };
  });

  expect(result).toEqual({
    combo: "Ctrl+Alt+V",
    mainReleasedAfterModifiers: true,
    modifierReleasedFirst: true,
    unrelatedRelease: false,
  });
});
