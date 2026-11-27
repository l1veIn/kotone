# Kotone 插件标准 0.1（草案）

给插件作者看：在任何仓库、用任何语言写一个 Kotone 插件，要照哪些协议、写一份什么样的清单。

- 状态：**0.1 草案**（2026-11-28）。0.x 期间可能有破坏性的改动，每次都写进本文的[变更记录](#十变更记录)，Kotone 照样读上一个 0.x（一版过渡）。1.0 的条件见 [ADR 047](adr/047-plugin-standard.md) D6。
- 依据：[ADR 047](adr/047-plugin-standard.md)（两层、只加不改、第三方事件）、[ADR 043](adr/043-plugin-shape-and-pushes.md)（风险弹窗、推送不给第三方）、[MCP 客户端调研](research/2026-11-24-mcp-client.md)。
- 叫法照 [ADR 039](adr/039-desktop-companion-and-game-apps.md)：**本体**（Kotone 自己）、**插件**、**她**（琴音，玩家的角色）。代码里把第三方插件叫“远程插件”（`crates/kotone-remote-plugin`）。

## 一、两层：只有外层叫“标准”

| 层 | 是什么 | 稳不稳 |
| --- | --- | --- |
| **内层：内置插件** | 主仓库 `plugins/<id>/`，Rust crate，依赖 `kotone-plugin-api`，和本体一起编译、一起发布；清单是 `plugin.toml` | **不是标准**。`kotone-plugin-api` 是内部接口，随本体一起改；不发 crates.io、不给第三方。`plugin.toml` 照旧 `deny_unknown_fields`（一起编译，拼错应该当场报错） |
| **外层：本标准** | 一个**独立进程**的插件说的协议：MCP（工具、资源）+ MCP Apps（页面）+ MCP Events（事件）+ 一份 Kotone 清单（第三节） | 单向门：只加不改（第五节）。别的仓库里的插件只走这一层，不链接 `kotone-plugin-api` |

内置插件对外的那一面（比如英雄联盟单独发成 MCP 服务器 + App）也算外层，照本标准。

为什么分两层：第三方一旦依赖就收不回来，所以外层要小、要慢；内层在同一个仓库里一起改，快改不伤人（[原则 11](philosophy.md)）。DSH、Cortico 的插件跟着宿主版本碎成几批，原因是把进程内的代码接口当成了对外接口（ADR 047 背景）。

## 二、外层协议

Kotone 自己只定第三节那份清单，不另定传输、工具格式、页面桥。

### 2.1 MCP：工具和资源

- 照 [MCP 规范](https://modelcontextprotocol.io/specification)。传输两种：**子进程（stdio）**、**Streamable HTTP**。旧的 HTTP+SSE 不支持。
- 协议版本：**2025-11-25**（有 `initialize` 握手）和 **2026-07-28**（无状态，先 `server/discover`）。差别由宿主吸收，插件作者不用跟着动。
  - 现状（如实写）：Kotone 的 MCP 客户端（`crates/kotone-mcp`，基于官方 `rmcp` 3.11）现在只发 `initialize`（2025-11-25；服务器只会旧版本的，回它自己的版本也照样用）。只会 2026-07-28 的服务器要等客户端换成 rmcp 的 `Auto` 模式（先 `server/discover`，不认再退回）——那是 `kotone-mcp` 里一行的事，不影响本标准。
  - 清单的拿法不靠握手（第 3.1 节），两版协议下一样。测试里用 rmcp 的 `Discover` 模式（2026-07-28）连过同样的服务器，`resources/read kotone://manifest` 照样读到（`crates/kotone-remote-plugin/src/tests.rs` 的 `the_manifest_reads_the_same_over_2026_07_28`）。
- 玩家怎么加：插件页上粘贴 `mcpServers` 配置，或者从 Claude Desktop、VS Code 导入（`KOTONE_HOME/mcp.json`，格式就是 `mcpServers`）。玩家给的短名就是**插件 id**。
- 插件在 Kotone 里的样子：

| 项 | 怎么做 |
| --- | --- |
| 工具名 | `<插件 id>.<工具原名>`。原名照 MCP 规范：字母、数字、`_`、`-`、`.`，1 到 128 个字；不合格的工具不进 Kotone |
| 结果 | 一律当**不可信的数据**（别人写的字，原则 9）：剥掉看不见的字符、限长；她读过之后再执行命令、改文件会先问 |
| 只读 | 工具标 `annotations.readOnlyHint: true` 才算只读。注解只是提示：玩家**信任**这个插件之前，只读的工具第一次也先问；没标只读的永远第一次先问，可以“总是允许” |
| 给谁 | 只在 Kotone 里用：她、自动化的程序、热键。**不经** `kotone.exe mcp` 转给外部 Agent（隐私） |
| `tools/list_changed` | 支持；从她的下一段对话起生效（一段对话里工具清单不变） |
| `serverInfo` | 名字、标题、图标（只认 `https:` 和 `data:image/`）只在卡片上显示，不进她的系统提示 |
| Key | 配置里写 `${input:名字}`，值存 Windows 凭据库，不进 `mcp.json` |

### 2.2 MCP Apps：页面

- 照 [MCP Apps](https://github.com/modelcontextprotocol/ext-apps)（扩展 `io.modelcontextprotocol/ui`，2026-01-26 稳定版）。页面是 `ui://` 资源，类型 `text/html;profile=mcp-app`，宿主 `resources/read` 取回、放进沙盒 iframe（另一个源）。
- 页面就是 MCP App，**不写进清单**。点开插件卡片：有页面就整块显示它（fullscreen），几页时上面一排切换；没有页面就显示一页工具列表。
- 只给页面用的工具照规范写 `_meta.ui.visibility: ["app"]`：不进她的清单。工具用哪个页面显示结果：`_meta.ui.resourceUri`。
- 页面要连的域名自己声明（`_meta.ui.csp`）。本机和局域网的地址（`127.0.0.1`、`localhost`、`192.168.*`……）一律丢掉：页面不能借 Kotone 碰你电脑上、局域网里的服务。

### 2.3 MCP Events：事件

- 事件照 **MCP Events**（[SEP-3415](https://modelcontextprotocol.io/seps/3415-events-extension)：`events/list`、`poll`、`stream`、`subscribe`）。
- 现状（如实写）：**宿主现在还不支持订阅**。`rmcp` 3.11 没有现成的 Events 客户端，Kotone 不自己造协议。先用**观察项**补上（第 3.4 节）：从只读工具里产生事件，事件名同一套（`<插件 id>.<名字>`）。以后 rmcp 支持了、服务器在能力里声明了事件，Kotone 改成直接订阅，玩家的自动化不用改。
- 不管从哪来，事件都要先在清单的 `events` 里**登记**：没登记的宿主不收（和内置插件的 `[[events]]` 一样）。

## 三、Kotone 清单

一份 JSON，字段名 camelCase（照 MCP 的习惯）。告诉 Kotone：挂在哪个游戏上、会发哪些事件、怎么从只读工具里产生事件、有什么风险要先告诉玩家。没有清单的 MCP 服务器照样是一个插件，只是没有这些。

### 3.1 从哪来

宿主按顺序合成一份“生效的清单”：

| 顺序 | 来源 | 说明 |
| --- | --- | --- |
| 1 | **服务器自己给的** | MCP 资源，URI 固定 **`kotone://manifest`**，`mimeType` 为 `application/json`，文字内容（`text`）是清单。宿主每次连上都直接 `resources/read`：**不依赖** `resources/list` 里列不列它，也不依赖握手（`initialize` / `server/discover`）的返回。读不到（`-32002` 资源不存在、方法不存在、5 秒没回）= 没给，不是错。读到的原文和工具清单一起记在磁盘上，没连上时也能显示 |
| 2 | **Kotone 带的预设** | 服务器自己**没给**时才用（给了但读不出来也算没给）。编进程序（`crates/kotone-remote-plugin/presets/*.json`），按服务器的网址或它自报的名字（`serverInfo.name`）认。给常用的、作者不会来写清单的工具用（第一份是绝区零一条龙，ADR 047 第 2 期） |
| 3 | **玩家本地的** | `<KOTONE_HOME>/plugins/<插件 id>/manifest.json`，**同一个格式**。只**加**事件和观察项（同名的不覆盖，忽略并列进“清单的问题”）；1、2 都没有 `games` 时才用它的 `games`；`standard`、`risk` 只认 1、2 |

- 读回来的文字最大 **64 KiB**，超了当没给，插件页说明。
- 预设的写法 `{ "match": { "url": "…", "name": "…" }, "manifest": { … } }`：`match` 只在 Kotone 内部用，**不属于本标准**。网址比较前先规范化（协议和主机小写、`localhost` 当 `127.0.0.1`、去掉默认端口和末尾的 `/`）。
- `games` 跟着生效的清单：插件出现在这些游戏的配置页上。清单是连上以后才到的；变了的话 Kotone 自己重新登记（不断开连接）。

### 3.2 字段

```json
{
  "standard": "0.1",
  "games": ["zzz"],
  "events": [
    {
      "name": "run_finished",
      "label": "一条龙跑完了",
      "data": { "status": "跑完时的状态", "message": "它说的原话（可能为空）" }
    }
  ],
  "watches": [
    {
      "event": "run_finished",
      "tool": "get_run_status",
      "arguments": {},
      "everySeconds": 30,
      "compare": "changed",
      "field": "status",
      "to": ["finished", "succeeded"]
    }
  ],
  "risk": "一段给玩家看的风险说明（可空）"
}
```

| 字段 | 类型 / 默认 | 说明 |
| --- | --- | --- |
| `standard` | 字符串，缺 = `"0.1"` | 写清单时照的标准版本（最低要求），`主.次`。宿主不认识更高的版本时照样按自己认识的字段读，插件页提示“这个插件按更新的标准写的” |
| `games` | 字符串数组，缺 = `[]` | 游戏 id，和本体的游戏列表（`crates/kotone-core/resources/profiles/*.json` 的 `id`，如 `lol`、`cs2`）同一套。这台电脑上没有的 id 保留、插件页提示。空 = 不挂在哪个游戏上 |
| `events` | 数组，缺 = `[]` | 登记的事件（第 3.3 节） |
| `watches` | 数组，缺 = `[]` | 观察项（第 3.4 节） |
| `risk` | 字符串，缺 = 没有 | 风险说明，最长 2000 字（超出的截掉）。进这个插件的“风险和许可”弹窗（ADR 043：一个插件一个弹窗，不在卡片下、设置页里摆），排在“它是第三方插件”那一段后面；第一次点开插件时弹一次。原样显示，里面的网址不变成链接 |
| `$schema` | 字符串 | 可写，宿主不看（方便编辑器校验） |

清单里**没有**推送字段、宿主能力字段（ADR 047 D3）：推送不给第三方（第四节）；第三方拿到的宿主能力比内置的窄，这一版不加。

### 3.3 事件（`events`）

| 字段 | 类型 / 默认 | 说明 |
| --- | --- | --- |
| `name` | 字符串，必填 | snake_case：小写字母开头，小写字母、数字、下划线，最长 64。宿主里完整的名字是 **`<插件 id>.<name>`**（插件 id 是玩家给的短名，所以作者只写后半段） |
| `label` | 字符串，必填 | 一句中文说明，最长 64 字。玩家在自动化的“当……的时候”里看到它 |
| `data` | 对象，缺 = `{}` | 数据字段（camelCase）→ 一句说明（最长 200 字）。观察项发的事件只带这里登记过的字段 |

同一份清单里重名的事件只留第一个。

### 3.4 观察项（`watches`，ADR 047 D5）

服务器不支持 MCP Events 时，用观察项从**只读工具**里产生事件：隔一阵子调一次工具，比较结果，变了就发。照 Home Assistant 的 REST 传感器、Zapier / n8n 的轮询触发。**每个事件最多一个观察项**（宿主按事件名认观察项，玩家的开关也按它记）。

| 字段 | 类型 / 默认 | 说明 |
| --- | --- | --- |
| `event` | 字符串，必填 | 发哪个事件（`events[].name`；没登记的观察项丢掉、列进问题） |
| `tool` | 字符串，必填 | 服务器上工具的**原名**（不带 `<id>.` 前缀） |
| `arguments` | 对象，缺 = `{}` | 调工具的参数 |
| `everySeconds` | 整数，缺 = 60 | 间隔（秒）。下限：本机（stdio、`127.0.0.1`、`localhost`）**10 秒**，别的网址 **60 秒**；`games` 为空（一直轮询）时也是 60 秒。低于下限按下限 |
| `compare` | `"changed"` / `"new"`，必填 | 两种比较，见下 |
| `field` | 字符串 | `changed` 必填：结果里的字段路径，点分隔（`run.status`、`items.0.state`；数字段在数组里当下标） |
| `to` | 字符串或字符串数组，可缺 | `changed` 用：只在变成这些值之一时发；缺 = 变了就发 |
| `list` | 字符串，缺 = 结果本身 | `new` 用：结果里数组的路径 |
| `id` | 字符串，缺 = `"id"` | `new` 用：数组元素里当 id 的字段 |

- **结果从哪取**：工具结果的 `structuredContent`。服务器给的、玩家加的观察项要求工具返回 `structuredContent`，否则这个观察项停下并在卡片上说明；只有 Kotone **预设**里的观察项可以把第一段文字内容当 JSON 解析（很多服务器把 JSON 放在文字里）。
- **changed**（状态变了）：第一次只记底、不发；之后值变了（且满足 `to`）才发，`data` = `{ "from": 旧值, "to": 新值 }`，再加结果顶层里、在事件 `data` 里登记过的标量字段。
- **new**（新条目）：第一次只记底、不发；之后按 `id` 去重，新出现的元素每个发一次，`data` = 元素里在事件 `data` 里登记过的字段。记底的 id 最多存 500 个（旧的丢掉）。
- **什么时候查**：插件开着、玩家信任过这个插件、**这个观察项玩家打开过**（插件页“事件”一块的开关；服务器给的和预设的默认关，第一次打开要确认；玩家自己加的加完即开）；工具要么标了只读，要么玩家在确认里明确同意。`games` 非空时只在其中一个游戏开着时查（游戏退出后再查最后一次，宽限 3 分钟：很多工具跑完会关游戏）。
- **失败**：退避（间隔 ×2，最多 10 分钟）；连续失败 5 次停下，插件卡片上写原因，玩家点“重试”或改了设置再开始。
- 事件内容按不可信处理：字符串剥掉看不见的字符，单个字符串截到 500 字。

> 0.1 现状：清单里的观察项宿主已经会读、会检查，**轮询还没上**（ADR 047 第 2 期）。

### 3.5 `_meta`

- 清单的任何一层都可以有 `_meta`（照 MCP）。宿主不认识的 `_meta` 键一律忽略。
- Kotone 的前缀是 **`com.l1vein.kotone/`**（和应用标识 `com.l1vein.kotone` 一致，照 MCP 的反向域名约定），**保留给以后的工具级提示**。0.1 里没有字段用到它。

### 3.6 宿主怎么读（作者要知道的）

| 情况 | 宿主怎么做 | 插件页上 |
| --- | --- | --- |
| 不认识的字段（任何一层） | 忽略 | 列出路径：“这份清单里 Kotone 不认识的字段：`watches[0].retry`、`colour`（插件按更新的标准写的，或者拼错了）” |
| 字段名以 `experimental` 开头 | 忽略 | 不列（第五节第 4 条） |
| 类型不对 | 当缺（用默认值） | 列进“清单的问题” |
| 不合格的条目（事件名不是 snake_case、`label` 太长、观察项缺 `tool`……） | 丢掉这一条，别的照常 | 列进“清单的问题” |
| 整份不是 JSON 对象、不是合法 JSON、超过 64 KiB | 当没给清单（服务器的读不出来时用预设） | 说一句 |
| `standard` 比宿主新 | 照样按认识的字段读 | “这个插件按更新的标准写的” |

这些提示是小字，在插件「设置」的“Kotone 清单”一节，给作者看。

## 四、第三方事件只进自动化；推送不给第三方

- 第三方插件的事件**只进自动化**：经游戏中心发给自动化的“当……的时候”。不直接进她的上下文（`game.state` 的最近事件里也不放第三方事件），不推送。
- 推送（聊天里的卡片、悬浮窗的一行、她说一句）**不给第三方插件**（ADR 043 D11 和“不做”）。玩家想要，就在自动化的程序里写：`push()` 推一条、`ask()` 让她看着办、`say()` 说一句（ADR 045、046）。
- 为什么：
  1. 不叫模型就往聊天和她的上下文里塞东西，是别人写的字直接进她的脑子（原则 9：别人写的东西是数据，不是指令）。经过玩家自己写的自动化，“推不推、说什么”是玩家定的。
  2. “事件先过自动化”这一层正是 Kotone 比 Cortico 多的那层（[2026-11-27 调研](research/2026-11-27-dsh-and-game-plugins.md)第九节）。
  3. 给第三方的权力是单向门：以后放宽（比如允许某类事件进她的上下文）可以，收紧很难（ADR 047“有多深远”）。

## 五、规矩：只加不改（ADR 047 D4，照原则 19）

1. **只加不改**：已有字段不改名、不改含义、不加旧宿主不认识的取值；新能力用新字段。**不做纯改名**（Cortico 5 → 6 的教训）。
2. **宿主读第三方清单**：不认识的字段忽略，插件页列出来提示作者；缺的字段用默认值（第 3.6 节）。内置插件的 `plugin.toml` 照旧 `deny_unknown_fields`。
3. **按能力协商，不靠一个总版本号**：插件用到宿主没有的能力时，只那一项不可用并说明，插件其余部分照常。
4. **新东西先放进 `experimental`**：字段名带这个前缀，只给内置插件或打开开发者开关的玩家；至少过一个正式版、有内置插件真用过，再转正。0.1 里宿主还没有任何实验字段。
5. **真要改含义，学 Android 的 `targetSdkVersion`**：只对 `standard` 写了新版本的插件生效，旧插件照旧。
6. **弃用**：标出来以后至少保留到下一个大版本；宿主带上一个大版本的过渡层（照 Cortico）；插件页提示作者。
7. **兼容测试 + 检查命令**：CI 里固定放一组“老插件”（每个标准版本一份最小插件，加一个真实插件的快照），每次改宿主都跑；作者发布前自己跑检查命令（第七节）。

照谁：Linux“不破坏用户空间”、VS Code 的 stable / proposed API 和 `engines.vscode`、Android `targetSdkVersion`、MCP 的能力协商、Cortico 2026-10-16 以后的过渡层和 `pnpm check:extension`（出处见 ADR 047“参考了谁”）。

## 六、一个最小例子

一个只读工具、一个事件、一个观察项。任何能写 MCP 服务器的语言都行（官方 SDK：TypeScript、Python、Rust、C#、Go……），下面是伪代码。

```text
server = McpServer(name = "my-farm", version = "1.0.0")

# 只读工具：返回 structuredContent（观察项要它）
server.tool("farm_status",
    annotations = { readOnlyHint: true },
    inputSchema = { type: "object" },
    handler = () => {
        content: [{ type: "text", text: "跑完了" }],
        structuredContent: { status: "finished", message: "今天的体力用完了" }
    })

# Kotone 清单：一个资源，不用列进 resources/list
server.resource("kotone://manifest", mimeType = "application/json",
    read = () => file("kotone-manifest.json"))

server.serve_stdio()
```

`kotone-manifest.json`：

```json
{
  "standard": "0.1",
  "games": ["genshin"],
  "events": [
    {
      "name": "farm_finished",
      "label": "自动刷本跑完了",
      "data": { "message": "它说的原话" }
    }
  ],
  "watches": [
    {
      "event": "farm_finished",
      "tool": "farm_status",
      "everySeconds": 30,
      "compare": "changed",
      "field": "status",
      "to": "finished"
    }
  ],
  "risk": "它会替你操作游戏窗口：跑的时候别动鼠标键盘。"
}
```

玩家加进来、起名 `farm` 以后：工具是 `farm.farm_status`；事件是 `farm.farm_finished`，在自动化的“当……的时候”里选；`status` 变成 `finished` 时发一次，`data` 是 `{ "from": "running", "to": "finished", "message": "今天的体力用完了" }`。

## 七、发布前跑检查命令

> 占位：**第 4 期提供 `kotone-cli plugin check`**（照 Cortico 的 `pnpm check:extension`、MCP Inspector）：连上你的服务器，读 `kotone://manifest`，列出不认识的字段和清单的问题，试调观察项用的工具、看结果里有没有 `structuredContent` 和 `field` 指的字段。

在那之前：把插件加进 Kotone，打开它的「设置」，看“Kotone 清单”一节有没有提示。

## 八、别的仓库的插件

- [l1veIn/kotone-plugin-steam](https://github.com/l1veIn/kotone-plugin-steam)（**建设中**，第 4 期）：只靠本标准、在别的仓库里跑通的第一个插件。

## 九、Kotone 里的代码在哪

| 要看的 | 在哪 |
| --- | --- |
| 读清单、合成、预设、观察项的结构 | `crates/kotone-remote-plugin/src/manifest.rs` |
| 连上时读 `kotone://manifest`、记在磁盘上 | `crates/kotone-remote-plugin/src/server.rs`（`remote-cache.json`） |
| 预设 | `crates/kotone-remote-plugin/presets/` |
| 插件页要的样子 | `apps/desktop/src-tauri/src/remote_plugins.rs`（`describe`） |
| 插件页 | `apps/desktop/src/lib/components/plugins/RemotePluginSettings.svelte`、`PluginRiskDialog.svelte` |

## 十、变更记录

| 版本 | 日期 | 改了什么 |
| --- | --- | --- |
| 0.1 | 2026-11-28 | 草案。两层；外层协议（MCP 工具和资源、MCP Apps、MCP Events 先用观察项补）；清单字段 `standard`、`games`、`events`、`watches`、`risk`；清单从哪来（`kotone://manifest`、Kotone 带的预设、玩家本地的 `manifest.json`）；`_meta` 前缀 `com.l1vein.kotone/` 保留；只加不改的规矩 |
