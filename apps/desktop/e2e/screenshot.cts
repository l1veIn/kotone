// spec 里的截图（不是测试文件，供各 spec 引用：`import { screenshot } from "./screenshot.cjs"`）。是 .cts：本体的 spec
// 按 ES 模块编译、plugins/*/e2e 的按 CommonJS 编译，两边都引用它；写成 .ts 的话同一个文件会被按两种方式编译，
// 一起跑时插件的 spec 报 “Unexpected token 'export'”。
// 截图是给人和 Agent 看界面用的，不是断言：CI 里不截（没人看、也不上传，只花时间）。
// KOTONE_E2E_SHOTS=1：CI 里也截；KOTONE_E2E_SHOTS=0：本地也不截（例如推送前多 worker 全量跑，不覆盖
// target/plugin-shots/ 里的审查截图）。
import type { Locator, LocatorScreenshotOptions, Page, PageScreenshotOptions } from "@playwright/test";

const setting = process.env.KOTONE_E2E_SHOTS;
const SHOOT = setting === "1" || (setting !== "0" && !process.env.CI);

/** 截一张图存到 `options.path`。不截的时候，截元素前的等待照旧（等它出现、只有一个），省掉的只是存图。 */
export async function screenshot(
  target: Page | Locator,
  options: PageScreenshotOptions & LocatorScreenshotOptions & { path: string },
): Promise<void> {
  if (SHOOT) {
    await target.screenshot(options);
  } else if ("waitFor" in target) {
    await target.waitFor();
  }
}
