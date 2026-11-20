# ADR 031：内置浏览器与浏览器操作——在后台替玩家操作网页

状态：Accepted / 已确认（2026-11-20），玩家决定随 0.2.0-rc.1 一起上（“做完再发”），位置改成聊天页右侧栏（见第 1 节）。相关：[ADR 019](019-game-computer-use.md)（操作游戏）、[ADR 029](029-voice-call-and-background-tasks.md)（后台任务）、[ADR 030](030-subagent-shell-and-files.md)（读过外面的东西先问）。调研：[常见音乐软件](../research/2026-11-20-music-apps.md)。

## 背景

玩家想说“我要听晴天”。调研发现点歌要么靠各家未授权的搜索接口查歌曲编号，再用随版本变化的链接或命令行让客户端播放；要么像 GitHub 上的几个网易云 MCP 那样自己当播放器。而且网易云客户端收到播放链接后会把主窗口拉到前台，打断游戏。

已有的操作电脑（ADR 019）是给游戏用的：截图看画面，模拟真实的键盘鼠标，要求目标窗口在前台。玩家在游戏里时，它没法去操作别的程序。

浏览器正好补上：网页本身就有搜索框和播放按钮；网页在 Kotone 自己的窗口里（聊天页右侧栏），完全不碰前台。

| | 操作电脑（ADR 019） | 浏览器操作（本 ADR） |
| --- | --- | --- |
| 操作谁 | 游戏和桌面程序 | Kotone 自己窗口里的网页 |
| 怎么看 | 截图 + 文字识别 | 读网页结构，每个元素一个编号 |
| 怎么动 | 模拟真实键鼠，要求在前台 | 经浏览器调试协议直接发给页面，不发系统按键 |

## 照谁做（不自己造轮子）

读了 DeepSeek Harness（DSH）生态里排名靠前的浏览器插件和 DSH 本身（2026-11-20）：

- **DSH 核心**（MIT）没有自研浏览器插件，只包了 Playwright MCP、Chrome DevTools MCP、Stagehand 三个后端（`packages/experimental/browser-use-*`）。DSH 桌面版的侧栏浏览器明确**不开调试端口、不接自动化**，设计记录写将来只能走“经过认证、只能碰单个页面”的中间层（`.agents/notes/implemented/feature/2026-09-20-desktop-browser-webview.md`）——和本 ADR 的进程内调用一致。
- **快照和点击照 `omdsh-dev/dsh-ego-browser`**（MIT）：全是调试协议调用，不往页面注入代码库；新标签强制后台打开，吞掉会把窗口拉到前台的调用（`transport.mjs`）。
- **安全外壳照 `omdsh-dev/dsh-browser`**（MIT，1.1k★）：页面内容包进带随机标记的“不可信”边界（`security/untrusted.ts`），密码和卡号字段打码（`content/privacy.ts`），按站点授权。
- **网址检查照 `xylt369/dsh-browser`**（MIT）的 `url-guard.ts`：只许 http(s)、网址里不许带账号密码、拦内网和保留地址（解析域名后再查一遍）。
- **最大的坑来自 `wqty123/dsh-browser`** 的实测（`docs/limitations.md`）：看不见的页面收不到调试协议发的点击和按键，Chromium 静默丢弃却照样回成功；Windows 上被别的窗口完全挡住也算看不见。玩家打全屏游戏时，我们的窗口正好是“被完全挡住”。

不照搬的：
- **驱动玩家自己的 Chrome**（dsh-browser、dsh-browser-cdp 的做法）。它要么装扩展，要么开调试端口（本机任何程序都能接管）；Agent 能碰到玩家所有已登录的网站（邮箱、Steam、支付），同时又在读不可信网页、还能对外操作，Rule of Two 三样全占；Windows 上切标签会把 Chrome 拉到前台。
- **几十个工具**（这些插件有 16–48 个）。我们保持 7 个。

## 决定

### 1. 内置浏览器

