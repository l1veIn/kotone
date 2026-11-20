# ADR 031：内置浏览器与浏览器操作——在后台替玩家操作网页

状态：Proposed / 方向已确认（2026-11-20），0.2.0 之后实现，先做原型验证。相关：[ADR 019](019-game-computer-use.md)（操作游戏）、[ADR 029](029-voice-call-and-background-tasks.md)（后台任务）、[ADR 030](030-subagent-shell-and-files.md)（读过外面的东西先问）。调研：[常见音乐软件](../research/2026-11-20-music-apps.md)。

## 背景

玩家想说“我要听晴天”。调研发现点歌要么靠各家未授权的搜索接口查歌曲编号，再用随版本变化的链接或命令行让客户端播放；要么像 GitHub 上的几个网易云 MCP 那样自己当播放器。而且网易云客户端收到播放链接后会把主窗口拉到前台，打断游戏。

已有的操作电脑（ADR 019）是给游戏用的：截图看画面，模拟真实的键盘鼠标，要求目标窗口在前台。玩家在游戏里时，它没法去操作别的程序。

浏览器正好补上：网页本身就有搜索框和播放按钮；网页可以在 Kotone 自己的隐藏窗口里，完全不碰前台。

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

- Kotone 里单独一个 WebView2 窗口，平时隐藏，需要玩家时才显示。只有一个页面，不做标签页、地址栏、书签。
- 数据单独存放在 `<KOTONE_HOME>/browser/`（Tauri `data_directory`，独立的 WebView2 环境），和 Kotone 自己的界面分开。
- 这个窗口不在任何权限声明里（Tauri capabilities），网页调用不了 Kotone 的任何功能。
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

## 原型先验证

1. **隐藏的 WebView2 能不能收到操作**：窗口不可见时，页面内 `click()` 和调试协议的鼠标事件分别能不能生效。
2. **被全屏游戏挡住时**：加了上面的启动参数后，能不能收到操作、音乐会不会被降频或暂停。
3. **自动播放**：页面内 `click()` 不算用户手势，加了自动播放参数后，网易云、QQ 音乐网页版能不能开始放歌。
4. 网易云、QQ 音乐网页版登录后常见的歌能不能直接播放；扫码登录在这个窗口里能不能完成。
5. 暂停、下一首、读歌名能不能直接调用播放页里的播放器做到（不经过后台任务）。
6. 单独的数据目录、单独的启动参数、“网页调用不了 Kotone”在 Tauri 里确实生效；`CallDevToolsProtocolMethod` 怎么调（2020 年有人反映只能在 WebView 回调里调用；我们踩过在 `with_webview` 回调里发事件导致死锁的坑，要避开）。

## 参照

- DSH 浏览器插件（都是 MIT）：`omdsh-dev/dsh-ego-browser`（`runtime/ego-linux/src/snapshot.mjs`、`src/snapshot-diff.ts`、`transport.mjs`、`captcha.ts`）、`omdsh-dev/dsh-browser`（`security/untrusted.ts`、`content/privacy.ts`）、`xylt369/dsh-browser`（`url-guard.ts`）、`wqty123/dsh-browser`（`docs/limitations.md`）。DSH 核心：`deepseek-ai/deepseek-harness` `docs/subsystems/browser-use.md`。
- Playwright MCP（Apache-2.0）：https://github.com/microsoft/playwright-mcp
- WebView2：`ICoreWebView2::CallDevToolsProtocolMethod`，https://learn.microsoft.com/microsoft-edge/webview2/how-to/chromium-devtools-protocol
- Tauri 2：没有匹配任何 capability 的 webview 不能访问 IPC（https://v2.tauri.app/reference/acl/capability/）；`WebviewWindowBuilder::data_directory`。
