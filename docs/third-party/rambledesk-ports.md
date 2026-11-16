# 来源记录：从 RambleDesk 复制的前端代码

设置窗口的前端重构（[ADR 016](../adr/016-ui-foundation-shadcn.md)）从 RambleDesk 复制了一部分前端代码。本文记录每份代码的来源、上游许可证与 Kotone 的修改，参照 RambleDesk 自己的 `docs/CODEG_PORTS.md`。

- **RambleDesk**：`C:\Users\A\Desktop\rambledesk`（`apps/desktop`），MIT，Copyright (c) 2026 l1veIn，与 Kotone 同一作者、同一许可证。复制时的工作区状态未固定 commit。
- **Codeg**：<https://github.com/xintaofei/codeg>，固定 commit `3ebdfed1d7c0b71d71880a3d2e0f8e09545feae1`，Apache-2.0，版权归 Codeg 作者和贡献者。RambleDesk 中源自 Codeg 的部分，Kotone 照搬时保留文件头的 Apache-2.0 声明；条款全文见 [`licenses/codeg-APACHE-2.0.txt`](../../licenses/codeg-APACHE-2.0.txt)。
- **shadcn-svelte**：<https://github.com/huntabyte/shadcn-svelte>，MIT。基础组件以源码形式复制。
- 其余依赖（bits-ui、lucide、tailwind-merge、tailwind-variants、clsx、tw-animate-css 等）作为 npm 包使用，各自遵循自己的许可证，见 `THIRD_PARTY_NOTICES.md`。

## 第一步：底子（shadcn-svelte + 外观系统）

| Kotone 文件 | 来源 | 上游许可证 | Kotone 的修改 |
| --- | --- | --- | --- |
| `apps/desktop/components.json` | RambleDesk `apps/desktop/components.json` | MIT | 原样。 |
| `apps/desktop/src/lib/utils.ts` | RambleDesk `src/lib/utils.ts`（shadcn-svelte 约定） | MIT | 加中文说明。 |
| `apps/desktop/src/lib/components/ui/{alert,badge,button,collapsible,dialog,dropdown-menu,scroll-area,select,separator,skeleton,switch,tabs,tooltip}/` | RambleDesk `src/lib/components/ui/`（shadcn-svelte 生成） | MIT | `tailwind-variants` 改用 `tailwind-variants/lite`（完整版自带一份 tailwind-merge，重复打包约 30 kB；组件都经 `cn()` 合并类名，行为不变）。 |
| `apps/desktop/src/lib/components/ui/{input,textarea}/` | 按 shadcn-svelte 注册表的 input / textarea 写法补齐 | MIT | 与其它组件同一风格。 |
| `apps/desktop/src/lib/appearance/palettes.css` | RambleDesk `src/lib/appearance/palettes.css` ← Codeg `src/app/globals.css`、`src/lib/theme-presets.ts` | **Apache-2.0**（文件头保留声明） | 保留 Codeg 的 12 套配色（neutral … violet）与 RambleDesk 加的 success / warning / info；删掉 RambleDesk 自己的 classic 配色与窗口外框变量；新增 Kotone 自己的 `kotone` 配色（深色 = 现有界面，另配浅色）。 |
| `apps/desktop/src/lib/appearance/appearanceSettings.ts` | RambleDesk `src/lib/appearance/appearanceSettings.ts` ← Codeg `src/lib/theme-presets.ts`（配色 id 与色块） | **Apache-2.0**（文件头保留声明） | 默认 `kotone` 配色；中文名称；明暗模式并入同一份偏好；字体改为 Windows 中文字体 + 自定义；去掉代码字体、背景图片；缩放档位 80–150%。自定义字体名的校验沿用 RambleDesk。 |
| `apps/desktop/src/lib/appearance/appearancePreferences.ts` | RambleDesk `src/lib/appearance/appearancePreferences.ts` | MIT | 只保留偏好读写（去掉背景图片 IndexedDB 存储）；键名 `kotone.appearance.v1`；读失败回退默认。 |
| `apps/desktop/src/lib/appearance/appearanceRuntime.ts` | RambleDesk `src/lib/appearance/appearanceRuntime.ts` | MIT | 只在设置窗口调用；缩放用 CSS `zoom`（不需要额外的窗口权限）；不注册 Ctrl +/- 快捷键（会和热键录制冲突）；新增 `data-bridge`：非默认外观时让旧页面的固定颜色跟随主题（见 `src/app.css`）。 |
| `apps/desktop/src/lib/appearance/AppearanceSettingsPanel.svelte` | 参照 RambleDesk `src/lib/appearance/AppearanceSettingsPanel.svelte` 重写 | MIT | runes 写法；中文；去掉背景图片与代码字体部分。 |

`src/app.css` 中 shadcn 设计变量与 `@theme inline` 映射的写法参照 RambleDesk `src/app.css`；Kotone 的变量值、`data-bridge` 过渡规则与作用域边框规则为 Kotone 自己的实现。