- **在聊天页的右侧栏里**（2026-11-20 玩家决定，照 DSH 桌面版的侧栏浏览器）：右侧栏常驻后台任务卡片和浏览器卡片，浏览器展开时也在右侧展开。浏览器是主窗口（`main`，关闭时只是隐藏）里的一个 Tauri 子网页视图（`Window::add_child`，tauri `unstable` 特性），第一次打开网页时创建；位置和大小跟着右侧栏里那块区域走（界面报给 `browser_set_bounds`）。收起、切到别的页面、有对话框时只隐藏、不销毁：页面、登录、正在放的歌都保留（照 DSH `keepMounted`）。只有一个页面，不做标签页、地址栏、书签。
- 数据单独存放在 `<KOTONE_HOME>/browser/`（Tauri `data_directory`，独立的 WebView2 环境），和 Kotone 自己的界面分开。
- 网页调用不了 Kotone 的任何功能：Tauri 2.11 对非本地来源一律拒绝自定义命令和插件命令，除非某个 capability 写了 `remote`（`tauri` `webview/mod.rs` 的 remote-origin 守卫）；我们的 capabilities 一个都没写（有测试）。
- 启动参数只加在这个浏览器自己的环境上（照 ego 和 wqty123）：`--disable-renderer-backgrounding`、`--disable-background-timer-throttling`、`--disable-backgrounding-occluded-windows`、`--disable-features=CalculateNativeWinOcclusion`；为了网页播放器，试 `--autoplay-policy=no-user-gesture-required`。
- 设置里一个按钮“清除浏览器登录”，删掉这个目录。

### 2. 浏览器操作工具（名字照 Playwright MCP，做法照 ego）

| 工具 | 做什么 |
| --- | --- |
| `browser.open {url}` | 打开网址（先过网址检查和站点授权） |
| `browser.snapshot {more?}` | 读页面：带编号的元素清单和文字；默认约 2.4 万字，用 `more` 接着读；和上一次相比只给变化（结果标记为不可信） |
| `browser.click {ref}` | 点击某个元素 |
| `browser.type {ref, text, submit?}` | 在输入框里输入，可选回车 |
| `browser.press {key}` | 按一个键（发给页面，不是系统按键） |
| `browser.wait {text?, seconds?}` | 等某段文字出现或等几秒 |
| `browser.handover {reason}` | 交给玩家（登录、验证码、付款），见第 3 节 |

实现要点：
- **全走 WebView2 的 `CallDevToolsProtocolMethod`**（程序内调用，不开调试端口）。
- **快照**：一次 `DOMSnapshot.captureSnapshot` 拿整页（含 iframe），用浏览器的节点号做元素编号，节点在编号就不变；每次快照记一个代数，拿旧代数的编号来操作直接拒绝，让模型重新读（照 ego `snapshot.mjs`、`snapshot-diff.ts`，Rust 移植约 700 行）。密码框的值打码。
- **点击**：隐藏状态下主路径是找到节点后在页面里调用它的 `click()`（`DOM.resolveNode` + `Runtime.callFunctionOn`）；鼠标事件（移动 → 按下 → 松开，发完检查有没有点中）作备选（照 ego）。
- **不抢前台**：吞掉 `Page.bringToFront`、`Target.activateTarget`，新页面不激活窗口（照 ego）。
- **验证码**：快照里命中验证码特征（中英文，含“滑块验证”，照 ego `captcha.ts`）就自动 `browser.handover`。
- 只给后台任务用，主会话没有这些工具（主会话要快）。

### 3. 登录：玩家自己来，扫码优先

照 ChatGPT Agent、Claude in Chrome 的做法：登录、验证码、付款交还给人。调研的几个插件都没有专门的扫码交接，这部分是 Kotone 自己的：

1. 后台任务读页面发现要登录，调用 `browser.handover {reason: "网易云要登录"}`。
2. 主会话告诉玩家“网易云要登录一下，我把窗口打开了，用手机扫个码”，同时显示浏览器窗口。
3. 页面变成已登录（或玩家说“好了”），窗口收起，后台任务接着做。

登录状态留在 `<KOTONE_HOME>/browser/`，重启后还在，过期了再扫一次；玩家是会员的，网页版同样按会员播放。

不做：Agent 不在密码框里输入（代码拦下）；不从玩家的 Chrome / Edge 导入登录信息（Chrome 127 起复制登录信息行不通，只能装扩展；也涉及隐私）。

### 4. 安全

- 页面内容包进带随机标记的“不可信”边界（照 dsh-browser）；读过之后，执行命令、写文件前先问一句（ADR 030 的标记照常生效）。
- 网址检查（照 xylt369 `url-guard.ts`）：只许 http(s)，网址里不许带账号密码，拦内网和保留地址。
- 第一次打开某个网站先问一句（“要我打开 music.163.com 吗？”），按域名记住。
- 登录、付款、替玩家发帖评论下单：交给玩家（`browser.handover`）。

### 5. 第一个用处：放歌（2026-11-20 玩家决定）

