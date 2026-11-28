# ADR 051：英雄联盟玩家的 happy path——一个 DeepSeek Key，她就用 DeepSeek；聊天页上“准备好了吗”

状态：Accepted（2026-11-28）/ 已实现（浏览器预览的端到端测试），待真机。

修订：[ADR 028](028-game-companion-interaction.md) 的“一个 Key 打开全部”（从引导和 Key 卡片扩到所有存 DeepSeek Key 的地方）。相关：[ADR 044](044-run-as-admin-by-default.md)（管理员身份）、[ADR 043](043-plugin-shape-and-pushes.md)（推送）、[ADR 050](050-humanlike-speech.md)（这局结束的那句话）。

## 背景

玩家：

> “今天一个老用户使用，配置了 deepseek … 但是默认模型给了一个之前配置的很差的千问。我建议确立一套 happypath，确保每个用户能完整体验产品。初期用户我打算都给英雄联盟玩家推送。”

她用哪条连接是 `Settings::companion_connection_id()`：新用户是 `assistant.connectionId`，0.1.x 升级后启用了她的是 `assistant.companionBindings.connectionId`。引导里填 Key（`connectDeepSeek`）会把她换到新的 DeepSeek 连接，但别的入口不会：「设置 → API 连接」里加一条 DeepSeek 不换她；“她用的模型”列出每条有 Key 的连接，0.1.x 留下来的千问（旧提醒助手用的）也在里面。还有一个错：已经启用她的 0.1.x 用户再走一遍填 Key（聊天页的 Key 卡片、重开引导），写的是 `assistant.connectionId`——那是旧提醒助手的连接，她没换，语音识别反倒被换了。那位老用户走的是哪条路不知道（诊断包里没有 config.json），所以每条路都要对。

## 决定

### happy path（英雄联盟玩家，从安装到打完一局）

1. 安装，打开 Kotone：默认以管理员身份运行（ADR 044，点一次 UAC）。
2. 填一个 DeepSeek Key：她、语音识别（0.1.x 用户除外）、文字识别、语音合成、联网搜索都用 DeepSeek。
3. 设呼叫键（新用户默认 `Ctrl + ~`；0.1.x 用户自己设，推荐 `Ctrl + ~`）。
4. 呼叫她一次，听到她回答。
5. 英雄联盟插件开着（默认开，“读客户端数据”也默认开）。
6. 下一局：选英雄的卡片、游戏里的提醒（到 6 级、关键装备、资源、阵亡……）、这局结束的总结和那句话。

### 规则：一个 DeepSeek Key → 她就用 DeepSeek，不管从哪存的

1. **存了 DeepSeek 的 Key 的每个入口**（引导、聊天页的 Key 卡片、「设置 → API 连接」新加或换 Key 的 DeepSeek 连接、“准备好了吗”里的「换成 DeepSeek」）都走同一个函数 `switchToDeepSeek`（`apps/desktop/src/lib/deepseekSetup.ts`）：她换成这条连接，文字识别、语音合成、联网搜索跟着换；新用户的语音识别也换（自己下好并选了本地识别的除外），0.1.x 用户的语音识别不动。“是不是 DeepSeek”和后端 `Settings::deepseek_connection_id` 同一条规则（服务商是 deepseek，或地址里有 `api.deepseek.com`）。
2. **0.1.x 用户**：已经启用她的只换 `companionBindings.connectionId`，呼叫键、旧提醒助手的连接和键都不动；还没启用她的（选过“继续使用语音输入法”），在「API 连接」里存 DeepSeek Key 和在 Key 卡片里填一样——启用她，落地页改成聊天。
3. **「API 连接」里先测一次**（读模型列表，不花钱）：通过了才换；没通过连接照样存着、测试结果写在卡片上、她不换。存 DeepSeek 时编辑框里不再有“存好后设为她用的”勾选框，换成一行说明。
4. **之后照玩家的**：只在存 Key 的那一下换。玩家在“她用的模型”里自己换成别的，不换回去；“准备好了吗”里会多一行“她用的模型不是 DeepSeek”和「换成 DeepSeek」按钮，点不点由他。

### 聊天页最上面的“准备好了吗”

她能回答以后（有能用的连接；之前聊天页是 Key 卡片，那就是第一步），聊天页最上面一块，一行一样、✓ 或 ✗，每个 ✗ 一个按钮，当场改好或跳到改它的地方；全都 ✓ 就不显示。只用现成的状态，没有新的设置项。

| 行 | ✓ 的条件 | ✗ 时的按钮 |
| --- | --- | --- |
| DeepSeek Key | 有一条存了 Key 的 DeepSeek 连接 | 「填 DeepSeek Key」：打开引导的填 Key 那一步，接着设呼叫键 |
| 她用的模型 | 她的连接是 DeepSeek（没有 DeepSeek 连接时这行并进上一行） | 「换成 DeepSeek」：`switchToDeepSeek` |
| 呼叫键 | 设了 | 「设呼叫键」：角色页「唤醒」 |
| 管理员身份 | `get_elevation_status` 是管理员（不是 Windows 时没有这一行） | 「以管理员身份重启」：打开“启动时以管理员身份运行”再重启 |
| 英雄联盟插件 | 插件开着、“读客户端数据”开着 | 「打开」：当场打开（风险和许可的弹窗照旧在第一次打开插件页时弹） |
| 文字识别 | `companion_status` 的文字识别就绪 | 有 DeepSeek Key 时「用 DeepSeek 文字识别」，没有时「去选文字识别」（「设置 → 模型 → 文字识别」） |

## 不做

- **客户端连上没有**：英雄联盟插件没有便宜的状态可读（要调工具或听事件），这一版不放进清单；连没连上，下一局的选英雄卡片就知道。
- 不加“别再提醒”的开关：全 ✓ 就消失；故意用别家模型的玩家会一直看到“她用的模型”这一行，先这样，听玩家反馈。
- 不在启动时自动把她换成 DeepSeek（只在存 Key 的那一下换）。

## 实现情况

- `deepseekSetup.ts`：`connectDeepSeek` = 建连接 + 测试 + `switchToDeepSeek`；修了已启用她的 0.1.x 用户写错字段的错。
- `ConnectionsSection.svelte`：存了 DeepSeek 的新 Key → 测一次 → `switchToDeepSeek`。
- `lib/agentChat/ReadyChecklist.svelte`（聊天页 `AssistantChat.svelte` 顶上）。
- 预览：`?mockLegacyUpgrade=optedIn`（已经启用她）、`?mockLegacyReminder=qwen | deepseek`（旧提醒助手用千问 / DeepSeek）。
- 测试：`apps/desktop/e2e/lol-happy-path.spec.ts`——新用户；0.1.x 用户旧提醒助手用千问，经升级引导、经「API 连接」（已启用她 / 选过继续用语音输入）、经清单的「填 DeepSeek Key」；0.1.x 就配过 DeepSeek；她被换回千问后清单的「换成 DeepSeek」；管理员和插件两行的按钮；Key 没通过测试时不换。
- 待真机：管理员那一行的重启、真 Key 下「API 连接」的测试，和一局完整的英雄联盟（见 [playtest-0.2.0.md](../playtest-0.2.0.md)“英雄联盟玩家：从安装到打完一局”）。
