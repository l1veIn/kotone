# 2026-11-23 会话交接：英雄联盟工具（ADR 035）做完、游戏事件（ADR 036）待定

接手前先读：本文、[ADR 035](../adr/035-lol-tools.md)、[ADR 036](../adr/036-game-state-and-events.md)；三份调研 [lol-live-data-akari](2026-11-23-lol-live-data-akari.md)、[lol-data-sources](2026-11-23-lol-data-sources.md)、[game-events](2026-11-23-game-events.md)。

## 一、现在的状态

| 东西 | 状态 |
| --- | --- |
| `origin/main` | `b89520a7` = tag `v0.2.0-rc.5`（音乐页和 `music` 工具、可编辑左侧栏、CS2 内置配置）。CI 和 Release Windows 都是绿的，草稿预发布里有 setup.exe、.sig、latest-cos.json、SHA256SUMS，**还没发布** |
| `claude/adr-035-lol-tools` | 从 main 分出，**没有合并**。推送到 origin 的是前四个提交：ADR 035 和调研、英雄联盟工具代码、ADR 036 草案、真人对局实测记录。**之后本地又有三个提交，没推送**（见第六节）：`4f3b4a98` 随机英雄的噪音和不开游戏的真模型测试、`38ee787d` ADR 036 玩家确认、`5dbf0d56` ADR 036 一期 |
| 工作区里不是我的改动 | `.gitignore`（改动）、`.codegraph/`、`scripts/manual-e2e/__pycache__/`：别提交，别动 |

## 二、这轮做了什么（ADR 035）

玩家原话：“琴音，帮我查一下这把游戏选手们的战绩，告诉我对面谁是突破口”“帮我记录一下对面的召唤师技能。谁没闪？帮我发在聊天框”。

- **新 crate `crates/kotone-lol`**：
  - `auth`：从 `LeagueClientUx.exe` 的命令行读端口和令牌；
  - `client`：LCU 和局内接口（2999），只发 GET、只连 127.0.0.1；
  - `gamedata`：客户端自带的中文资料，有英雄的称号、名字和英文 id，召唤师技能的冷却，装备，海克斯，队列；
  - `live`：局内数据；
  - `history`：战绩、隐私、段位；
  - `scout`：统计和突破口；
  - `spells`：手动计时，模式急速加明朗之靴；
  - `text`：写成给她看的文字；
  - 例子 `examples/render_probe.rs`：用探测结果离线跑一遍三个工具。
- **`kotone-platform-windows/src/process_cmdline.rs`**：不提权读别的进程的命令行。
- **工具**（`crates/kotone-agent/src/lol.rs`，主会话用，马上返回）：
  - `lol.match`：这局的样子，以及谁没闪、谁没带闪；
  - `lol.spell`：玩家说“阿狸闪了”就记下，返回 `chat`，例如“阿狸 闪 14:32好”；
  - `lol.scout`：10 个人的战绩和突破口。
- **桌面端 `apps/desktop/src-tauri/src/lol.rs`**：
  - 客户端开着时每 2 秒看一次对局阶段；
  - 进加载界面就查战绩，同时 2 个、每人 20 局；
  - 这局结束时清掉技能记录。
- **设置** `lol.enabled`（默认开）；“游戏列表 → 英雄联盟”配置页有开关和风险提示。
- **玩家改了三处**：
  - 不用那么谨慎，加提示就行；
  - 冷却先试接口；
  - 查全部模式，隐藏战绩照读并标出来。
- **检查**：
  - 工作区 clippy 没有警告，fmt 通过；
  - 改到的 crate 的测试全过（kotone-lol 23 个）；
  - svelte-check 0 错误；Playwright 144 个全过。

**实测（只读，没按键）**：

- 训练模式：不提权读得到命令行；国服不写 lockfile；局内接口 26 个路径都没有冷却字段；`championName` 是称号（“刀锋舞者”）；海斗队列 id 是 2400。
- 一局真人海斗：进了对局后，10 个人的 puuid、英雄、召唤师技能、战绩都拿得到，每人约 1.3 秒；用真实数据跑三个工具的文字，结果正常。

