# 2026-11-20 常见音乐软件：怎么发现、怎么控制、怎么点歌

玩家想说“换一首”“暂停”“这首歌叫什么”“我要听 xxx”。这里调研网易云音乐、QQ 音乐、酷狗、酷我、汽水音乐、Spotify、Apple Music（Windows）。只读资料和源码，没有在玩家电脑上运行这些软件。

标记：**V** 有一手依据（官方文档、源码、安装清单或我们自己的测试）；**V3** 第三方源码或实测记录；**U** 未证实，要真机验证。

## 结论

分三层，越往下越依赖具体软件、越容易随版本失效：

1. **发现**：系统媒体控制（SMTC）列出所有正在播放的软件；再按进程名认出没接入 SMTC 的。
2. **控制和“正在放什么”**：只用 SMTC 就能覆盖绝大多数软件（Lyricify 只认 SMTC，把网易云、QQ、汽水、Spotify 评为完全支持）。少数软件要在它自己的设置里打开“系统播放控件”。
3. **按歌名点歌**：每个软件各不相同，大多要用未授权的搜索接口拿歌曲编号，再用它自己的链接或命令行让客户端播放。脆弱、随版本变，部分做法会把客户端窗口拉到前台（抢游戏焦点）。

**不做**：注入 DLL、给客户端打补丁、读写其他进程内存、用登录态伪装客户端调接口。游戏反作弊同时运行时，这些行为可能被判可疑；也都随版本失效。

## 一、各软件一览

| 软件 | 进程 | 安装检测 | 链接协议 | SMTC 来源 id / 开关 |
| --- | --- | --- | --- | --- |
| 网易云音乐 3.x | `cloudmusic.exe`（CEF） | 卸载项 `网易云音乐`，整机安装 V | `orpheus` → `cloudmusic.exe --webcmd="%1"`（本机 V） | `cloudmusic.exe`；3.1.21 起设置里有“开启 SMTC”V3，默认开否 U |
| 汽水音乐 | `SodaMusic.exe`（定制 Electron），`Chrome_WidgetWin_1`，主窗口标题为空 V3 | HKCU 卸载项 `{87285ABF-…}_is1`，用户级 V | `luna` → `SodaMusic.exe "%1"`（本机 V） | `汽水音乐`（中文显示名）V3；能读歌名、歌手、封面、进度 |
| QQ 音乐 | `QQMusic.exe`（32 位），主窗口 `TXGuiFoundation` V | `HKLM\…\Uninstall\QQMusic` V | 没有自己的歌曲协议（`tencent://` 属于 QQ）V | ≤22.76 `QQMusic.exe`，≥22.82 `Tencent.QQMusic` V3；开关在“设置 → 通用设置 → 通知 → 显示 SMTC”V |
| 酷狗 | `KuGou.exe`，窗口 `kugou_ui` V | `HKCU\Software\KuGou`；卸载项按显示名“酷狗音乐”找 V | `kugou`（用途见第三节）| `kugou` / `KuGou.exe` V；开关“设置 → 常规 → 播放 → 支持系统播放控件”，没有进度信息 V |
| 酷我 | `KwMusic.exe`（发声的是子进程 `KwService.exe`），窗口 `kwmusicmaindlg` V | `HKLM\…\Uninstall\KwMusic` V | `kuwo` V | 没找到支持 SMTC 的证据 U |
| Spotify | `Spotify.exe` | — | `spotify` | `Spotify.exe`，完整支持 V |
| Apple Music | 包 `AppleInc.AppleMusicWin` V | — | 没有可用的协议 U | 支持，但曲目信息不全 V |
| 浏览器 | — | — | — | Chrome 整个浏览器一个会话（PWA 各自一个）V |

本机（2026-11-20）：网易云音乐 3.1.24、汽水音乐 3.1.0；协议 `orpheus`、`luna` 已注册。

## 二、控制：暂停、继续、上一首、下一首、正在放什么

**主路线：SMTC**（`Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager`，Win10 1809+）V
- `GetSessions()` 列出全部会话；`GetCurrentSession()` 是系统认为最该控制的那个；`SourceAppUserModelId` 认软件；`TryGetMediaPropertiesAsync` 读歌名歌手；`GetPlaybackInfo` 读状态和能力（`IsNextEnabled` 等）；`TryTogglePlayPauseAsync` / `TrySkipNextAsync` / `TrySkipPreviousAsync` 控制。
- Rust：`windows` 0.61 的 `Media_Control`（加 `Foundation_Collections`）；异步结果可以直接 `.await`；放在 tokio 线程上，不在界面主线程上阻塞。V
- 选哪个会话（照 Home Assistant `homeassistant/components/media_player/intent.py`）：优先正在播放的；没有就用 `GetCurrentSession()`；“继续播放”优先选刚才被我们暂停的那个。
- 汽水卡住时读取可能一直不返回，所有调用都加超时。V3
- 发出命令后等 `MediaPropertiesChanged` / `PlaybackInfoChanged` 再说“换好了，现在是《xxx》”，超时就说没换成。

