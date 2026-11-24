// 插件 logo（ADR 039 D8，docs/design/lol-app.md 第五节）：插件自己的 `plugins/<id>/logo.svg` 在这里一次收齐
// （Vite 的 import.meta.glob；开发时是开发服务器上的地址，打包后是同源的 /assets/… 或 data: 地址，主窗口 CSP 的
// img-src 'self' data: 都放行）。新插件放一个 logo.svg 就有，不用改这里，也不用改插件的 ui/index.ts。
//
// 没有 logo 的插件（以后的第三方）用“首字块”：名字的第一个字（英文取第一个字母大写），底色按插件 id 定，
// 同一个插件每次都是同一个颜色。第三方 MCP 服务器自己声明了图标（MCP 的 `icons`）时用它：`PluginLogo` 的 `src`
// （`mcpServers.ts` 的 `remoteIcon`，只用 data: 的，主窗口不去连别人的网址拿图）。

/** 插件 id → logo 地址。 */
const LOGOS: Record<string, string> = Object.fromEntries(
  Object.entries(import.meta.glob<string>("../../../../plugins/*/logo.svg", { eager: true, query: "?url", import: "default" }))
    .map(([path, url]) => [path.split("/plugins/")[1]?.split("/")[0] ?? "", url]),
);

/** 插件的 logo 地址；没有 logo 时 null（用 [`fallbackLogo`]）。 */
export function pluginLogo(id: string): string | null {
  return LOGOS[id] ?? null;
}

/** 首字块的 8 个底色：5 个内置插件的主色，加石板、玫红、橄榄（设计文档第五节 3）。白字在上面都看得清。 */
export const FALLBACK_COLORS = ["#4c58dd", "#179160", "#9b47c8", "#0b87aa", "#d46a17", "#5b6b7f", "#b0466f", "#6f7a1f"] as const;

/** 没有 logo 的插件的首字块：名字的第一个字（英文大写），底色按 id 的哈希从 8 个里挑。 */
export function fallbackLogo(id: string, name: string): { text: string; color: string } {
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.codePointAt(0)!) >>> 0;
  const first = Array.from(name.trim() || id)[0] ?? "?";
  return { text: first.toUpperCase(), color: FALLBACK_COLORS[hash % FALLBACK_COLORS.length] };
}
