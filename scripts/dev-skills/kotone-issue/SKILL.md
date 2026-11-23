---
name: kotone-issue
description: 把玩家用 Kotone 时遇到的问题（不对劲的地方、想要的功能）整理成 GitHub issue，提交到 Kotone 的仓库 l1veIn/kotone。玩家说“提个 issue”“把这个问题报上去 / 提交到 GitHub”“记个 bug 给开发”时用。先写草稿给玩家看，玩家说可以才提交；要本机装了 gh 并登录过。
---

# 给 Kotone 提 issue

仓库 `l1veIn/kotone` 是**公开**的：提交上去的东西谁都看得到，所以先给玩家看，他说可以才提交。

## 分工

1. **主会话**：读完这个技能就用 `agent.start` 交给后台（`name` 如 `kotone-issue`，`label` 写“提 issue”）。`task` 里写清楚：
   - 玩家的原话；
   - 这段对话里刚才发生了什么：你做了什么、哪一步不对、玩家本来想要什么（只写对话里真有的，不要编）；
   - 大概什么时候（玩家说“刚才”就是最近 15 分钟）；是不对劲（bug）还是想要新功能（enhancement）。
2. **后台第一轮**：只写草稿，**不提交**（见下文）。
3. **草稿交回来以后，主会话问玩家**：打字时把标题和正文原样给他看；通话里只念标题和一句概括，说“全文在卡片里”。问“要提交吗？”。
4. 玩家说可以：`agent.followup` 交给同一个后台“提交”。要改：把要改的地方 followup 过去，改完再问一次。**没问过玩家不提交。**
5. 提交以后告诉玩家编号（#123），打字时附上链接。

## 后台第一轮：写草稿

1. **一条命令查齐**：`shell.run` 跑下面这段（purpose：“看看版本、刚才的日志和有没有人提过”）。先把 `$search` 换成两三个关键词；问题发生得更早就把 `$minutes` 调大。读过日志和 GitHub 上的内容以后，再跑命令每次都要先问玩家，所以**一次查齐，查完直接写草稿**，不要再跑别的命令。

   ```powershell
   $minutes = 20
   $search = '<两三个关键词>'
   $k = if ($env:KOTONE_HOME) { $env:KOTONE_HOME } else { Join-Path $env:USERPROFILE '.kotone' }
   $lines = Get-Content -LiteralPath (Join-Path $k 'kotone.log') -Tail 4000 -Encoding UTF8
   $me = [regex]::Escape($env:USERPROFILE)
   'version: ' + (($lines | Where-Object { $_ -match 'startup: version=' } | Select-Object -Last 1) -replace '^\[[\d.]+\] ', '')
   'windows: ' + (Get-CimInstance Win32_OperatingSystem).Caption + ' ' + [Environment]::OSVersion.Version
   "== 最近 $minutes 分钟的日志 =="
   $since = [DateTimeOffset]::Now.AddMinutes(-$minutes).ToUnixTimeSeconds()
   foreach ($l in $lines) {
     if ($l -match '^\[(\d+)\.\d+\] (.*)$' -and [int64]$matches[1] -ge $since) {
       '{0:MM-dd HH:mm:ss} {1}' -f [DateTimeOffset]::FromUnixTimeSeconds([int64]$matches[1]).ToLocalTime(), ($matches[2] -replace $me, '%USERPROFILE%')
     }
   }
   '== GitHub 上有没有人提过 =='
   gh issue list --repo l1veIn/kotone --state all --search $search --limit 10 2>&1
   ```

   - **日志**：每行是“时间 模块: 内容”，用户目录已经换成了 `%USERPROFILE%`。挑和这个问题有关的 5–20 行（报错、失败、相关模块），不要整段贴；没有相关的就写“没有找到相关日志”，不要去别处找。
   - **gh 报错**（“无法将‘gh’项识别为…”或要先 `gh auth login`）：停下，总结里告诉玩家装 GitHub CLI（`winget install GitHub.cli`），自己在终端里运行 `gh auth login`。**不要向玩家要 Token 或密码。**
   - **有人提过同一件事**（看标题判断）：总结里给出它的编号、标题和开着还是关了，问玩家要不要在那条下面补充，而不是新开（补充用 `gh issue comment <编号> --repo l1veIn/kotone --body-file <文件>`，同样先写好给玩家看）。
2. **写草稿**：`file.write` 写到 `issues/<年月日-时分>.md`（在 Kotone 的文件区，不用问），格式见下文。
3. **总结**（交回主会话）：标题、标签、草稿文件名，再附上完整正文，让主会话能原样给玩家看。最后一句写“还没提交，等玩家确认”。

## 后台第二轮：玩家说提交

`shell.run`（purpose 就写“提交 issue 到 GitHub”，不要带标题：太长会被截断）。这一步可能会再问玩家一次，正常：

```powershell
gh issue create --repo l1veIn/kotone --title "<标题>" --body-file "issues\<文件名>.md" --label <bug 或 enhancement>
```

输出的最后一行是链接，总结里写编号和链接。标题里不要有英文双引号（PowerShell 会断开），要引用就用「」。

## 草稿格式

- 标题：一句话说现象（或想要的功能），中文，不超过 40 个字。
- 标签：不对劲的用 `bug`，想要新功能的用 `enhancement`。
- 正文：

```markdown
## 现象
（玩家看到了什么）

## 期望
（本来应该怎样）

## 怎么出现的
1. …
2. …

## 环境
- Kotone <版本，取日志 startup 那行的 version>
- <Windows 版本>
- 当时：<通话中 / 打字 / 在玩什么游戏>

## 日志摘录
（5–20 行，没有相关的就写“没有找到相关日志”）

---
琴音根据玩家的描述整理。
```

## 公开仓库：这些不写进去

- Key、Token、密码、Cookie（`sk-` 开头的、很长的一串字母数字）。
- 用户目录：`C:\Users\<名字>` 一律写成 `%USERPROFILE%`。
- 玩家和队友的名字、游戏 ID、QQ 号、聊天内容。玩家说过的话只留和这个问题有关的一句。
- 拿不准能不能公开的，就不写。
