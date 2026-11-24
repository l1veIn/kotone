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
| 用录下来的真实数据离线跑三个工具的文字 | `examples/render_probe.rs` |

## 数据从哪来

| 数据开关 | 读什么 | 默认 |
| --- | --- | --- |
| `client` 读客户端数据 | 英雄联盟客户端的本机接口（LCU，端口和令牌从 `LeagueClientUx.exe` 的命令行读，不用管理员）和局内接口（Live Client Data，`127.0.0.1:2999`）。只读，只连 127.0.0.1 | 开 |

设置在 `config.json` 的 `plugins.lol`（`enabled`、`sources.client`）。以前的 `lol.enabled` 读进来时搬到 `plugins.lol.sources.client`。

## 风险

国服用这类功能可能违反腾讯的用户协议，LeagueAkari 等工具有封号、禁言的报告；Riot 25.17 起限制第三方的技能冷却计时。默认开，界面上写着风险，用不用由玩家决定。

**不做**：读游戏内存、注入。只用客户端和游戏自己提供的本机接口。

## 照着谁做

[LeagueAkari](https://github.com/LeagueAkari/LeagueAkari)（MIT 的 `dev` 分支，不拷 v1.3.x–v1.4.3 的 GPL 版本）：读命令行、接口路径、模式急速、开黑分组。突破口的算法是自己写的。

## 测试

```powershell
cargo test -p kotone-plugin-lol
python -X utf8 -I scripts/manual-e2e/lol_tools_real.py --while-playing   # 假客户端 + 真 DeepSeek，不开游戏
python -X utf8 -I scripts/manual-e2e/game_events_real.py                 # 假客户端按时间线放一局
```
