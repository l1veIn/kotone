import { expect, test } from "@playwright/test";

test("WebView hotkey fallback recognizes combos and paired releases", async ({ page }) => {
  await page.goto("/#/settings?onboarding=never");

  const result = await page.evaluate(async () => {
    const { keyboardEventCombo, mouseEventCombo, releasesHotkey } = await import(
      "/src/lib/hotkeyCombo.ts"
    );
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
      mouse4: mouseEventCombo(new MouseEvent("mousedown", { button: 3 })),
      modifiedMouse5: mouseEventCombo(
        new MouseEvent("mousedown", { button: 4, ctrlKey: true, shiftKey: true }),
      ),
      ordinaryMouseButton: mouseEventCombo(new MouseEvent("mousedown", { button: 0 })),
    };
  });

  expect(result).toEqual({
    combo: "Ctrl+Alt+V",
    mainReleasedAfterModifiers: true,
    modifierReleasedFirst: true,
    unrelatedRelease: false,
    mouse4: "Mouse4",
    modifiedMouse5: "Ctrl+Shift+Mouse5",
    ordinaryMouseButton: null,
  });
});
