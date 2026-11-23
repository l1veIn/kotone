# 2026-11-23 网易云音乐非官方接口调研（ADR 034 的依据）

起因：内置浏览器放歌（ADR 031）慢、意外多，玩家决定音乐收敛成工具，先只做网易云，用非官方接口 + 扫码登录（“我们是非盈利的纯开源项目……怎么好用怎么来”）。这里记调研到的事实，2026-11-23 用 `gh`、crates.io、npm 核对过源码和 issue。

## 一、上游和分支

- **Binaryify/NeteaseCloudMusicApi** 已归档，仓库只剩一句“保护版权,此仓库不再维护”（最后推送 2024-02-28）。npm 包还在发版，但源码不公开。
- **[NeteaseCloudMusicApiEnhanced/api-enhanced](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced)**：现在维护的分支，MIT（© 2013-2022 Binaryify），2026-11-22 还在提交。加密常量在 [util/crypto.js](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/blob/main/util/crypto.js)，请求怎么拼在 [util/request.js](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/blob/main/util/request.js)。

## 二、加密

| 方式 | 做法 | 用在哪 |
| --- | --- | --- |
| weapi | 两次 AES-128-CBC（预置键 `0CoJUm6Qyw8W8jud`，再用 16 位随机键），IV `0102030405060708`；`encSecKey` = 反转随机键后做无填充 RSA（e=0x10001，1024 位模数见 Enhanced 源码） | 每日推荐、用户歌单、私人 FM、红心 |
| eapi | AES-128-ECB，键 `e82ckenh8dichen8`，大写十六进制；`digest = md5("nobody"+url+"use"+text+"md5forencrypt")`，`data = url+"-36cd479b6b5-"+text+"-36cd479b6b5-"+digest`；POST 到 `interfacepc.music.163.com/eapi/…`，JSON 里带 `header`（os、appver、deviceId、`__csrf`、MUSIC_U / MUSIC_A），同样放进 Cookie | 扫码登录、搜索、歌词、歌单内容、喜欢列表 |
| linuxapi | AES-128-ECB，键 `rFgB&h#%2?^eDg:Q`，`eparams` 包一层转发到 `/api/linux/forward` | 旧接口 |
| xeapi（2026 新） | X25519 + AES-GCM，密钥从 `/api/bsr/sk/get` 取，安卓身份 | Enhanced 自 2026-05-29 起取播放链接（`song/url/v1`）、匿名注册、`like/v1` 用它 |
| neapi（2026-09 新） | ChaCha20-Poly1305 + zstd | — |

**风险**：取播放链接已经被网易云往更难的加密上推。eapi 版的 `song/url/v1` 现在还能不能用要真机验证；退路有 `/song/download/url/v1`（有用户反馈 -460 时它还能用）和匿名的 `music.163.com/song/media/outer/url?id=`（只有免费歌）。

## 三、扫码登录（2025–2026）

1. `/api/login/qrcode/unikey`，`{type:3}`（eapi）→ `unikey`；
2. 二维码内容 `https://music.163.com/login?codekey={unikey}`（网页平台再加 `&chainId=v1_{deviceId}_web_login_{毫秒}`，修了“设备环境异常”）；
3. 轮询 `/api/login/qrcode/client/login`，`{key, type:3}`：**800** 过期、**801** 等扫码、**802** 扫了等确认、**803** 成功（Set-Cookie）。

