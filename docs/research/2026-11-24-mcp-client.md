# Kotone 的 MCP 客户端：调研和方案（草案）

日期：2026-11-24。状态：**草案，等玩家看**；没有改代码。相关：[ADR 038](../adr/038-kotone-and-apps.md)（D5、D7 第 6 期、“不做”、“怎么定的”里第三方的方向）、[软件哲学](../philosophy.md)（原则 6–9、11–16）。

玩家的话：

- “事实上我们现在是不是能接入所有的 MCP apps？……开源社区肯定有非常多各种场景的 MCP apps 吧？肯定有适合我们游戏场景的，我们都能无缝接入吗”
- “我个人觉得，类似 LeagueAkari 这样的桌面应用会收敛成 MCP APPS，如果市面上还没有大量出现，那么我来推动这件事”

## 一、要达到什么（玩家看到的）

1. 在“插件”页加一个 MCP 服务器：粘贴配置，或者从 Claude / VS Code 导入，或者拖进一个 `.mcpb` 安装包，以后还能从目录里挑。加好以后它就是插件页的一张卡片：能开关、有风险提示、需要的 Key 在卡片里填。
2. 它的工具她能用（第一次用会问你），它的 MCP App（带界面的）点卡片就在 Kotone 里打开，聊天里也能出它的卡片。
3. 英雄联盟插件反过来也做成一个 MCP 服务器 + App：在 Claude Desktop、Goose、VS Code 里也能打开“这局”“战绩”。这是“推动”的那一半。

## 二、现在的情况

**Kotone**（只读核对，2026-11-24）：

- Kotone 是 MCP **服务器**（`kotone.exe mcp`，`crates/kotone-cli/src/mcp.rs`，手写的 JSON-RPC），不是**客户端**：代码里没有连接外部 MCP 服务器的部分。唯一的例外是英雄联盟插件里为 OP.GG 手写的一小段（`plugins/lol/src/stats/opgg.rs`、`stats/mod.rs` 的 `opgg_post` / `opgg_call`：每次调用发三个请求、没有会话复用、没有 `tools/list`）。
- 插件契约已经照 MCP 写：`ToolSpec` ≈ `tools/list` 的一项加 `_meta.ui`，`ToolResult` ≈ `CallToolResult`（`content` / `structuredContent` / `ui`），可见性 ≈ `_meta.ui.visibility`。挡着第三方的有三处：插件表是编译时的静态表（`apps/desktop/src-tauri/src/plugins.rs`）；页面地址只认 `ui://kotone/…`（`Manifest::owns_uri`、前端 `pluginOfUri`）；主窗口的 CSP 没有 `frame-src`，iframe 和内联脚本都会被挡（`tauri.conf.json`）。
- 插件页的 App 位置已经留好（`PluginsPage.svelte`、`PluginPage.svelte` 的注释），还没有任何 iframe / postMessage。
- **一个现成的 bug**：`kotone.exe mcp` 把 `x.y` 换成 `kotone_x_y`，换回来时以为名字里没有下划线，`voice_chat.say` 会变成 `voice.chat.say`（`mcp.rs` 228–233 行）。外部 Agent 经 MCP 调游戏语音的工具会失败。游戏语音一直这样，不是插件重构带进来的。

**外面**（详细出处见文末）：

