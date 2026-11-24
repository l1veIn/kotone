// 图片（ADR 039 D6：素材不进安装包；docs/design/lol-app.md 第一节 7“取图”）。页面数据里只有客户端的资源路径
// （`/lol-game-data/assets/v1/champion-icons/103.png` 这样）。在 Kotone 里先从回环代理的 `/assets/lol/<路径>` 取
// （插件的素材层：缓存 → 客户端 → CommunityDragon），取不到再直接用 CommunityDragon；在 Claude 这类宿主里只有
// CommunityDragon（域名写在资源的 CSP 声明里）。都取不到时页面显示名字的第一个字（Pic.svelte）。

const CDRAGON = "https://raw.communitydragon.org/latest/plugins/";

/** 客户端路径的前缀 → CommunityDragon 的目录；第三项：后面要不要全部小写（游戏资料要，前端的静态资源不要）。 */
const RULES: [string, string, boolean][] = [
  ["/lol-game-data/assets/", "rcp-be-lol-game-data/global/default/", true],
  ["/fe/lol-static-assets/", "rcp-fe-lol-static-assets/global/default/", false],
  ["/fe/lol-shared-components/", "rcp-fe-lol-shared-components/global/default/", false],
];

/** Kotone 回环代理上的素材前缀：App 的地址是 `/<密钥>/app/lol/<页面>`，素材在 `/<密钥>/assets/lol/`。不在 Kotone 里是 null。 */
export const kotoneAssets: string | null = (() => {
  const match = /^(\/[A-Za-z0-9]+)\/app\/lol\/[a-z0-9-]+$/.exec(location.pathname);
  return match ? `${match[1]}/assets/lol/` : null;
})();

/** 客户端路径 → CommunityDragon 的地址；不认识的前缀给空。 */
export function onlineUrl(path: string): string {
  const p = path.startsWith("/") ? path : `/${path}`;
  for (const [from, to, lower] of RULES) {
    if (p.startsWith(from)) {
      const rest = p.slice(from.length);
      return CDRAGON + to + (lower ? rest.toLowerCase() : rest);
    }
  }
  return "";
}

/** 按顺序试的地址（Kotone 的素材层，然后 CommunityDragon）。路径空时没有。 */
export function sources(path: string | null | undefined): string[] {
  if (!path) return [];
  const out: string[] = [];
  if (kotoneAssets) out.push(kotoneAssets + path.replace(/^\/+/, ""));
  const online = onlineUrl(path);
  if (online) out.push(online);
  return out;
}

/** 英雄头像（数据里没带 `icon` 时按 id 推）。 */
export function championIconPath(id: number): string {
  return id > 0 ? `/lol-game-data/assets/v1/champion-icons/${id}.png` : "";
}

/** 召唤师头像。 */
export function profileIconPath(id: number | null | undefined): string {
  return typeof id === "number" && id >= 0 ? `/lol-game-data/assets/v1/profile-icons/${id}.jpg` : "";
}

/** 段位小徽章（`DIAMOND` → `diamond.svg`）；没定级不画（设计第一节 7）。 */
export function crestPath(tier: string): string {
  return /^[A-Z]+$/i.test(tier) ? `/fe/lol-static-assets/images/ranked-mini-crests/${tier.toLowerCase()}.svg` : "";
}
