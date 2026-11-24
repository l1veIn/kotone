# 英雄联盟插件（`lol`）

像 LeagueAkari 那样查这局 10 个人的战绩、找对面的突破口、记对面交的召唤师技能；给游戏中心报英雄联盟的对局阶段和局内的事。
决定和实测见 [ADR 035](../../docs/adr/035-lol-tools.md)（工具）、[ADR 036](../../docs/adr/036-game-state-and-events.md)（游戏中心）、[ADR 038](../../docs/adr/038-kotone-and-apps.md)（插件），计划见 [插件重构计划](../../docs/plugin-refactor.md)。

## 有什么

| 东西 | 在哪 |
| --- | --- |
| 清单：名字、挂在游戏配置 `lol` 上、`lol` 适配器、数据开关、页面、给她的两段话 | `plugin.toml` |
| 给她和页面的工具 `lol.match`、`lol.spell`、`lol.scout`（都马上返回，结果按不可信数据给她） | `src/tools.rs` |
| 后台：客户端开着时每 2 秒看一次阶段，进加载界面就查战绩，往游戏中心报阶段和局内的事；工具的实现 | `src/watcher.rs` |
| 客户端和局内接口、游戏资料、战绩、突破口、技能计时、写给她看的文字（纯函数 + 只连 127.0.0.1 的 HTTP） | `src/client/` |
| 页面（插件页、游戏配置页里的一节） | `ui/` |
| 界面文字（`plugin_lol_*`，中文要和 `plugin.toml` 一致，有测试） | `messages/` |
| 页面的测试（桌面端的 Playwright 一起跑） | `e2e/` |
| 网上的统计（腾讯 101、aramhex-data、外号和名字表、OP.GG）和它的工具 `lol.champion` / `lol.tier` | `src/stats/`、`src/stats_tools.rs` |
| 覆盖层：选英雄时的胜率面板、海斗三选一的海克斯评分（照 aramhex） | `src/overlays.rs`、`ui/*Overlay.svelte` |
| 页面的数据（这局、战绩、单局详情） | `src/pages.rs` |
| 用录下来的真实数据离线跑三个工具的文字；真接口跑一遍统计 | `examples/render_probe.rs`、`examples/stats_probe.rs` |

## 数据从哪来

| 数据开关 | 读什么 | 默认 |
| --- | --- | --- |
| `client` 读客户端数据 | 英雄联盟客户端的本机接口（LCU，端口和令牌从 `LeagueClientUx.exe` 的命令行读，不用管理员）和局内接口（Live Client Data，`127.0.0.1:2999`）。只读，只连 127.0.0.1 | 开 |
| `stats` 用网上的统计 | 腾讯 101 / 掌盟（`mlol.qt.qq.com`，峡谷强度、克制、出装、符文、加点，海斗英雄榜和海克斯榜）；[czgreat/aramhex-data](https://github.com/czgreat/aramhex-data)（海斗英雄 × 海克斯胜率，**CC BY-NC 4.0**，运行时下载、页面上署名、不能商用）；腾讯 `game.gtimg.cn` 的英雄外号和装备、符文、召唤师技能名字。按需请求，缓存在插件目录 | 开 |
| `opgg` 用 OP.GG 的数据 | OP.GG 官方 MCP（`mcp-api.op.gg/mcp`），只在打开英雄页、她查一个英雄时请求，存 24 小时，页面上写 OP.GG 的出处。等 OP.GG 回复 [issue #26](https://github.com/opgginc/opgg-mcp/issues/26) | **关** |

设置在 `config.json` 的 `plugins.lol`（`enabled`、`sources.client`）。以前的 `lol.enabled` 读进来时搬到 `plugins.lol.sources.client`。

## 风险

国服用这类功能可能违反腾讯的用户协议，LeagueAkari 等工具有封号、禁言的报告；Riot 25.17 起限制第三方的技能冷却计时。默认开，界面上写着风险，用不用由玩家决定。

**不做**：读游戏内存、注入。只用客户端和游戏自己提供的本机接口。

## 照着谁做

[LeagueAkari](https://github.com/LeagueAkari/LeagueAkari)（MIT 的 `dev` 分支，不拷 v1.3.x–v1.4.3 的 GPL 版本）：读命令行、接口路径、模式急速、开黑分组、腾讯 101 的解析。突破口的算法是自己写的。[aramhex](https://github.com/czgreat/aramhex)（代码 MIT）：覆盖层怎么显示、显示性能怎么排查（只在需要时显示、窗口小、识别线程低优先级、截图时不认自己画的）。

## 测试

```powershell
cargo test -p kotone-plugin-lol
python -X utf8 -I scripts/manual-e2e/lol_tools_real.py --while-playing   # 假客户端 + 真 DeepSeek，不开游戏
python -X utf8 -I scripts/manual-e2e/game_events_real.py                 # 假客户端按时间线放一局
```