**音乐只在 Kotone 的内置浏览器里放，不去控制玩家电脑上的音乐软件**：玩家不用另外开网易云、QQ 音乐，也就不需要“系统媒体控制”这一层。

- 每个音乐网站一份技能（网易云、QQ 音乐、酷狗、酷我）：怎么判断有没有登录、怎么搜、怎么播放第一条。只是说明，不写代码。“我要听 xxx”交给后台任务。
- “暂停、继续、下一首、这首歌叫什么”：直接在 Kotone 自己的播放页上做（调用页面里的播放器，不经过后台任务），所以当场生效。具体做法原型时定。
- **让玩家知道能放歌**（不加新界面）：
  - 聊天页的起手提示加一条“来首歌：”（以冒号结尾，放进输入框让玩家接着写歌名）；
  - 能力概览加“放歌”一项，玩家问“你能做什么”时她会说；
  - 更新日志写一句。
  - 第一次点歌时如果没登录，她会请玩家扫码（第 3 节），这一步本身也在告诉玩家“歌在 Kotone 里放”。

## 不做的

- 不开浏览器调试端口；不驱动玩家自己的浏览器；不做通用浏览器界面。
- 不导入其他浏览器的登录信息；不替玩家输入密码、付款。
- 不截图识别（以后真遇到只能看图的页面，再照 Playwright MCP 的 vision 选项加）。

## 原型先验证（2026-11-21 真机结果，主窗口一直隐藏 = 玩家在全屏游戏里）

1. **隐藏的 WebView2 能不能收到操作**：能。页面内 `click()`、原生 setter 输入、提交都生效（必应搜索、点开第一条结果跳到别的网站）；调试协议的鼠标事件只作备选，没用上。
2. **被全屏游戏挡住时**：主窗口隐藏比“被挡住”更严，上面已经通过；音乐会不会被降频没有测到（没放成功，见 4）。
3. **自动播放**：没测到（同 4）。
4. **网易云 / QQ 音乐网页版**：歌曲页能打开、能读、能点“播放”；不登录时网易云《晴天》点了播放不出声，要靠交给玩家扫码登录。扫码登录本身要玩家实测。
5. **暂停、下一首、读歌名**：能找到页面里的播放器（含不挂在页面上的 `Audio`），播放 / 暂停直接调用；“下一首”按网站的按钮表点；放没放以 WebView2 的“页面在出声”为准。真放起来以后的效果要登录后实测。
6. **数据目录、启动参数、网页调不了 Kotone**：去掉测试环境变量后生效；`CallDevToolsProtocolMethod` 在 `with_webview` 里调用、回调里只发 channel，没有死锁；远程网页拿不到命令由 Tauri 的 remote-origin 守卫保证，我们的 capabilities 没有 `remote`（有测试）。

## 实现情况（2026-11-21，`claude/voice-call`）

| 部分 | 在哪 | 照谁 |
| --- | --- | --- |
| 工具说明、网址检查、页面快照（角色 / 名字 / 编号 / iframe / 密码打码）、分页、和上次比、不可信边界、站点授权、验证码特征、音乐网站“下一首”按钮表、交给页面执行的点击 / 输入 / 按键脚本 | `crates/kotone-agent/src/browser.rs`（纯逻辑，有单测） | ego `snapshot.mjs` / `snapshot-diff.ts` / `captcha.ts`，dsh-browser `untrusted.ts` / `privacy.ts`，xylt369 `url-guard.ts` |
| 子网页视图（右侧栏）、访客策略（权限 / 下载 / 认证弹窗 / 新窗口 / 导航）、状态与轮询、清除登录 | `apps/desktop/src-tauri/src/browser/mod.rs` | DSH 桌面版侧栏浏览器记录 |
| 程序内调试协议桥（`CallDevToolsProtocolMethod`，回调里只发 channel） | `browser/cdp.rs` | WebView2 文档 |
| 打开、读页面、点击（页面内 `click()` 为主、鼠标事件备选）、输入、按键、等待、交给玩家、`music` | `browser/page.rs` | ego |
| 后台任务的 `browser.*`：第一次打开某个网站经主会话问、解析后的地址再查一遍、遇到验证码自动交给玩家、语音入口时开口说一句（不把 Kotone 拉到前台） | `apps/desktop/src-tauri/src/agent/browser_tools.rs` | ADR 030 的确认路径 |
| 界面命令 `browser_state / browser_set_bounds / browser_control / browser_handover_done / browser_clear_data`，事件 `kotone://browser` | `browser_cmd.rs` | — |