- 会话 Cookie：`MUSIC_U`（凭证）、`__csrf`。扫码登录的 Cookie 不能用 `/login/refresh` 续。
- 每个请求带默认指纹：`os=pc`、`appver=3.1.17.204416`、`osver=Microsoft-Windows-10-…`、`channel=netease`、52 位十六进制 `deviceId`、匿名 `MUSIC_A`、`NMTID`。
- **旧的 weapi `type:1` 已经被风控**：扫完返回 `-462 请完成验证操作` 或 `8821`（[YesPlayMusic #2493](https://github.com/qier222/YesPlayMusic/issues/2493)，2026-10-18）；`type:3` 能用。代理 / 有风险的 IP 上第一步就可能 `-462`。
- 密码、短信登录更容易被 `8821 需要行为验证码验证` 拦（[api-enhanced #4](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/issues/4)）；反复登录会“登录存在风险已被拦截”一天以上（[#8](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/issues/8)）。
- 别人用的退路：内嵌 `music.163.com` 登录窗口读 `MUSIC_U`（SPlayer）、手动粘贴 `MUSIC_U`。

## 四、要用的接口

| 要做的 | 路径 | 加密 | 参数 |
| --- | --- | --- | --- |
| 搜歌 | `/api/cloudsearch/pc` | eapi | `s`、`type:1`、`limit`、`offset` |
| 播放链接 | `/api/song/enhance/player/url/v1` | xeapi（Enhanced）/ eapi（旧） | `ids:"[id]"`、`level`、`encodeType:"flac"` |
| 歌词 | `/api/song/lyric` | eapi | `id` |
| 每日推荐 | `/api/v3/discovery/recommend/songs` | weapi | 要登录 |
| 我的歌单 | `/api/user/playlist` | weapi | `uid` |
| 我喜欢的 | `/api/song/like/get` | eapi | `uid` |
| 歌单里的歌 | `/api/v6/playlist/detail` 再 `/api/v3/song/detail`（每次最多 1000 首） | eapi | `id` |
| 私人 FM | `/api/v1/radio/get` | weapi | — |
| 红心 | `/api/radio/like` | weapi | `trackId`、`like` |

- 音质 `level`：`standard`、`higher`、`exhigh`、`lossless`、`hires`…；非 VIP 最高 `exhigh`。
- 能不能放：`fee` 0 免费或没版权、1 VIP、4 付费专辑、8 低音质免费；`privilege.st < 0` 是灰掉的歌；单条 `code ≠ 200` 放不了；VIP 歌给非 VIP 的是试听片段（`freeTrialInfo {start,end}`），YesPlayMusic 跳过这种。
- 播放链接 1200 秒后过期（`expi`）。
- **账号安全**：别反复登录；Enhanced 每个响应缓存 2 分钟防“IP 高频”；非中国 IP `-460`；[#300](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/issues/300)（2026-11-06）1.3 秒里约 30 次取歌词导致整个账号长时间取不到播放链接（-460）。

## 五、Rust 实现（Kotone 是 MIT）

| 项目 | 许可 | 状态 | 扫码 | 说明 |
| --- | --- | --- | --- | --- |
| [ncmapi](https://github.com/two-mountains/ncmapi-rs) | MIT | 1.9.1，2026-11-17 | 有（eapi `type:3`） | tokio + reqwest + rustls，纯 Rust 加密；接口：搜索、播放链接、歌词、每日推荐、喜欢列表、歌单、红心、FM、2 分钟缓存（moka）、Cookie 持久化；MSRV 1.98；依赖 `rsa` |
| [cria-api](https://github.com/alpaca-herd/cria) | MIT | 0.1.1，2026-11-20 | 有 | 约 370 个接口、含 xeapi；但用 compio（不是 tokio）+ OpenSSL |
| [ncm-api-rs](https://github.com/SPlayer-Dev/ncm-api-rs) | WTFPL | 2026-04-07 | 有 | 没有 xeapi |
| gmg137/netease-cloud-music-api | GPL-3.0 | 2026-11-03 | 还是 `type:1` | **只能参考** |
| Mineral | AGPL-3.0 | 活跃 | 有 | **只能参考**；`MusicChannel` trait + `caps()` 能力声明 |

MIT / WTFPL / Apache-2.0 的代码能拷（留署名），GPL / AGPL 只能看。

## 六、可以照着学的播放器

- **YesPlayMusic**（MIT）：按 `fee` / `privilege` 判断能不能放、跳过只有试听的歌、匿名退路 `song/media/outer/url`。扫码登录的 bug（#2469、#2493）还开着。
- **[dsh-netease-music](https://github.com/zeelinkCN/dsh-netease-music)**（MIT，DeepSeek Harness 插件）：给 Agent 的工具是 `netease_play` / `netease_player`，和 Kotone“主会话直接用工具”最像；取链接先 `exhigh` 再 `standard`，标出试听。
- **多来源的结构**：listen1（MIT）每个平台一个类、同一套方法（`search`、`bootstrap_track`、`lyric`、`login`），按 ID 前缀（`ne`、`qq`）分派；MusicFree（AGPL，只参考）插件接口 `search`、`getMediaSource(item, quality)`、`getLyric`；Mineral 的 Rust trait 带 `caps()`。
