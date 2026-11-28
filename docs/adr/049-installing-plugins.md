# ADR 049：装插件——一个目录、一种包、Kotone 自己下载安装；国内从 COS 下

状态：Accepted（2026-11-28，玩家：“插件相关的那些，按照你的建议写吧，这轮不改插件，只写文档”）/ 未实现：这一轮只写文档，分期见文末。

补充：[ADR 047](047-plugin-standard.md)（插件标准只管装上以后怎么说话，没写插件从哪来、怎么装）。相关：[ADR 027](027-0.2-upgrade-transition.md)（腾讯云 COS 优先、GitHub 兜底的更新通道）、[ADR 043](043-plugin-shape-and-pushes.md)（每个插件一个“风险和许可”弹窗）、[ADR 044](044-run-as-admin-by-default.md)（Kotone 启动的程序一律降回普通权限）。调研（2026-11-28，读源码，没运行任何下载的代码）：Coopanion / Cortico 的扩展市场，MCPB（Claude Desktop 的扩展包），官方 MCP Registry。

## 背景

玩家（2026-11-28）问 Steam 插件怎么加载，然后问：

> “那正常理想中的流程应该是什么样呢？用户想安装一个插件？”
>
> “至于下载方面，看一下这个项目（Coopanion），它是怎么处理国内用户的插件获取和下载的”