**没有 SMTC 会话时**：
- 告诉玩家去这个软件的设置里打开“系统播放控件 / SMTC”（位置见上表），这是最简单可靠的办法。
- 不靠前台的兜底（各软件自己的）：网易云对主窗口发 `WM_HOTKEY`（热键编号用 `GlobalFindAtom("next_local")` 等查）V3；酷狗对 `kugou_ui` 发 `WM_HOTKEY` 0x40A–0x40D V3；QQ 音乐 `QQMusic.exe /playcontrol 'next'` V3；酷我只有 F5 播放 / 暂停 V3。
- 最后一层：系统媒体键（`SendInput` `VK_MEDIA_*`）。它控制的是 SMTC 的当前会话，而且前台是以管理员运行的程序时会被系统悄悄拦下。V

## 三、点歌：“我要听 xxx”

两步：用歌名查到这个软件里的歌曲编号；再让客户端播放这个编号。

| 软件 | 查编号 | 让客户端播放 | 注意 |
| --- | --- | --- | --- |
| 网易云 | 官方：网易云音乐开放平台（要申请 appId / 私钥，NetEase/skills `ncm-cli`）V；非官方：`music.163.com/api/cloudsearch/pc` 匿名可用，但版权没有的歌只搜到翻唱 V | `orpheus://` + base64(`{"type":"song","id":…,"cmd":"play"}`)，网页版 JS 里就是这么拉起客户端的 V；3.1.24 认不认 U | 客户端收到后会把主窗口恢复出来，**可能抢游戏焦点** V3；NeteaseCloudMusicApi 2024 年收到律师函后归档 V |
| QQ 音乐 | 官方：`tencentmusic/qqmusic-skills`（Apache-2.0），要玩家自己的 API Key，返回 songMid V；songMid 换数字 songid 要非官方接口 V3 | `QQMusic.exe /playbysongid "cmd_count==1&&id_0==<songid>&&songtype_0==0"`，有项目实测能直接播放 V3 | 22.82 改了来源 id 和渲染层 |
| 酷狗 | 非官方 `mobilecdn.kugou.com/api/v3/search/song` 拿 hash V3 | 读共享内存找到窗口，再发 `WM_COPYDATA`，20.0.81 实测后台播放 V3 | 只在一个版本上验证过 |
| 酷我 | 非官方搜索接口，要 csrf，现在能不能用 U | `kuwo://play/?…musicrid0=<base64("MUSIC_<rid>")>…`，收到后是否自动播放 U | 证据最少 |
| 汽水 | 没有干净的来源：PC 搜索接口要客户端签名，移动端要登录 Cookie V3 | `luna://luna.com/playing?track_id=<id>&media_type=track`，3.7.0 测过 V3 | 整个链接要用 ShellExecute 打开，否则 `&` 会截断 |
| Spotify | 官方 Web API 能搜能播（需会员），但开发者政策 III.3 **禁止用它做语音控制 Spotify**（2025-05-15 起）V | `spotify:track:` 打开后会不会自动播放 U | 只能走链接或界面 |
| Apple Music | 没有公开接口（Apple DTS：“no supported way”）V | — | 只能控制，不能点歌 |

**最通用的兜底**：打开软件、点进搜索框、输入歌名、播放第一条（Kotone 已有的“操作窗口”能力）。所有软件都能用、不碰未授权接口，但一定会把音乐软件拉到前台，在游戏里会打断玩家。

## 四、成熟参考

- **current-song2 / `win-gsmtc`**（Rust，MIT OR Apache-2.0）：只用 SMTC，在 tokio 线程上推送会话变化，按来源过滤。
- **WindowsMediaController**（C#，MIT）：SMTC 封装。
- **Lyricify**：只认 SMTC；每个软件一页“怎么打开系统播放控件”的说明。
- **WebNowPlaying**（MIT）、**OBS Tuna**（GPL-2.0）：SMTC 为主，窗口标题等兜底。
- **Home Assistant** `HassMediaSearchAndPlay`：先搜索再播放第一条；下一首 / 暂停只对正在播放的播放器。
- **xiaotang-daozai**（MIT）：网易云、QQ、酷狗、酷我都覆盖，SMTC 优先，找不到会话再用各家的窗口消息 / 命令行。
- 没有许可证、只能参考不能抄：AwooMusicBot、QQMusic-mcp、top-island。

## 五、要在玩家电脑上实测的

1. 网易云 3.1.24、汽水 3.1.0：放歌时 SMTC 里有没有会话、能不能读歌名、能不能切歌（只读和控制，会出声）。
2. `orpheus://` base64 链接能不能直接播放；会不会把网易云窗口拉到前台。
3. `luna://` 播放链接能不能直接播放；会不会拉到前台。
