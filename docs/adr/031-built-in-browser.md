# ADR 031：内置浏览器与浏览器操作——在后台替玩家操作网页

状态：Proposed / 方向已确认（2026-11-20），0.2.0 之后实现，先做原型验证。相关：[ADR 019](019-game-computer-use.md)（操作游戏）、[ADR 029](029-voice-call-and-background-tasks.md)（后台任务）、[ADR 030](030-subagent-shell-and-files.md)（读过外面的东西先问）。调研：[常见音乐软件](../research/2026-11-20-music-apps.md)。

## 背景

玩家想说“我要听晴天”。调研发现点歌要么靠各家未授权的搜索接口查歌曲编号，再用随版本变化的链接或命令行让客户端播放；要么像 GitHub 上的几个网易云 MCP 那样自己当播放器。而且网易云客户端收到播放链接后会把主窗口拉到前台，打断游戏。

已有的操作电脑（ADR 019）是给游戏用的：截图看画面，模拟真实的键盘鼠标，要求目标窗口在前台。玩家在游戏里时，它没法去操作别的程序。

浏览器正好补上：网页本身就有搜索框和播放按钮，用户怎么点，Agent 就怎么点；而且网页可以在 Kotone 自己的隐藏窗口里，完全不碰前台。

| | 操作电脑（ADR 019） | 浏览器操作（本 ADR） |
| --- | --- | --- |
| 操作谁 | 游戏和桌面程序 | Kotone 自己窗口里的网页 |
| 怎么看 | 截图 + 文字识别 | 读网页结构：按钮、输入框、文字，每个元素一个编号 |
| 怎么动 | 模拟真实键鼠，要求在前台 | 经浏览器调试协议直接发给页面，不发系统按键，窗口隐藏也能操作 |

## 决定

### 1. 内置浏览器

- Kotone 里单独一个 WebView2 窗口，平时隐藏，需要玩家时才显示。只有一个页面，不做标签页和地址栏。
- 数据单独存放在 `<KOTONE_HOME>/browser/`（Tauri `data_directory`），和 Kotone 自己的界面分开。
- 这个窗口不在任何权限声明里（Tauri capabilities），网页完全调用不了 Kotone 的功能。
- 设置里一个按钮“清除浏览器登录”，删掉这个目录。

### 2. 浏览器操作工具（照 Playwright MCP）

照微软的 Playwright MCP（`microsoft/playwright-mcp`，Apache-2.0）：默认不截图，把页面整理成可访问性快照，每个可操作的元素带编号，模型按编号操作。工具名和参数照它的形状，只取最少的几个：

| 工具 | 做什么 |
| --- | --- |
| `browser.open {url}` | 打开网址 |
| `browser.snapshot` | 读页面：带编号的元素清单和文字（结果标记为不可信） |
| `browser.click {ref}` | 点击某个元素 |
| `browser.type {ref, text, submit?}` | 在某个输入框里输入，可选回车 |
| `browser.press {key}` | 按一个键（发给页面，不是系统按键） |
| `browser.wait {text?, seconds?}` | 等某段文字出现或等几秒 |
| `browser.handover {reason}` | 交给玩家（登录、验证码、付款），见第 3 节 |

- 实现：WebView2 的 `CallDevToolsProtocolMethod` 在程序内部调用浏览器调试协议（快照、点击、输入都走它），**不开调试端口**，别的程序控制不了这个浏览器。
- 只给后台任务用，主会话没有这些工具（主会话要快）。

### 3. 登录：玩家自己来，扫码优先

照 ChatGPT Agent、Claude in Chrome 的做法：登录、验证码、付款交还给人。

1. 后台任务读页面发现要登录，调用 `browser.handover {reason: "网易云要登录"}`。
2. 主会话告诉玩家“网易云要登录一下，我把窗口打开了，用手机扫个码”，同时显示浏览器窗口。
3. 页面变成已登录（或玩家说“好了”），窗口收起，后台任务接着做。

登录状态留在 `<KOTONE_HOME>/browser/`，重启后还在，过期了再扫一次；玩家是会员的，网页版同样按会员播放。

不做：Agent 不在密码框里输入（代码拦下）；不从玩家的 Chrome / Edge 导入登录信息（要解密别的浏览器的数据，涉及隐私，也容易被安全软件拦）。

### 4. 安全（照 ADR 030 的思路）

- 网页内容都是不可信结果：读过之后，执行命令、写文件前先问一句（ADR 030 的标记照常生效）。
- 第一次打开某个网站先问一句（“要我打开 music.163.com 吗？”），按域名记住。
- 登录、付款、替玩家发帖评论下单：交给玩家（`browser.handover`）。

### 5. 第一个用处：点歌

- 每个音乐网站一份技能（网易云、QQ 音乐、酷狗、酷我）：怎么判断有没有登录、怎么搜、怎么播放第一条。只是说明，不写代码。
- “换一首、暂停、这首歌叫什么”仍走系统媒体控制（SMTC）；网页播放器没接入时，技能里写怎么点它的按钮。
- 玩家在用桌面客户端放歌时，不另开网页放（避免两边一起响）。

## 不做的

- 不开浏览器调试端口；不做通用浏览器界面（标签页、地址栏、书签）。
- 不导入其他浏览器的登录信息；不替玩家输入密码、付款。
- 不截图识别（以后真遇到只能看图的页面，再照 Playwright MCP 的 vision 选项加）。

## 原型先验证

1. 窗口隐藏后，网页里的音乐会不会被暂停或降频。
2. 网易云、QQ 音乐网页版登录后，常见的歌能不能直接播放；扫码登录在这个窗口里能不能完成。
3. 网页播放器有没有接入系统媒体控制。
4. 单独的数据目录和“网页调用不了 Kotone”在 Tauri 里确实生效。
5. `CallDevToolsProtocolMethod` 在 Tauri 里怎么调：2020 年有人反映只能在 WebView 回调里调用；我们踩过在 `with_webview` 回调里发事件导致死锁的坑（见开发记录），要避开。

## 参照

- Playwright MCP（Apache-2.0）：https://github.com/microsoft/playwright-mcp ——“LLM-friendly. No vision models needed, operates purely on structured data.”
- WebView2：`ICoreWebView2::CallDevToolsProtocolMethod`，https://learn.microsoft.com/microsoft-edge/webview2/how-to/chromium-devtools-protocol
- Tauri 2：没有匹配任何 capability 的 webview 不能访问 IPC（https://v2.tauri.app/reference/acl/capability/）；`WebviewWindowBuilder::data_directory`。