- 工具都走 `Source::Exec`（经桌面端 `BuiltinHost::exec`，带调用 id、取消和“每次询问”），只给后台；`music` 是主会话的内置工具。`browser.*` 的结果按不可信算，接入 ADR 030 的“读过外面的东西”标记。新的副作用 `browses-web` / `plays-media` 默认允许（什么时候问由上面的规则定）。
- 和设计不同的地方：
  - 编号就是浏览器的节点号（照 ego）；“旧代数拒绝”做成：导航后清空上一次读页面的编号表，点击 / 输入只认最近一次读到的编号。
  - “和上次比只给变化”做成：同一网址再读时在结果前面附一句“新增 / 少了 / 变了几行”，正文仍是完整页面（模型不用自己拼）。
  - 网页不能抢焦点、弹对话框，用每个 frame 的初始化脚本做（`window.focus` / `alert` / `confirm` / `prompt` 置空），代替 ego 在调试协议上吞调用（我们自己从不调用 `bringToFront`）。
  - 真机脚本用环境变量 `KOTONE_BROWSER_MUTE` 让浏览器静音（半夜测放歌）。
- 实现时踩到、已经处理的：
  - **测试环境变量会让浏览器建不出来**：真机脚本给 Kotone 主界面设的 `WEBVIEW2_USER_DATA_FOLDER` / `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 会盖过这里指定的数据目录和启动参数，浏览器和主界面抢同一个数据目录、参数又不同，WebView2 报 `0x8007139F`，Tauri 照样返回句柄、之后发给它的消息全被悄悄丢掉。建浏览器前去掉这两个变量（玩家不会设）。
  - **Tauri / wry 自己的错误原来看不到**：它们走 `log` 库，Kotone 没接；现在把警告和错误转进 Kotone 的日志（上面的问题就是这样查到的）。
  - **网页的原生对话框会卡住隐藏着的页面**：关掉 WebView2 默认对话框，alert 直接确定，其余当取消，并记日志。
  - **网易云这类播放器用不挂在页面上的 `Audio` 对象**：初始化脚本登记所有调用过 `play()` 的媒体，`music` 用这份登记；“在不在放”另外用 WebView2 的 `IsDocumentPlayingAudio`（不管播放器怎么写都准）。
  - 只在调试版里可用的 `browser_probe` 命令：不经过模型直接打开 / 读 / 点 / 输入，真机冒烟脚本 `scripts/manual-e2e/browser_probe_real.py` 用它（发布版拒绝）。
- 真机（2026-11-21，真实 DeepSeek，主窗口一直隐藏、浏览器静音；`scripts/manual-e2e/browser_real.py`、`browser_probe_real.py`）：
  - 不经过模型（`browser_probe_real.py`）13/13：打开 example.com 1.2 秒、读页面 0.3 秒；必应输入“晴天 周杰伦”提交、点开第一条结果跳到 B 站；本机 / 内网 / `file:` / `tauri.localhost` 都打不开。
  - 经过模型（`browser_real.py`）：打开 example.com（第一次先问、记住同意）说出标题；必应搜索并点开第一条结果（百度百科《晴天》）。
  - “我要听晴天”：不登录时网易云网页版搜什么都是 0 首，QQ 音乐也要登录。第一次跑时模型把它当成“没这首歌”，换了四个网站，最后在酷狗点了播放但没出声；提示词补上“要登录就在这个网站上交给玩家，不要换网站”后，第二次在网易云上交给玩家扫码，没人扫码（脚本 20 秒后点“好了”）就照实说没放成。真放起来要玩家登录后实测（0.2.0 试玩清单）。
  - 主会话的 `music` 第一次跑时报“没有名为 music 的内置动作”（漏进了桌面端执行的名单），已修并加了测试。
  - 聊天页没有出现 mock 截图里那条 “Cannot read properties of null” 提示。
- 四个音乐网站不登录能不能放（2026-11-21，主窗口隐藏、浏览器静音，用调试版 `browser_probe` 不经过模型逐步操作；“在放”看 WebView2 `IsDocumentPlayingAudio`，静音时它照样如实回答）：

  | 网站 | 搜索网址 | 不登录点播放 | `IsDocumentPlayingAudio` / 媒体会话 |
  | --- | --- | --- | --- |
  | 网易云 | `music.163.com/#/search/m/?s=…&type=1` 只显示“找到 0 首” | 歌曲页“播放”点了没反应；不少歌是“VIP单曲”，周杰伦没有版权；页面一直引导“去客户端播放” | false / 无 |
  | QQ 音乐 | `y.qq.com/n/ryqq/search?w=…&t=song`（跳到 `ryqq_v2`） | 一打开就弹扫码登录 | false / 无 |
  | 酷狗 | `www.kugou.com/yy/html/search.html#searchType=song&searchKeyWord=…` 有结果 | 点结果进歌曲页、加入队列，弹扫码登录；点播放条 `#toggle` 没有音频地址（非周杰伦的歌也一样） | false / 无 |
  | 酷我 | `www.kuwo.cn/search/list?key=…` 有结果 | “播放全部”“立即播放”都弹“付费内容提醒”，音频地址残缺（不收费的歌也一样） | false / 无 |

  结论：四家都要登录（不少歌还要会员），登录后的播放留给玩家试玩时实测；`music` 的暂停 / 继续 / 下一首因此没有在真播放上验证过。“下一首”选择器表只留核对过**按钮存在**的三家（网易云 `a.nxt`、酷狗 `#next`、酷我 `div.next`），QQ 音乐不登录看不到播放条，去掉、只靠通用的“下一首”规则。