- **MCP Apps** 是 MCP 的可选扩展（`io.modelcontextprotocol/ui`），稳定版 2026-01-26：`ui://` 资源是一个 HTML（`text/html;profile=mcp-app`），宿主用 `resources/read` 取回，放进沙盒 iframe；宿主和页面之间是 postMessage 上的 JSON-RPC（`ui/initialize` 握手，宿主把工具的输入和结果推给页面，页面可以调工具、读资源、请求全屏 / 画中画）。页面要连哪些域名得自己声明，宿主按声明拼 CSP，没声明就全拒。
- 在渲染 MCP Apps 的：Claude（网页、桌面、移动）、ChatGPT、Microsoft 365 Copilot、Goose（实验）、VS Code（实验开关）、Postman、MCPJam。有一个 2026-05 的 issue 说 Claude Desktop / claude.ai 渲染不出来，到现在还开着。
- **MCP 2026-07-28** 改动很大：协议变成无状态（去掉 initialize 握手）、订阅改成 `subscriptions/listen`、旧的 HTTP+SSE 正式弃用、OAuth 动态注册改成 CIMD。
- **工具数量**：Anthropic 的文档说超过 30–50 个工具，选对工具的准确率明显下降；VS Code 每次最多 128 个；Claude Code 和 Goose 都用“先只给名字、要用再加载”的工具搜索。
- **游戏**：没找到带界面的游戏 MCP App。纯工具的有：OP.GG 官方（英雄联盟、云顶、无畏契约，27 个工具，不要 Key）、lcu-mcp（读英雄联盟客户端、事件只能轮询，1 星）、读对局数据的、Riot API（要 Key）、几个社区的 Steam、Minecraft（Mineflayer）。LeagueAkari（约 4.8k 星）自己没有对外的 MCP。
- **游戏要的、MCP 没有的**：“往外推事件”已经有定稿的扩展 **SEP-3415 Events**（`events/list`、`poll`、`stream`、`subscribe`，带游标和断档处理，事件内容按不可信处理），但几乎没有宿主实现（只有 ChatGPT 部分支持 webhook）。“盖在全屏游戏上面的浮层”没有任何提案；MCP Apps 只有 inline / fullscreen / pip（宿主窗口里的画中画）。

## 三、照谁做

| 要做的 | 照谁 | 为什么 |
| --- | --- | --- |
| 整体结构 | **Goose** | Rust 核心用官方 `rmcp` 当客户端，桌面端渲染 MCP Apps：Rust 侧用 axum 在本机回环地址起一个代理（独立端口 = 独立源），CSP 在 Rust 侧拼。和 Kotone（Rust + WebView）几乎一样 |
| Rust 库 | **`rmcp`**（官方，Apache-2.0，3.11.0，2026-11-17） | 子进程、Streamable HTTP、OAuth（PKCE、CIMD）都有；目标 2026-07-28，兼容 2025-11-25；迭代快，要锁小版本 |
| 配置格式 | `mcpServers` JSON（Claude Desktop / VS Code / Claude Code 的事实标准） | 能一键“从 Claude 或 VS Code 导入” |
| 一键安装 | Claude Desktop 的 **`.mcpb`** | 它的 `user_config` 能自动生成“只填 Key”的表单，敏感字段存凭据库 |
| 审批 | **Codex** 的 `writes` 模式 + **OpenClaw** 的“总是允许”粒度 | 没标只读的工具第一次问；可以对“这个服务器的这个工具”总是允许；注解（readOnlyHint 等）只改善体验、不当安全边界（规范原话：注解默认不可信） |
| 防注入 | **Hermes** | 子进程只给配置里写的环境变量加一组安全基线；从工具结果和说明里剥掉不可见字符 |
| 超时、输出 | Codex（启动 10 秒、工具 60 秒）、Claude Code（结果上限 25k token，超长的落盘） | |
| 页面桥 | `@modelcontextprotocol/ext-apps` 的 **`AppBridge`**（框架无关，Apache-2.0） | 不引 React 版的 `@mcp-ui/client`；用 ext-apps 自带的示例做对照测试 |

## 四、方案

### 4.1 一个第三方服务器 = 一个插件

- 加一个服务器就生成一个“远程插件”（`RemoteMcpPlugin`，实现现有的 `Plugin`）：清单从配置合成，id 是玩家给的短名，工具名是 `<id>.<原工具名>`（照现有的“插件工具以 `<前缀>.` 开头”）。插件页、开关、她的工具清单、工具服务这些现有的路都不用另开一套（原则 15：一种东西一种登记方式）。
- 插件表从静态表改成加了锁的表（内置的还是编译进来，远程的运行时加入）；`owner`、`offers`、`list` 照旧能用。
- 页面地址放开：远程插件的 `ui://` 不限于 `ui://kotone/`，宿主按“哪个服务器给的”记归属。

### 4.2 连接

- 新 crate `crates/kotone-mcp`：在 `rmcp` 上包一层，管子进程（把 `kotone-exec` 里的 Job Object 代码抽出来复用：Kotone 退出时子进程一起关）和 Streamable HTTP；只做这两种传输（不做旧 SSE）。
- Windows：把 `npx` / `uvx` 解析成真实路径，必要时 `cmd /c` 包一层并注意转义（Rust 1.77.2 起跑 `.cmd` 的参数转义问题，CVE-2024-24576）；Node 是否由 Kotone 自带，看第一批服务器再定（Claude Desktop 自带 Node）。
- 服务器掉线：对话里照常有它的工具（一段对话里工具清单不变，ADR 024），调用时回答“这个插件现在没连上”，不说“没有这个工具”。
- `tools/list_changed`：从下一段对话起生效（照 ADR 024）。

