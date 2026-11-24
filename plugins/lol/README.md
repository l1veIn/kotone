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
| 页面的数据（这局、战绩、单局详情；图只给 id 和客户端路径） | `src/pages.rs` |
| 素材层：页面要的图（英雄、召唤师头像、段位、海克斯、装备、技能、符文），见下面“素材” | `src/client/assets.rs` |
| 用录下来的真实数据离线跑三个工具的文字；真接口跑一遍统计 | `examples/render_probe.rs`、`examples/stats_probe.rs` |

## 数据从哪来

| 数据开关 | 读什么 | 默认 |
| --- | --- | --- |
| `client` 读客户端数据 | 英雄联盟客户端的本机接口（LCU，端口和令牌从 `LeagueClientUx.exe` 的命令行读，不用管理员）和局内接口（Live Client Data，`127.0.0.1:2999`）。只读，只连 127.0.0.1 | 开 |
| `stats` 用网上的统计 | 腾讯 101 / 掌盟（`mlol.qt.qq.com`，峡谷强度、克制、出装、符文、加点，海斗英雄榜和海克斯榜）；[czgreat/aramhex-data](https://github.com/czgreat/aramhex-data)（海斗英雄 × 海克斯胜率，**CC BY-NC 4.0**，运行时下载、页面上署名、不能商用）；腾讯 `game.gtimg.cn` 的英雄外号和装备、符文、召唤师技能名字。按需请求，缓存在插件目录 | 开 |
| `opgg` 用 OP.GG 的数据 | OP.GG 官方 MCP（`mcp-api.op.gg/mcp`），只在打开英雄页、她查一个英雄时请求，存 24 小时，页面上写 OP.GG 的出处。等 OP.GG 回复 [issue #26](https://github.com/opgginc/opgg-mcp/issues/26) | **关** |

设置在 `config.json` 的 `plugins.lol`（`enabled`、`sources.client`）。以前的 `lol.enabled` 读进来时搬到 `plugins.lol.sources.client`。

## 素材（图片）

安装包里不带图（ADR 039 D6，[对标调研](../../docs/research/2026-11-25-lol-app-parity.md)第三节）。页面数据里只有 id 和客户端的资源路径（例如 `/lol-game-data/assets/v1/champion-icons/103.png`），页面自己决定从哪取：

- Kotone 里：经回环代理交给 `src/client/assets.rs`，顺序是磁盘缓存（插件目录下的 `asset-cache/`，上限 64 MB，超了删最久没用的，14 天重新取）→ 客户端（开着、而且“读客户端数据”开着时，带本机令牌，只读）→ CommunityDragon → 页面显示占位。
- 只放行 `/lol-game-data/assets/`、`/fe/lol-static-assets/`、`/fe/lol-shared-components/` 三个前缀下的图片（png / jpg / svg / webp，按内容认），路径里有 `..`、`%`、查询串的一律拒绝。不把客户端的别的接口转给页面：页面在沙盒里，转了等于让页面能操作客户端。
- 别的宿主（Claude、Codex）里：页面直接换成 CommunityDragon 的地址。
- 测试：`KOTONE_LOL_CDRAGON=<本机地址>` 把 CommunityDragon 换成本机的假服务器（只认本机，写错了就不连网）。

**CommunityDragon**（https://raw.communitydragon.org ，社区维护的客户端资源镜像）：客户端路径换成它的地址的规则照它的目录结构（`/lol-game-data/assets/…` → `plugins/rcp-be-lol-game-data/global/default/…`，后面全部小写）。只请求公开的图片路径，不带玩家的任何数据。

**拳头的声明**（Riot Games “Legal Jibber Jabber”，https://www.riotgames.com/en/legal ）：Kotone isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or anyone officially involved in producing or managing Riot Games properties. Riot Games, and all associated properties are trademarks or registered trademarks of Riot Games, Inc.

英雄、召唤师头像、段位、海克斯、装备、技能、符文的图属于拳头；Kotone 用了这些素材，拳头不认可、也不赞助 Kotone。同一份政策写着：在应用里用拳头的 IP 要有书面许可或有效的 API Key，Kotone 两样都没有（LeagueAkari、AramHex 也没有）。这一条写进英雄联盟 App 的风险提示，只提示，不拦着。不用拳头的商标和 logo（ADR 039 D5、D8）。

## 风险

国服用这类功能可能违反腾讯的用户协议，LeagueAkari 等工具有封号、禁言的报告；Riot 25.17 起限制第三方的技能冷却计时。默认开，界面上写着风险，用不用由玩家决定。

**不做**：读游戏内存、注入。只用客户端和游戏自己提供的本机接口。

## 在 Claude Desktop、Codex 里用

英雄联盟插件本身就是一个独立的 MCP 服务器 + MCP App（ADR 039 D5）：`kotone.exe mcp` 是标准的 stdio MCP 服务器，Claude Desktop、Codex 这类宿主直接连它就能调英雄联盟的工具；会显示 MCP App 的宿主（Claude、ChatGPT、VS Code、Goose 等）还会把结果画成英雄联盟 App 的页面，和 Kotone 里显示的是同一份。

**前提**：Kotone 在跑，而且开着工具服务（默认关：让外部 Agent 先调 `kotone_launch`，它会打开工具服务、在后台启动 Kotone；或者自己退出 Kotone → `kotone.exe config set toolService.enabled true` → 重新启动，见 [工具服务手册](../../docs/tool-service.md)第二节）。`kotone.exe mcp` 自己不带业务逻辑，每次调用都转给运行中的 Kotone：Kotone 里关掉英雄联盟插件或者某个数据开关，对应的工具和页面也就没了。

Claude Desktop：设置 → 开发者 → 编辑配置，打开 `%APPDATA%\Claude\claude_desktop_config.json`，加上：

```json
{
  "mcpServers": {
    "kotone": {
      "command": "C:\\Users\\<你的用户名>\\AppData\\Local\\Kotone\\kotone.exe",
      "args": ["mcp"]
    }
  }
}
```

Codex：`%USERPROFILE%\.codex\config.toml` 里加上：

```toml
[mcp_servers.kotone]
command = 'C:\Users\<你的用户名>\AppData\Local\Kotone\kotone.exe'
args = ["mcp"]
```

`command` 写 `kotone.exe` 的完整路径（安装包默认装在 `%LOCALAPPDATA%\Kotone\`；JSON 里的反斜杠要写两个，TOML 用单引号就不用）。改完重启 Claude Desktop / Codex。

**能用的工具**（对外的名字是 Kotone 工具名的点换成下划线、前面加 `kotone_`；只给页面按钮用的标 `visibility: ["app"]`，宿主不交给模型）：

| 工具 | 做什么 | 页面 | 要开的数据开关 |
| --- | --- | --- | --- |
| `kotone_lol_scout` | 这局 10 个人的战绩和突破口 | `ui://kotone/lol/match` 这局（选人、加载、对局中、结算） | 读客户端数据 |
| `kotone_lol_tier` | 强度榜（峡谷按位置、海斗） | `ui://kotone/lol/tier` 强度榜 | 用网上的统计 |
| `kotone_lol_match` | 这局的样子、谁的召唤师技能没好 | — | 读客户端数据 |
| `kotone_lol_spell` | 记对面交的召唤师技能 | — | 读客户端数据 |
| `kotone_lol_player` | 任何人的战绩 | `ui://kotone/lol/player`（还在 Kotone 里，见下） | 读客户端数据 |
| `kotone_lol_champion` | 英雄的强度、克制、出装、海斗海克斯 | `ui://kotone/lol/champion`（同上） | 用网上的统计 |
| `kotone_lol_client` | 操作客户端：接受对局、选 / 禁英雄 | — | 操作客户端（**默认关**） |
| `kotone_lol_game`、`kotone_lol_champions` | 只给页面用：单局详情、英雄列表 | — | 同上 |

页面一页页从 Kotone 里的 Svelte 换成 App，以 `src/app.rs` 的 `APP_PAGES` 为准：是 App 的，工具带 `_meta.ui.resourceUri`、`resources/list` 里有它；还不是的，外部宿主里这个工具只给文字。宿主不支持 MCP Apps 的（例如终端里的 Codex CLI）照样能用，只拿给模型的那几行字。图片在别的宿主里直接从 CommunityDragon 取（域名写在资源的 CSP 声明里），不经 Kotone。

**风险提示**（跟着服务器说明发：`initialize` 的 `instructions` 里有同样的话，宿主会交给模型）：读本机客户端和对局的接口可能违反腾讯（国服）和拳头（Riot）的用户协议，LeagueAkari 等工具有封号、禁言的报告；外服有人用了自动接受、自动选英雄后被永久封号（[LeagueAkari #523](https://github.com/LeagueAkari/LeagueAkari/issues/523)）。用不用由玩家决定，只提示、不拦着。操作客户端默认关，要玩家自己在 Kotone 的英雄联盟插件设置里打开。图片属于拳头，Kotone 没有拳头的许可（见上面“素材”）。

**怎么验的**：`scripts/manual-e2e/mcp_app_external_real.py` 用 MCP Apps 官方的参考宿主（`modelcontextprotocol/ext-apps` 的 `examples/basic-host`）经一个 stdio ↔ Streamable HTTP 的桥连 `kotone.exe mcp`，在里面调 `kotone_lol_tier` 和 `kotone_lol_scout`（假客户端），看 App 在它的沙盒里画出来、图从 CommunityDragon 来。Claude Desktop 里的实际效果还没试。

## 照着谁做

[LeagueAkari](https://github.com/LeagueAkari/LeagueAkari)（MIT 的 `dev` 分支，不拷 v1.3.x–v1.4.3 的 GPL 版本）：读命令行、接口路径、模式急速、开黑分组、腾讯 101 的解析。突破口的算法是自己写的。[aramhex](https://github.com/czgreat/aramhex)（代码 MIT）：覆盖层怎么显示、显示性能怎么排查（只在需要时显示、窗口小、识别线程低优先级、截图时不认自己画的）。

## 测试

```powershell
cargo test -p kotone-plugin-lol
# 录下来的一局 → 给她的字，加上三个页面的数据样例（去掉玩家名和 puuid）
cargo run -p kotone-plugin-lol --example render_probe -- target/lol-probe/20261123-210745 target/lol-probe/gamedata target/plugin-shots/round7
python -X utf8 -I scripts/manual-e2e/lol_tools_real.py --while-playing   # 假客户端 + 真 DeepSeek，不开游戏
python -X utf8 -I scripts/manual-e2e/game_events_real.py                 # 假客户端按时间线放一局
```
