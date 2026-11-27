# CS2 插件

ADR 036 D9、ADR 047 第 3 期（“自己做”这一种）。数据只来自 Valve 官方的游戏状态接口（Game State Integration，GSI）：
不读游戏内存、不注入、不改封包。

## 怎么收数据

- 插件开着时在 `127.0.0.1` 上听一个端口（默认 23730，被占了往后找最多 20 个，找到的存进 `plugins.cs2.settings.port`）。
- 往 CS2 的 cfg 目录（`<安装目录>/game/csgo/cfg`）写 `gamestate_integration_kotone.cfg`：只发到本机这个端口、带随机令牌
  （`plugins.cs2.settings.token`），令牌不对的 POST 回 403 丢掉。默认就写（`writeCfg`，缺省 true）；插件页上的开关关掉、
  或者插件关了，就删掉——只删第一行带 `// Written by Kotone` 的那个。只在 CS2 没开时写、删；同名但不是我们写的文件，写之前
  备份到 `<KOTONE_HOME>/plugins/cs2/backups/`，要删时不碰。
- 找 CS2：`KOTONE_CS2_DIR`（测试用，指向 `target/` 下的假目录）→ 隔离的实例（设了 `KOTONE_HOME`）不找 → Steam 库里的 appid 730。

## 给出去的东西

- 游戏中心适配器 `cs2`：菜单 / 热身（`pre_match`）/ 对局中（含半场换边）/ 结算；30 秒收不到、游戏也不在了就交回。
- 事件 `cs2.*`、推送（下包倒数、炸弹还剩 10 秒、赛点、半场、比赛结束）：见 `plugin.toml`。只说事实，同样的输入同样的输出。
- 工具：`cs2.state`、`cs2.rounds`（给她、马上返回）；`cs2.status`、`cs2.cfg`（只给页面）。

## 自己打时拿不到的

GSI 自己打的时候只给自己的数据（和你在看的队友）：别人的血量、位置、炸弹在哪、`phase_countdowns` 只有观战才有。
所以下包倒数按 40 秒（`mp_c4timer` 默认）从收到“下了”那一刻算；观战时用游戏给的倒数。

## 测试

- `cargo test -p kotone-plugin-cs2`：GSI 解析（`tests/fixtures/` 里是照 Valve 文档字段合成的样例）、令牌、只听 127.0.0.1、
  端口被占换端口、阶段 / 事件 / 推送的判定、cfg 写入 / 删除（`target/tmp/cs2-tests/` 下的假目录）。
- Playwright：`e2e/cs2.spec.ts`（插件页）。
- 真机：`scripts/manual-e2e/cs2_gsi_real.py`（隔离实例 + 假 CS2 目录，按时间线 POST 一局）。