### 4.3 她怎么用

- **主会话用哪些工具只看快不快**（玩家 2026-11-24：“我觉得只要返回够快就可以让主会话使用”；ADR 029 的标准本来就是“一两秒内出结果”）。第三方工具事先不知道快不快，所以先把 ADR 029 写了但**代码里还没做**的那条补上：**主会话里任何工具超过 3 秒没回来，就转给后台**（她说“这个要一会儿，好了告诉你”）。有了这条，第三方工具可以直接进主会话，慢的自动转走；现在手工维护的主会话名单（`MAIN_TOOLS`）和插件的 `instant` 标记也可以跟着简化（原则 2：选能删掉规则的设计）。
- 结果**一律按不可信**处理（ADR 030 的标记：读过之后再执行命令、改文件会先问），在主会话里也一样。
- 工具多了：超过一定数量（先定 20 个左右）就只给后台“按关键词找工具”（`tools.find` 已经有了），不把全部工具塞进清单。
- 只给页面用的工具（`visibility: ["app"]`）不进她的清单（规范要求）。

### 4.4 权限和安全

- 加服务器时弹一次信任提示：它会在你电脑上运行谁写的什么程序、能连哪些网址、要哪些 Key（照 VS Code 和 Claude Desktop）。
- 工具的副作用从注解推出来，**永远不为空**：没标只读的都算“第三方插件的操作”（新加一个副作用分类，默认询问）；标了只读的也只当提示，第一次照样问（服务器还没被你“信任”之前）。你可以对“这个服务器的这个工具”选总是允许。
- 第三方插件拿到的宿主能力比内置的窄（ADR 038“不做”：给插件的权力是单向门）：没有替玩家按键、没有“停下她”、没有免问。第三方工具不能绕过 ADR 032 的输入闸门。
- 子进程只拿配置里写的环境变量；Key 存 Windows 凭据库（`<插件 id>:<名字>`，和现在一样）。
- 加进来的第三方工具**默认不再经 `kotone.exe mcp` 转给外部 Agent**（要不要转见第七节）。

### 4.5 显示别人的 MCP App

- 照 Goose：Rust 侧在本机回环地址起一个小代理（带随机密钥，只认本机），App 放在另一个端口上（= 另一个源，碰不到 Kotone 的页面和 IPC）；CSP 从 `default-src 'none'` 开始，按 App 声明的域名放行。
- 前端：插件页的 App 位置和聊天卡片放一个 `McpAppFrame`，用 `AppBridge` 握手（严格照规范的顺序，`initialized` 之前不发消息，吸取 ext-apps #671 的教训），App 调工具时由宿主绑定“是哪个插件在调”（不信前端传来的插件 id）。
- 对应关系：插件页 = fullscreen；聊天卡片 = inline；App 请求 pip 时，Kotone 可以映射成自己的置顶覆盖层（游戏上面），这正是游戏场景要的（见第五节）。
- 主窗口 CSP 加 `frame-src http://127.0.0.1:<端口>`；WebView2 下的行为要真机测。

### 4.6 配置

- `KOTONE_HOME` 下一份 `mcp.json`，格式就是 `mcpServers`（`command` / `args` / `env` 或 `url` / `headers`），能直接从 Claude Desktop、VS Code 的配置导入；开关照旧在 `plugins.<id>.enabled`；Key 不写进文件（凭据库）。
- 这是新的配置格式（单向门，原则 11）：照着事实标准写，就不用自己发明。

### 4.7 OP.GG 是第一个用户

英雄联盟插件里手写的 OP.GG 连接换成 `kotone-mcp`（行为不变，原则 14：OP.GG 是第一个用户，通用客户端是第二个）。之后玩家也可以直接把 OP.GG 的 MCP 服务器当一个远程插件加进来。

## 五、推动“桌面应用收敛成 MCP Apps”