## 三、下一步（按顺序）

1. **小修（ADR 035）**：随机英雄的模式（大乱斗 450、海斗 2400）不算“这个英雄最近没玩过”，标签和突破口都去掉。实测时几乎每个人都被标了，是噪音。改 `scout::tags` / `weak_spots` / `text::render_scout`，加一个“随机英雄”的参数，依据是队列 id，或者 `queues.json` 里的 `pickMode = AllRandomPickStrategy`。
2. **真模型测试，不用开游戏**：
   - 给 `lol.rs` 加两个环境变量。`KOTONE_LOL_LCU=<端口>:<令牌>` 跳过读命令行，`KOTONE_LOL_LIVE=<地址>` 改局内接口地址。
   - 用一个小服务把 `target/lol-probe/<时间>/` 里录好的 JSON 放出来。
   - 照 `scripts/manual-e2e/music_real.py` 写 `lol_tools_real.py`：隔离的 KOTONE_HOME，真 DeepSeek，经 WebView2 调试端口打字。问这几句，检查她调了哪个工具、怎么说：
     - “查一下这把的战绩，对面谁是突破口”；
     - “对面丽桑卓闪了”；
     - “谁没闪”；
     - “发出去”，这一句要禁掉真按键的工具，只看她想不想调 `game_chat.send`。
   - 录好的 JSON 里有真实玩家名，只放在 `target/`，**不提交**；要提交的测试数据先把名字和 puuid 换掉。
3. **还没测的**（要玩家开局）：
   - 选英雄阶段（`lol_probe.py --watch 900` 在开局前就跑起来）；
   - 加载界面那几秒；
   - 隐藏战绩；
   - 开黑分组对不对，问玩家这局是不是和卡尔萨斯、布兰德一起开黑的；
   - 国服聊天有没有时间戳。
4. **ADR 036（游戏状态和事件）**：等玩家确认，尤其是下面三条：
   - D7：游戏一启动就自动切到对应的游戏配置，热词跟着变；
   - D8：自动化的第三种动作“让她做”，每次触发花一次 AI 调用；
   - D9：CS2 / Dota 2 要往游戏目录写 GSI 配置文件。

   确认后做一期：游戏中心、`generic` 和 `lol` 两个适配器、复盘改成订阅、`game.state` 工具、自动化的 `on` 和“让她做”、界面。
5. **知识类工具**（另写 ADR 037，见 lol-data-sources 调研）：
   - 腾讯 101 / 掌盟接口：国服的峡谷、大乱斗、海斗的强度、克制、出装；
   - czgreat/aramhex-data：国服海斗“英雄 × 海克斯”胜率，CC BY-NC，要署名；
   - OP.GG 官方 MCP：全球数据的备用，有对线指南。用之前照 Glyph 的做法在 opgginc/opgg-mcp 提 issue 问一声，**这是对外发言，要玩家点头**；
   - 外号用腾讯 `hero_list.js` 的 `keywords`。
6. **要玩家一句话才能做的**：
   - 发布 rc.5 草稿；
   - 把 `claude/adr-035-lol-tools` 合并进 main；
   - 出下一个 RC。

## 四、更早留下的

- **rc.5 音乐没验证的**：
  - 非 VIP 账号跳过 VIP 歌和试听片段；
  - FLAC；
  - 她说话时音乐压低，听起来怎么样；
  - 放音乐时的回声消除；
  - 网易云停用 eapi 取链接时的备用接口。
- **issue #21**：英文界面标题栏还写着“琴音”。

## 五、规矩（详见 `~/.claude/projects/C--Users-A-Desktop-kotone/memory/`）

- **玩家在这台电脑上打游戏**：
  - 编译、跑测试前先看 `tasklist` 里有没有 `League of Legends.exe` 这类游戏进程，有就不做重活。2026-11-23 晚上英雄联盟加上几个后台代理把内存吃满过一次。
  - 前台是游戏时不发任何按键；真模型测试禁掉会按键的工具。
