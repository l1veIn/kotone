# 2026-11-23 英雄联盟数据来源调研：BP 胜率、克制、出装、海斗海克斯（ADR 035“以后”的依据）

起因：玩家说开源社区有插件能给出 OP.GG 的数据（选英雄时看胜率、按自己的英雄池选克制），也有开源的海斗项目能查各英雄的海克斯数据，“这些也可以作为各种 tool 帮助我们的 kotone 理解比赛阅读游戏给出建议”。

这里记 2026-11-23 核对过的事实，接口都在这台电脑上实际请求过；**所有站点的使用条款都没能确认允许程序调用**，这是最大的未决问题。

## 一、选英雄辅助工具都从哪拿数据

| 项目 | 许可 · 星 · 最后提交 | 数据从哪来 | 现在还能用吗 |
| --- | --- | --- | --- |
| [LeagueAkari](https://github.com/LeagueAkari/LeagueAkari) | MIT · 4.8k · 2026-11-22 | 两个可选：OP.GG 内部 JSON `lol-api-champion.op.gg`（[opgg](https://github.com/LeagueAkari/LeagueAkari/blob/main/src/shared/http-api-axios-helper/opgg/index.ts)），腾讯 `mlol.qt.qq.com`（[qq101](https://github.com/LeagueAkari/LeagueAkari/blob/main/src/shared/http-api-axios-helper/qq101/index.ts)） | 两个今天都有返回 |
| [Frank](https://github.com/SYJun404/frank) | MIT · 1.9k · 2026-08-16 | 腾讯静态文件 `lol.qq.com/act/lbp/common/guides/champDetail/champDetail_{id}.js`，加 lol.ps 的强度榜 | 腾讯文件是最新的（16.23，2026-11-22） |
| [ChampR](https://github.com/cangzhang/ChampR) | LGPL-2.1 · 1.7k · 2026-04-17 | 自己发的 npm 包 `@champ-r/<来源>` | **数据停了**：`@champ-r/op.gg` 最后一次发布是 2024-03（14.5） |
| [Seraphine](https://github.com/Zzaphkiel/Seraphine) | 无许可证 · 3.1k | OP.GG 内部 JSON | 2025-03 归档 |
| [Glyph](https://github.com/SirWinstonChurchillTheFirst/Glyph) | GPL-3.0 | OP.GG 的 MCP 服务，缓存 24 小时 | 作者说 OP.GG 同意了（见第二节） |
| [draftgap](https://github.com/vigovlugt/draftgap) | MIT · 70 · 2026-11-21 | Lolalytics `a1.lolalytics.com/mega/?ep=…` | 活跃 |
| [uggo](https://github.com/kade-robertson/uggo) | MIT · 9 · 2026-11-23 | U.GG `stats2.u.gg/lol/1.5/...` | 活跃 |
| [league-assistant-cn](https://github.com/pearljamx/league-assistant-cn) | MIT · 0 · 2026-11-21 | 腾讯 mlol、AramHex、aramgg 的收费接口 `data.dtodo.cn` | 国服数据源最全的一份清单（[providers](https://github.com/pearljamx/league-assistant-cn/tree/main/src/main/providers)） |

## 二、OP.GG 官方的 MCP 服务

- [opgginc/opgg-mcp](https://github.com/opgginc/opgg-mcp)（MIT，2026-11-19）只是一个转发；真正的工具在 OP.GG 服务器上，不开源。
- 地址 `https://mcp-api.op.gg/mcp`（Streamable HTTP），**不用 Key**，今天实际握手、调用都通了；要求带一个 `client_app` 名字；没看到限流头。
- 服务一共列出 30 个工具，和我们有关的：
  - `lol_get_champion_analysis`：模式有排位、灵活、无限火力、大乱斗、极限闪击；返回胜率、选取率、禁用率、梯队、出装、符文、加点、召唤师技能、克制（`weak_counters` / `strong_counters`）和搭配；
  - `lol_list_lane_meta_champions`：各路的排位强度榜；
  - `lol_get_lane_matchup_guide`：对线指南，几个数据源里只有它有文字的对线技巧；
  - `lol_get_champion_synergies`：英雄搭配；
  - **`lol_list_aram_augments`**：每个英雄的海斗海克斯，带梯队、表现分和热门度，只返回 3 级以上的。
- **实测**：
  - 艾瑞莉娅的大乱斗数据，`lang=zh_CN`，返回 16.23 版本，约 1 秒；
  - 中文名用的是称号（例如“熔岩巨兽”），和国服一样；
  - 海克斯有 122 行，有的名字是 null，但 id 能在 `cherry-augments.json` 里查到；
  - 返回的不是 JSON，是一种紧凑的类声明文本（[#23](https://github.com/opgginc/opgg-mcp/issues/23) 说格式不统一）。
- **缺的**：
  - 查召唤师不支持 `CN`；
  - 没有海克斯大乱斗这个模式，所以拿不到海斗的英雄梯队；
  - 所有数据都是全球的，不是国服的。
- **条款**：网站协议（[2026-09-14](https://op.gg/lol/policies/agreement)）禁止“自动化脚本”和抓取，只许个人非商业使用，没提 MCP。
  - Glyph 的作者[问过 OP.GG](https://github.com/opgginc/opgg-mcp/issues/17)，说得到了同意，并加了 OP.GG 的标志；OP.GG 的回复不公开，无法核实。
  - 另一个免费桌面应用提的同样问题 [#24](https://github.com/opgginc/opgg-mcp/issues/24) 还开着。
- 在大陆网络能不能连：没核实（在 Cloudflare 后面）。

## 三、其他非官方的统计接口（都返回 JSON，都没有文档，条款都没核实）

- **OP.GG 内部 JSON**（协议禁止脚本访问）：
  - [`/api/contents/tiers?type=aram_mayhem`](https://lol-api-champion.op.gg/api/contents/tiers?type=aram_mayhem)：海斗英雄梯队；
  - `/api/contents/stats/champions/39/aram-augments`；
  - `/api/global/champions/ranked/39/top`。
- **U.GG**：
  - [版本表](https://static.bigbrain.gg/assets/lol/riot_patch_update/prod/ugg/ugg-api-versions.json)给出每个版本对应的文件版本；
  - `stats2.u.gg/lol/1.5/overview/16_23/normal_aram/39/1.5.0.json` 6.6 KB，排位的那份 433 KB。
- **Lolalytics**：`a1.lolalytics.com/mega/?ep=build-team&…`。

## 四、海斗（海克斯大乱斗）的海克斯数据

- **[czgreat/aramhex-data](https://github.com/czgreat/aramhex-data)**：最合适。
  - **只有国服数据**：七个大区都有，16.23 普通规则 499 万局，快照时间 2026-11-20 08:00（北京时间）。活动规则（例如“首个海克斯是棱彩”）单独放，早些版本还按分段分开。
  - 文件：`heroes.json`，`hero_augments.json`（英雄 × 海克斯的局数、胜场、胜率、`low_sample`），`augments.json`，`hero_spells.json`，`core_builds.json`，装备表。
  - 自带给程序和 AI 看的 [AGENTS.md](https://github.com/czgreat/aramhex-data/blob/main/AGENTS.md)：先读 `latest.json`，校验 SHA-256，小样本按 k≈300 平滑。`latest.json` 在 GitHub raw 和 jsDelivr 上都有返回。
  - **许可 CC BY-NC 4.0**：要署名、不能商用。Kotone 不收费，可以用；运行时下载并注明出处，不打包进 MIT 仓库。
  - 数据怎么采的没写（未核实）。配套应用 [aramhex](https://github.com/czgreat/aramhex)（MIT）只靠截图识别；网站 [aramhex.com/data](https://aramhex.com/data/)。
- **腾讯官方的海斗统计**（国服，每天更新，今天 20261122 那一版有返回）：
  - `mlol.qt.qq.com/go/battle_info/odp_proxy/fuwen_hero_rank?championid=39`：这个英雄的海克斯按稀有度排名、S / A 评级，加上鞋、装备、加点；
  - `fuwen_aram_rune_rank_v2?augmentid_level=255`：全部海克斯；
  - `fuwen_aram_hero_rank_v2`：英雄排名；
  - `fuwen_aram_hero_parttner`：英雄搭配。
  - 接口名来自 LeagueAkari 和 league-assistant-cn 的源码。
- **只参考**：[DeepARAM](https://github.com/nicothomas02/DeepARAM)、[mayhem-advisor](https://github.com/ItzStorDz/mayhem-advisor)，都至少部分用 AramHex 的数据。
- **游戏文件**：海斗的海克斯在中文 [`cherry-augments.json`](https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/zh_cn/v1/cherry-augments.json) 里（600 条，图标在 `UX/Kiwi/`），客户端本地也有同一份；OP.GG 的海克斯 id 和它一致（3118 奥术公理、1392 神圣雪球）。海克斯套装的加成在哪没查（未核实）。

## 五、静态资料

- 客户端本地 `/lol-game-data/assets/v1/…` 已经够用（ADR 035）。
- 中文 Data Dragon（[16.23.1](https://ddragon.leagueoflegends.com/cdn/16.23.1/data/zh_CN/champion.json)）里称号和名字是反的：艾瑞莉娅的 `name` 是“刀锋舞者”，`title` 是“艾瑞莉娅”。
- **外号**：腾讯的 [`hero_list.js`](https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js)（16.23，2026-11-18 生成）有 `keywords` 字段，里面是外号和拼音，例如“女刀, nd, airuiliya”。玩家说外号（“刀妹”“女刀”）时用它对上英雄。
- Meraki 已经不更新（英雄最后改于 2025-08，没有中文），不用。

## 六、国服专用的统计（腾讯官方，不用登录）

在这台电脑上测过，没在大陆网络以外测。都返回 JSON（`champDetail` 是包在 JS 里的 JSON），约 0.15–0.2 秒，没有文档，条款没核实。

- 版本列表：`mlol.qt.qq.com/go/database/versionlist?zone=lol&from=h5`。
- **峡谷**：`odp_proxy/lol_101strategy`（强度榜），加 `_confront`（克制）、`_build`、`_runeinfo`、`_skill_point`、`_skill`（召唤师技能）、`_partner`（搭配）、`_trend`。
  - 参数 `itier=255`（全部段位）、`version_id=16.23`、`lane=TOP`（大写）。小写或数字版本号会返回空。
  - 克制接口返回了艾瑞莉娅上路最难打和最好打的各 5 个。
- **普通大乱斗**：`aram_hero_overview?dtstatdate=YYYYMMDD`（每天一份）。
- **每个英雄的静态文件**：[`champDetail_39.js`](https://lol.qq.com/act/lbp/common/guides/champDetail/champDetail_39.js)，有 `championLane`、`championTrend`、`championFight`（对位）。

## 建议（按优先级）

1. **腾讯 mlol / 101** → `lol.champion_stats`、`lol.counters`、`lol.build`：国服的峡谷、大乱斗、海斗都在这里，不用 Key，快，服务器在国内。风险是没有文档、随时可能改，照 LeagueAkari 那样容错解析。
2. **czgreat/aramhex-data** → `lol.augments`：国服海斗的英雄 × 海克斯胜率，有版本、有校验，专门写给 AI 读；界面上署名，守 CC BY-NC。
3. **OP.GG MCP** → 全球数据的备用，以及 `lol.matchup_guide`（只有它有文字的对线指南和搭配分）。先照 Glyph 的做法问 OP.GG 同意、显示标志。
4. **客户端 `/lol-game-data` + `hero_list.js` 的外号** → 名字和 id（第 4 条的名字部分 ADR 035 已经在用）。
5. **只参考**：U.GG、Lolalytics、OP.GG 内部 JSON、aramgg（要付费 Key）、ChampR（数据停了）。