判断：方向是对的，而且现在正是空档。MCP Apps 规范已经稳定，Claude、ChatGPT、VS Code、Goose 都在渲染，但游戏场景一个带界面的 App 都没有；游戏工具要的两样（推送事件、盖在游戏上的浮层），前一样有定稿的扩展没人实现，后一样连提案都没有。

Kotone 能做的，按先后：

1. **做游戏场景的参考宿主**：同时实现 MCP Apps 和 Events（SEP-3415）的宿主几乎没有。Kotone 做了，就是“游戏工具做成 MCP App 以后在哪跑”的第一个答案。
2. **做参考 App**：把英雄联盟插件拆成一个独立的 MCP 服务器 + App（ADR 038 D7 第 6 期“`kotone.exe mcp` 提供 `ui://` 资源”）：
   - 工具：查这局、查战绩、查英雄、强度榜（标只读）；接受对局、选 / 禁英雄（要确认）；只给页面按钮用的标 `visibility: ["app"]`；
   - 事件：客户端的 `OnJsonApiEvent` 转成 `lol.gameflow.phase_changed`、`lol.champ_select.updated`、`lol.game.ended`（Events 扩展），要不要叫醒她由宿主定；
   - 界面：“这局”“战绩”“英雄”是 `ui://` 页面；选英雄时的胜率、海斗评分用 pip 请求，Kotone 映射成游戏上方的覆盖层，别的宿主就显示成画中画。
   - 发到官方 MCP Registry。这样在 Claude Desktop 里也能打开 Kotone 的“这局”。
3. **把缺的补成提案**：先用现有原语（浮层先复用 pip），真不够用再去 `modelcontextprotocol/ext-apps` 提 issue / SEP（例如“宿主可以把 pip 放在全屏应用上方”“游戏状态作为宿主上下文”）。
4. **找同路人**：给 LeagueAkari 提一个 issue，建议它对外提供 MCP（它是 MIT、Electron，结构上不难）；OP.GG 那边已经有 MCP 服务器（我们的 issue #26 也在那里）。

风险照旧：游戏协议（腾讯 / Riot）只“警告不拦”；永远不读写游戏内存。

## 六、分期（每期能单独编译、单独提交、单独验收）

| 期 | 做什么 | 玩家看到的 | 怎么验 |
| --- | --- | --- | --- |
| 0 | 修 `kotone.exe mcp` 的名字映射（`voice_chat.*`） | 外部 Agent 经 MCP 能调游戏语音 | 单元测试 + 一个 MCP 对话的真机脚本 |
| 1 | `crates/kotone-mcp`（rmcp）；OP.GG 迁过去 | 看不到（行为不变） | `lol_tools_real.py` 照过；OP.GG 开着时的真实调用 |
| 1.5 | 补 ADR 029 的“主会话里工具超过 3 秒没回来就转给后台” | 慢的事自动交给后台，不卡通话 | 一个故意慢的假工具的单元测试 + 语音真机脚本 |
| 2 | 加服务器：`mcp.json`、导入、信任提示、远程插件、卡片、开关、她在后台用、询问、不可信 | 插件页能加 MCP 服务器 | 本机起一个官方示例服务器（stdio、HTTP 各一个）的真机脚本；权限和不可信的单元测试 |
| 3 | 显示 MCP Apps：回环代理、`AppBridge`、插件页全屏、聊天卡片 | 点卡片打开别人的 App | ext-apps 的示例 App 和 basic-host 做对照；WebView2 真机 |
| 4 | Events + pip → 覆盖层 | 游戏里第三方 App 能贴到游戏上 | 假客户端推事件的真机脚本 |
| 5 | 英雄联盟插件对外做成 MCP 服务器 + App，发 Registry | Claude Desktop 里能打开 Kotone 的“这局” | 在 Goose / VS Code 里打开 |
| 6 | 目录（官方 Registry）、`.mcpb` 导入、OAuth | 从目录里挑着装 | |

## 七、要玩家定的（看得见的）