现在装一个第三方插件：自己 `cargo build`（或者找到 exe）→ 放到固定的地方 → 插件页粘贴一段 `mcp.json` → 信任一次。这只适合插件作者，玩家不该看到 exe 路径和 JSON。[kotone-plugin-steam](https://github.com/l1veIn/kotone-plugin-steam) 也还没有 Release。

## 别人怎么做

### Coopanion / Cortico

[Pal-AI-Lab/Coopanion](https://github.com/Pal-AI-Lab/Coopanion)（AGPL，只学做法）的扩展市场在它的框架 [Cortico](https://github.com/Pal-AI-Lab/Cortico)（MIT）里，`src/extensions.ts`、`src/extensions/market.ts`；官方索引仓 [cortico-extensions](https://github.com/Pal-AI-Lab/cortico-extensions)。

- **扩展就是 npm 包**，作者用自己的 npm 账号发。市场列表 = 按关键字查 npm（`/-/v1/search?text=keywords:…`），再合上官方索引仓里的两份文件：`catalog.json`（Action 每 30 分钟扫一次 npm 生成，现在 `packages/` 下 432 个包）和 `index.json`（标签词表、官方账号、认证、拉黑、覆盖）。
- **先上架、后拉黑**：npm 上带了关键字就出现在市场，不预审；八类（越狱、NSFW、攻击、追踪他人、恶意代码、联网游戏外挂、刷量、违法）一进市场就拉黑，同一作者两次永久拉黑。“认证”另外申请。作者在索引仓开 issue 申请标签、认证、申诉、`[sync]` 马上收录，Action 处理。评论和点赞在 GitHub Discussions，应用里用 GitHub 设备码登录。
- **安装**：在数据目录 `extensions/` 里跑 `corepack pnpm add <包>`。
- **国内用户怎么下**：
  1. **没有自己的镜像服务器，也没有内置国内源**，默认直连 npm 官方和 GitHub。
  2. **跟系统代理**：主进程用 Electron 的 `session.resolveProxy` 按目标主机问系统代理（PAC 也生效），每个主机的答案缓存 10 秒，开关代理软件不用重启；装扩展的 pnpm 子进程跟 npm registry 的答案（`core/system-proxy.ts`）。
  3. **扩展页能填“npm 源”**：写进 `extensions/.npmrc` 的 `registry=`，搜索、详情、检查更新、安装都走它。国内用户自己填 npmmirror（淘宝源），**国内的速度靠的是 npmmirror 免费、自动同步整个 npm**，作者什么都不用做。坑：npmmirror 不认 `keywords:` 查询，回空列表，市场就空了（0.1.24 修：回空时改按关键字做文本搜索，确实为空时写出正在用的源和换源方法；issue #78、#183 也都是“源写死、不吃代理”这类）。
  4. **索引文件三层退路**（`fetchIndexFile`）：先 `raw.githubusercontent.com`（CDN，5 分钟缓存，不限次数）→ 连不上、超时、非 2xx 改问 `api.github.com` 的 contents 接口（不带令牌每个 IP 每小时 60 次）→ 都不行用上次缓存的 `market-catalog.json` / `market-index.json`，页面写明是旧的。“索引源”也能换成别的地址（目录结构相同）。下载量只问 npm 官方（镜像没有这个数），3 秒没回就不显示。图标和截图仍从 raw.githubusercontent 取。

### MCPB（Claude Desktop 的一键扩展包）

[modelcontextprotocol/mcpb](https://github.com/modelcontextprotocol/mcpb)：一个 zip（`.mcpb`），里面是 `manifest.json`（`manifest_version` 0.3）加服务器本身；`server.type` 是 `node` / `python` / `binary`；`user_config` 声明要用户填的东西（Key、目录……），宿主装的时候弹表单，用 `${user_config.名字}` 填进 `env` / `args`；`compatibility.platforms` 写支持的系统。Claude Desktop 双击就装。

### 官方 MCP Registry

[modelcontextprotocol/registry](https://github.com/modelcontextprotocol/registry)：`server.json` 的 `packages` 可以是 `mcpb`，**必须带 `fileSha256`**，下载地址只允许 github.com / gitlab.com 的 Release（`internal/validators/registries/mcpb.go`）。它是“所有 MCP 服务器”的总目录，不分游戏、不审内容。

### Kotone 自己走过的路

0.1.x 的更新清单用过 ghfast.top、gh-proxy.com、ghproxy.net 这几个公共 GitHub 代理（`scripts/make-updater-jsons.mjs` 里还给旧客户端留着这几个变体），后来改成**腾讯云 COS 优先、GitHub 兜底**（ADR 027）。公共代理随时会挂、可能改内容，新东西不再用。

## 决定

### D1 玩家看到的流程

1. 插件页「找插件」：一列卡片，写名字、做什么、属于哪个游戏、作者，以及**读什么、联不联网、会不会操作键鼠**（取自插件的 Kotone 清单 `risk`，目录里抄了一份）。本机检测到装了某个游戏，对应的插件排前面。
2. 点「安装」：**一个弹窗**，把“它会在你电脑上运行一个程序”和“风险和许可”合在一起；插件要填的东西（MCPB 的 `user_config`，比如 Key）也在这个弹窗里填。确认一次。
3. Kotone 下载、校验、解压、登记、以普通权限启动它，卡片出现，马上能用。插件带事件的，顺便问一句要不要打开（还是 ADR 047 的确认）。
4. 有新版本时卡片上出现「更新」。只有风险项、事件或要先问的工具变了才再弹窗，别的点一下就更新。
5. 「卸载」：停掉、删文件、去掉登记、删它在凭据库里的东西；用了它事件的自动化标“用不了”，不删。

不在目录里的两个入口留着：**把一个 `.mcpb` 拖进插件页**（弹窗多一句“这个插件不在目录里，没人看过”），和现在的**粘贴 `mcp.json`**（给作者和高级玩家）。

### D2 目录：一个公开仓库里的一份 JSON

- 仓库 `l1veIn/kotone-plugins`，`catalog.json` 每个插件一项：`id`、名字、说明、`games`、作者、源码仓库、当前版本、包的地址（作者仓库的 GitHub Release）、**`sha256`**、大小、要求的 Kotone 版本和标准版本，以及从包里读出来的清单摘要（`risk`、事件、工具数）。照 Obsidian 的 `community-plugins.json` 和 Cortico 的 `catalog.json`。
- **收录要过一次人看**：作者提 PR 加一项，维护者看源码和清单后合并。之后的新版本由 Action 收：同一个仓库、Release 是 GitHub Actions 编出来的（带 [artifact attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations)，`gh attestation verify` 验得过）、`risk` 和要问的工具没变，就自动更新这一项；变了等人看。
- 理由：Kotone 的插件是**原生 exe**，以玩家的权限跑，能做的事比一个网页扩展多；玩家群还小，一个一个看得过来。Cortico 的“先上架后拉黑”适合几百个包、有人专门盯的市场。
- `blocked` 一栏：拉黑的插件不再出现；已经装了的，卡片上写原因、建议卸载（不替玩家删）。

### D3 包：只收 MCPB，0.x 只收 `binary`

- 一种格式：`.mcpb`。同一个包在 Claude Desktop 也能装，符合 ADR 047 “只靠外层协议”。Kotone 清单还是插件自己的 MCP 资源 `kotone://manifest`，不塞进 MCPB 的 manifest；MCPB manifest 里 Kotone 用不上的字段不管。
- `user_config` 就是安装弹窗里的表单；标了 `sensitive` 的存凭据库（`plugin:<id>:<名字>`），其余写进登记。它替代现在 `mcp.json` 里的 `${input:…}`。
- **0.x 只收 `server.type: binary`、`win32`**：Kotone 不带 Node、Python 运行时，`node` / `python` 的包装不了（Claude Desktop 自带 Node 才能装）。以后真有需要再说。
- 给作者：`kotone-cli plugin check` 能直接检查一个 `.mcpb`（解压到临时目录、照 manifest 起服务器、跑现在的那套检查）；`docs/plugin-standard.md` 加“发布”一节，Steam 插件的仓库给一个 Release 工作流做样子。

### D4 下载：COS 优先，GitHub 兜底，两边都对 sha256

- 目录仓库的 Action 在一个版本收录时：从作者的 Release 下载 → 对 `sha256` → 传到 COS `plugins/<id>/<版本>/<文件>`（和安装包一样，版本化的对象不许覆盖）。`catalog.json` 本身也同步一份到 COS。
- Kotone 读目录：**COS → raw.githubusercontent → api.github.com contents → 上次的缓存**（照 Cortico 的三层退路，前面加一层 COS），用了缓存就写“目录是 N 小时前的”。
- Kotone 下包：**COS → GitHub Release**，两边下下来都对目录里的 `sha256`，对不上不装、说清楚；都下不了就写出试过的地址和原因（照 Cortico 0.1.18、0.1.24 的教训：写出连的是哪、底层错误是什么）。
- 跟系统代理（Windows 的代理设置）；不用公共 GitHub 代理。Kotone 现在的 HTTP 客户端读不读系统代理要核对。
- 为什么不照 Coopanion 走 npm + npmmirror：免费、作者省事，但插件是 exe，要作者另开 npm 账号把 exe 塞进 npm 包；npmmirror 同步大二进制有时慢、我们管不了；Kotone 已经有 COS 和发版流程。代价是 COS 的存储和流量（一个插件几 MB），以及给发版子账号加 `plugins/*` 的写权限。

### D5 安装器

1. 下载到临时目录 → 对 `sha256` → 解压到 `<KOTONE_HOME>/plugins/<id>/<版本>/`（和 ADR 047 的 `plugins/<id>/manifest.json` 同一个文件夹）。
2. 先试起一次：照 MCPB manifest 起服务器、握手、读 `kotone://manifest`。起不来就不切过去，说原因。
3. 在 `mcp.json` 里写这一项（`command` 指向解压出来的 exe），另在 `plugins/<id>/installed.json` 记来源、版本、`sha256`、目录里的哪一项。玩家手写的 `mcp.json` 项不受影响。
4. 以普通权限启动（ADR 044），弹 D1 那一个弹窗。
5. 旧版本留一份：新版本起不来自动退回旧的；成功跑过一次后删更旧的。

### D6 更新

- 启动时和之后每 6 小时看一次目录（和 Kotone 自己的更新一样）；有新版本卡片上写「更新」。**0.x 不自动更新**，点了才换。
- 更新 = 下新版本 → 试起 → 停旧的 → 换 `mcp.json` 那一项 → 起新的。风险项、事件、要问的工具变了，先弹窗。

### D7 插件信息和头像

玩家（2026-11-28）：“插件包里应该自带头像……插件的头像很重要……插件卡片页需要用户能‘查看插件信息’，看一下作者啊，代码仓库之类的”。

**现在有的**（2026-11-28 核对代码）：

| 哪种插件 | 有的元信息 | 头像 | 界面上显示 |
| --- | --- | --- | --- |
| 内置（`plugin.toml`） | `id`、`name`、`description`、`games`，以及页面、事件、推送、数据开关这些功能项 | `plugins/<id>/logo.svg` | 头像、名字、说明；没有作者、版本、仓库（都是 Kotone 自己） |
| 第三方（握手时的 `serverInfo`） | `name`、`title`、`version`、`description`、`icons`、`websiteUrl`（MCP 2025-11-25） | `icons` 里 `data:image/…` 的才显示；`https://` 的过滤掉了 | 头像、名字、说明；`version`、`websiteUrl` 传到了前端但没显示 |
| 第三方（Kotone 清单） | `standard`、`games`、`events`、`watches`、`risk` | — | 设置页的“Kotone 清单”“事件”两节 |

没有的：作者、代码仓库、许可证、问题反馈地址、长说明、截图。MCPB 的 manifest 这些都有：`display_name`、`version`、`description`、`long_description`、`author {name, email, url}`、`repository {type, url}`、`homepage`、`documentation`、`support`、`license`、`keywords`、`privacy_policies`、`icon`（包里的 PNG 路径或 `https://` 地址）、`icons`（`src`、`size`、`theme` 亮 / 暗）、`screenshots`。

**决定：**

1. **头像跟着包走**：MCPB 包里必须有 `icon`（包内的 PNG，至少 256×256，建议 512×512，正方形）；可以用 `icons` 另给暗色主题的一张。装的时候和程序一起解压到本机，显示时只读本机文件，**不联网取头像**（国内也快，离线也有）。目录收录时 Action 把头像抽出来，和包一起传到 COS（`plugins/<id>/<版本>/icon.png`），「找插件」的列表装之前就有头像。检查命令（`plugin check`）没有头像算 ✗。
2. **哪种插件的头像从哪来**：内置的照旧 `logo.svg`；装的包用包里的 `icon`；粘贴 `mcp.json` 加的用 `serverInfo.icons`：`data:` 的照旧，`https://` 的**加的时候下载一次存到本机**（只收 PNG / JPEG / WebP、不超过 200 KB、不跟随跳转到别的主机），以后不再联网取；都没有用首字块。头像一律放进 Kotone 自己画的圆角方块里（`PluginLogo`），亮暗主题下都看得清。
3. **「插件信息」**：插件卡片上的“⋯”菜单和插件设置页的标题旁各一个入口，打开一个弹窗，只读：
   - 头像、名字、版本、一句话说明；有长说明就在下面（只认纯文字和链接，不渲染 HTML）；有截图就一排小图。
   - **作者**（名字，有链接就点开）、**代码仓库**、主页、文档、**问题反馈**、**许可证**。只显示有的，没有的那一行不出现；链接只认 `https://`，用系统浏览器打开。
   - **来源**：Kotone 内置 / 插件目录（收录的版本、什么时候收录的）/ 你拖进来的包（不在目录里，没人看过）/ 你粘贴的配置。
   - **它能做什么**：属于哪些游戏，几个工具、几个页面、几个事件、几种推送（数字，点开看名字）。
   - **它会运行什么**：程序的位置或服务器地址（现在信任弹窗里那一块 `McpTrustFacts` 搬过来用）；“风险和许可”一个链接，打开现有的弹窗。
   - 给作者看的小字：插件 id、标准版本、包的 SHA-256、装在哪个文件夹。
   - 内置插件的作者写“Kotone”，仓库指向 `l1veIn/kotone` 里 `plugins/<id>`，许可证 MIT；照着别人的开源项目写的部分，在这里写“参考”并链接 `THIRD_PARTY_NOTICES.md`。
4. **信息从哪来、谁说了算**：装的包以包里的 MCPB manifest 为准；目录里那一项只补“来源”和收录信息，不改作者自己写的；粘贴配置加的只有 `serverInfo` 那几项。两边都有的（名字、说明、版本），以包为准，运行时 `serverInfo` 报的版本不一样时，小字里写出来（多半是作者忘了改版本号）。
5. **目录收录要求**：`author.name`、`repository.url`、`license`、`icon` 必填，`support` 建议填；`repository.url` 必须就是 Release 所在的仓库（D2 的 attestation 也对它）。

### D8 不做

评分和评论（Cortico 有，以后再说）、付费、装插件时替它装依赖（Node、Python、VC 运行库）、公共 GitHub 代理、自动装“推荐插件”。

## 玩家定的（2026-11-28，照推荐）

1. **收录先过一次人看**：第一次收录由维护者看源码和清单；之后同一个仓库、GitHub Actions 编的、风险和要问的工具没变的新版本由 Action 自动收，变了再看。不照 Cortico 先上架后拉黑。
2. **国内下载走 COS**：要给 COS 发版子账号加 `plugins/*` 的写权限，目录仓库加两个 Secrets（实现时玩家来配）。不让作者发 npm，也不只靠 GitHub。
3. **包只收 MCPB，0.x 只收 `binary`**。
4. **0.x 更新手动点**，不自动。
5. **目录仓库叫 `l1veIn/kotone-plugins`**。

## 分期

0. 不等目录就能做的：「插件信息」弹窗（先显示现在就有的：内置插件的、`serverInfo` 的 `version` / `websiteUrl`、Kotone 清单、它会运行什么）；`serverInfo.icons` 的 `https://` 头像加的时候下载存本机。
1. 目录仓库 + 校验和同步 COS 的 Action；Steam 插件发第一个 `.mcpb` Release（GitHub Actions 编、带 attestation，自带头像，作者、仓库、许可证写全）；`plugin check` 认 `.mcpb`、检查 D7 的必填项；`docs/plugin-standard.md` 写“发布”。
2. Kotone：「找插件」（带头像）、安装弹窗、安装器、卸载；「插件信息」补上包里的作者、仓库、许可证、截图和来源。
3. 更新、拉黑。
4. 拖入 `.mcpb`。

## 风险

- **装的是陌生 exe**：人看的是源码和清单，看不出编出来的 exe 有没有夹带；所以要求 Release 由 GitHub Actions 从这个仓库编出来（attestation），Action 验过才收。
- **杀毒软件**：没签名的 exe 可能被 360、火绒报。Kotone 自己下载，文件没有“来自网络”的标记，不会弹 SmartScreen；被杀毒删了要说清楚是哪个文件、去哪恢复。
- **COS 费用**：插件小、玩家少，先不设上限，发版时看一眼账单。
- **目录读不到**：用缓存，写明是旧的；从没读到过就只剩“拖入 `.mcpb`”和“粘贴 `mcp.json`”。
