// 图片（ADR 039 D6：素材不进安装包）。在 Kotone 里先从回环代理的 /assets/lol/<路径> 取（插件从客户端取，
// 有磁盘缓存），取不到再用 CommunityDragon；在 Claude 这类宿主里只有 CommunityDragon（域名写在资源的 CSP 声明里）。
// 路径照客户端的资源接口写（lol-game-data/assets/v1/…），CommunityDragon 是同一份文件的公开镜像。

const CDRAGON_GAME_DATA = "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/v1";

/** Kotone 回环代理上的素材前缀：App 的地址是 `/<密钥>/app/lol/<页面>`，素材在 `/<密钥>/assets/lol/`。不在 Kotone 里是 null。 */
export const kotoneAssets: string | null = (() => {
  const match = /^(\/[A-Za-z0-9]+)\/app\/lol\/[a-z0-9-]+$/.exec(location.pathname);
  return match ? `${match[1]}/assets/lol/` : null;
})();

/** 按顺序试的地址。 */
function sources(gameDataPath: string): string[] {
  const online = `${CDRAGON_GAME_DATA}/${gameDataPath}`;
  return kotoneAssets ? [`${kotoneAssets}lol-game-data/assets/v1/${gameDataPath}`, online] : [online];
}

/** 英雄头像（方形）。 */
export function championIcon(id: number): string[] {
  return sources(`champion-icons/${id}.png`);
}