1. ~~第三方工具她能不能在语音里马上用？~~ **定了**（玩家 2026-11-24）：只看快不快。实现上先补 ADR 029 的“超过 3 秒就转后台”，第三方工具直接进主会话。
2. ~~加进来的第三方工具，要不要再经 `kotone.exe mcp` 转给外部 Agent~~ **定了**（玩家 2026-11-24 夜，选“连第 2 期也做”，照推荐的默认）：默认不转（隐私：别人的服务器只在 Kotone 里用）。
3. ~~第三方工具的询问~~ **定了**（同上）：照 Codex 的 `writes`——没标只读的第一次问，可以“总是允许这个工具”；标了只读的，信任这个服务器之前也问一次。
4. ~~推动的那一半做不做~~ **定了**（玩家 2026-11-24：“我觉得 lol 插件可以对外发布”）：第 5 期做。风险提示（腾讯 / Riot 协议、LeagueAkari #523）跟着服务器的说明和 App 一起发。

## 八、风险

- 第三方程序在玩家电脑上用玩家的权限跑，Windows 上还没有沙盒（ADR 030 已知风险 5）。只能靠信任提示、默认询问、不可信标记、环境变量白名单。
- 协议在快速变化（2026-07-28 大改、rmcp 一个月四个小版本）：锁版本、跟着 Goose 升。
- MCP Apps 在 WebView2 里的 iframe 行为、CSP 要真机测；Claude Desktop 自己都还有渲染问题（ext-apps #671）。
- 工具多了她会选错：靠“只给后台 + 按关键词找工具 + 数量上限”。

## 出处

- MCP Apps：[概览](https://modelcontextprotocol.io/docs/extensions/apps)、[规范 2026-01-26](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)、[SEP-1865](https://modelcontextprotocol.io/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp)、[ext-apps 仓库](https://github.com/modelcontextprotocol/ext-apps)、[AppBridge](https://apps.extensions.modelcontextprotocol.io/api/modules/app-bridge.html)、[#671](https://github.com/modelcontextprotocol/ext-apps/issues/671)、[博客 2026-01-26](https://blog.modelcontextprotocol.io/posts/2026-01-26-mcp-apps/)
- MCP 规范：[tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)、[ToolAnnotations](https://modelcontextprotocol.io/specification/2025-11-25/schema#toolannotations)、[2026-07-28 changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)、[注解博客 2026-03](https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/)、[Events SEP-3415](https://modelcontextprotocol.io/seps/3415-events-extension)、[客户端矩阵](https://modelcontextprotocol.io/extensions/client-matrix)、[Registry](https://github.com/modelcontextprotocol/registry)、[MCPB manifest](https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md)
- 宿主：Claude Desktop（[本地服务器](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop)、[Desktop Extensions](https://www.anthropic.com/engineering/desktop-extensions)）、[Claude Code MCP](https://code.claude.com/docs/en/mcp)、Goose（[扩展](https://goose-docs.ai/docs/getting-started/using-extensions)、[配置](https://goose-docs.ai/docs/guides/config-files)、[mcp_app_proxy.rs](https://github.com/aaif-goose/goose/blob/main/crates/goose/src/acp/mcp_app_proxy.rs)、[known issues](https://goose-docs.ai/docs/troubleshooting/known-issues)）、VS Code（[配置](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)、[管理](https://code.visualstudio.com/docs/agents/run/manage-mcp-servers)、[工具上限](https://code.visualstudio.com/docs/agents/run/tools)）、[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)、[OpenClaw MCP](https://docs.openclaw.ai/tools/mcp)、[Hermes MCP](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp)、[Anthropic tool search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool)
- Rust：[rmcp crates.io](https://crates.io/crates/rmcp)、[rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)、[OAuth](https://github.com/modelcontextprotocol/rust-sdk/blob/main/docs/OAUTH_SUPPORT.md)、[CVE-2024-24576](https://blog.rust-lang.org/2024/04/09/cve-2024-24576/)
- 游戏：[lcu-mcp](https://github.com/Triggered0/lcu-mcp)、[minecraft-mcp-server](https://github.com/yuniko-software/minecraft-mcp-server)、[opgg-mcp](https://mcpservers.org/en/servers/opgginc/opgg-mcp)、[游戏 MCP 汇总](https://dev.to/grove_chatforest/gaming-esports-mcp-servers-steam-opgg-minecraft-chess-cs2-twitch-igdb-and-more-56o3)、[LeagueAkari](https://github.com/LeagueAkari/LeagueAkari)
- 以上大多数据（星数、版本、是否还在维护）来自 2026-11-24 的网页和源码，未逐一核实的已在正文里注明。