- 点歌的做法写成内置技能 `music-web`（`crates/kotone-agent/skills/music-web/SKILL.md`）：各网站的搜索网址、没登录时的样子、播放条按钮、要会员 / 没版权时怎么说；核对不了的标“未核实”。后台的提示词让“我要听 xxx”先读它，默认网易云，玩家记忆里写过常用网站就用它。
- **拦下拉起本机程序**：网易云网页版会引导“去客户端播放”（`orpheus://`），这台电脑装了网易云客户端。WebView2 默认会弹“要打开这个应用吗”，窗口隐藏时没人点得掉。现在接 `LaunchingExternalUriScheme`（`ICoreWebView2_18`）一律取消并记日志（DSH 访客策略：拒绝特殊协议）。实测中有一次在网易云点“播放”后整个页面卡住约 60 秒、之后每次打开都要 60 秒（换新的浏览器视图后复现不出来，原因没坐实），加了这道拦截后连点 4 次都是 0.9 秒返回。
- **网易云页面时不时把自己的主线程占满**：同一套操作，有几轮每次执行脚本 0.3 秒，有几轮（多在新的浏览器数据、第一次打开网易云时）每次要 6–19 秒、偶尔超过 20 秒报“浏览器没有回应”。日志显示是页面忙，不是调用丢了；很可能是网易云页面自己在同步等网络请求。没有修，放进试玩清单观察。
- 经过模型的“我要听晴天”（`browser_real.py --music-only`，2026-11-21 再跑一次）：先读 `music-web` 技能，网易云要登录就交给玩家；脚本 20 秒后替玩家点“好了”但没真登录，她判断“周杰伦在网易云没版权”改去 QQ 音乐、再去酷狗，每个新网站先问，最后照实说没放成、请玩家登录。技能补了一句：点了“好了”还没登上就再请一次，只有没版权或玩家要求才换网站。
- 已知缺口：导航检查只管顶层页面，iframe 里的请求不拦（DSH 记录里也写了这不是完整的内网防火墙）；域名解析检查只在 `browser.open` 时做一次（解析和连接之间仍有时间差）；酷狗、酷我、QQ 音乐的“下一首”按钮选择器没有真机核对过，对不上时按 title / aria-label 里的“下一首”找。

## 参照

- DSH 浏览器插件（都是 MIT）：`omdsh-dev/dsh-ego-browser`（`runtime/ego-linux/src/snapshot.mjs`、`src/snapshot-diff.ts`、`transport.mjs`、`captcha.ts`）、`omdsh-dev/dsh-browser`（`security/untrusted.ts`、`content/privacy.ts`）、`xylt369/dsh-browser`（`url-guard.ts`）、`wqty123/dsh-browser`（`docs/limitations.md`）。DSH 核心：`deepseek-ai/deepseek-harness` `docs/subsystems/browser-use.md`。
- Playwright MCP（Apache-2.0）：https://github.com/microsoft/playwright-mcp
- WebView2：`ICoreWebView2::CallDevToolsProtocolMethod`，https://learn.microsoft.com/microsoft-edge/webview2/how-to/chromium-devtools-protocol
- Tauri 2：没有匹配任何 capability 的 webview 不能访问 IPC（https://v2.tauri.app/reference/acl/capability/）；`WebviewWindowBuilder::data_directory`。