- **不替玩家补按键**（ADR 032）。
- **不读游戏内存、不注入**。英雄联盟只用客户端和游戏自己提供的本机接口。
- **有风险的功能照做、默认开、加风险提示**，玩家说过“我们是开源软件，感觉没必要这么谨慎”。
- **先找成熟项目照着做**：LeagueAkari（`dev` 分支现在是 MIT，不要拷 v1.3.x–v1.4.3，那几版是 GPL）、Overwolf、Home Assistant……
- **测试和 Key**：用隔离的 `KOTONE_HOME`；真实 DeepSeek Key 只放内存，不打印、不写文件。
- **git**：
  - 只提交自己的文件，不用 `git add .`；每个提交都要能单独编译；
  - 推送、合并、打 tag 要玩家明说。这次推送的是“代码可以推送”，指的是分支。
- **界面文字**：放在 `apps/desktop/messages/zh-CN|en/*.json`，用词照 `docs/glossary.md`，由 `e2e/terminology.spec.ts` 守着。
- **跑 Rust 测试**：要 `TMP=TEMP=<仓库>\target\tmp`，并关掉沙盒。

## 六、同一天后半段（第二轮会话）

**做了**（都只在本地提交，没推送）：

1. `4f3b4a98`：
   - 随机英雄的模式（`queues.json` 的 `pickMode = AllRandomPickStrategy`，读不到时按 450 / 720 / 900 / 2400）不再标“这个英雄最近没玩过”“常玩这个英雄”，也不算进突破口；
   - `KOTONE_LOL_LCU=<端口>:<令牌>`、`KOTONE_LOL_LIVE=<本机地址>`；
   - `scripts/manual-e2e/lol_tools_real.py`：假客户端放录好的海斗，真 DeepSeek，两遍四句工具都用对了（结果见 ADR 035 实测）。
2. `38ee787d`：ADR 036 玩家确认，三条都选了推荐的那一项。
   - D7：自动切游戏配置；
   - D8：“让她做”可以，模板默认关；
   - D9：GSI 文件**默认就写**。
3. `5dbf0d56`：ADR 036 一期。游戏中心、`lol` 适配器、复盘改成订阅 `game.match_ended`、`game.state`、自动化的 `on` 和“让她做”、界面。
   - 实现记录和与草案不同的地方见 ADR 036 末尾；
   - `scripts/manual-e2e/game_events_real.py`：假客户端按时间线放一局，11/11。

**发现的事**：

- `lol_tools_real.py` 第一遍没禁 `game.focus`，她在“发出去”前先调了它，把玩家正在打的英雄联盟切到了前台（不是按键）。之后会动真实游戏窗口的工具也禁了。
- 她说突破口时会用“最值得打”“全队最惨”“脆又爱送”这类评价性的词，比工具说明里“说成信息”重一点。要不要改工具说明，玩家还没表态。
- 复盘有一次模型没回 JSON。现在解析失败时会记日志，再遇到能看出原因。

**下一步**：

1. **要玩家开局**：
   - 真客户端下游戏中心的阶段和局内事件（`game.state`、日志里的 `game: lol …`）；
   - D7 切配置以后热词是不是跟着变；
   - 真对局里的复盘；
   - 第三节第 3 条里还没测的那些（选英雄阶段、加载界面、隐藏战绩、开黑分组、国服聊天时间戳）。
2. **ADR 036 二期**：
   - `cs2` / `dota2` 适配器，GSI 文件默认写（D9）；
   - 外部 Agent 订阅事件（工具服务推送）；
   - `match` 的更多运算；
   - 按功能的“能不能收到”（绿 / 黄 / 红）。
3. **知识类工具**（ADR 037），同第三节第 5 条。
4. **要玩家一句话才能做的**：
   - 推送这三个本地提交；
   - 合并进 main；
   - 发布 rc.5 草稿 / 出下一个 RC。

**规矩补一条**：玩家 2026-11-23 说可以在对局时编译，条件是低优先级（BelowNormal）、`-j 2`。不发系统按键的真模型测试也可以对局时跑（`--while-playing`）。还是不能在游戏在前台时发任何系统按键。

