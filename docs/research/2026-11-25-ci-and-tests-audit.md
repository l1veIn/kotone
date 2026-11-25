# 2026-11-25 测试和 CI 评估：慢在哪、哪些多余

玩家（2026-11-25）：“是否我们项目存在过渡（过度）CI检测或者过渡测试的情况？需要分析一下”。看过分析以后：“可以，我同意，尽量减少我们CI的时间，实在是太长了。”

**怎么查的**：`gh run view` 拿 CI 每一步的用时和日志、`git log` 统计、代码检索，本机跑了一次 Playwright 对照（只开无头浏览器，不发系统输入；跑之前查过没有游戏进程，低优先级）。本机没跑 cargo：当时另一个子代理在跑 `cargo +1.100.0 test --workspace`，再起一个会抢内存（Windows 1455“页面文件太小”就是这么来的）。真机脚本（第六节）是逐个读脚本和它测的代码核对的，**一个都没跑**（玩家在用电脑）。

## 一、结论

1. **测试不算多，慢在“怎么跑”。** Rust 的 1654 个测试在 CI 上跑完只要约 30 秒（11-23 那次：24 个测试程序加起来 29 秒，doc-test 3.5 秒）。慢的是三件事：
   - **编译和链接是串行的**：`.cargo/config.toml` 里 `jobs = 1`，CI 也照这个来，4 核只用了 1 核。这行是 10-04 为本机的 Windows 1455 加的；加之前 CI 用 `jobs = 2` 跑了两个月，没出过 1455。
   - **Playwright 只开 1 个 worker**（`workers: 1, fullyParallel: false`，0.1.0 起就这样）。206 个测试平均每个 2.9 秒，大部分花在 `vite dev` 下把页面加载起来，一共 10.4 分钟。
   - **所有事情在一个 job 里排着做**：前端测试根本不用 Rust，也要等 clippy 跑完才开始。
2. **CI 里有几处检查做多了，都不是测试本身**：
   - 依赖审计每次推送都跑。47 次失败里 9 次是审计挂的，都是新发布的安全公告，和那次提交无关。
   - 每次推送都打一个 debug 安装包。发 RC 时 `release.yml` 本来就会打。
   - `release.yml` 把 main 上 CI 刚跑过的 Playwright 和 cargo test 再跑一遍。
   - spec 里 128 处 `page.screenshot()` 只是存图、不是断言，CI 也没上传这些图。
3. **急事：缓存冷的时候 CI 会超过 45 分钟的时限。** 11-25 的 45612535648 跑到 41 分钟时 cargo test 挂了。要是测试过了，后面还有 debug 安装包（冷约 3–4 分钟）和保存缓存（3–4.5 分钟），一共约 48–50 分钟，会被掐掉。`CACHE_ON_FAILURE` 是 false，超时就存不下缓存，下一次还是冷的，一直超时下去。写分析时新缓存（key `…-1163f1ea`）还没存成，CI 45622890365 正在跑，没等到结果。
4. **11-25 挂两次，根子是工具链没钉死。** `rust-toolchain.toml` 写的是 `channel = "stable"`：CI 上是 1.100.0，本机默认是 1.98.0。新 clippy 的弃用警告、新 cargo 把测试程序挪到 `target/debug/build/<包>/<hash>/out/` 下，本机都看不到。照 Zed 钉成 `"1.100.0"`，“推送前记得 `+1.100.0`”这条规矩就能删掉（原则 2）。

## 二、CI 慢在哪

### 每一步的用时（秒）

| 运行 | 缓存 | clippy | Playwright | cargo test | debug 安装包 | 保存缓存 | 总计 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 09-25 36133166012（`jobs=2`，冷） | 没命中 | 277 | 36 | 313 | 132 | 91 | 21 分 |
| 10-04 37183363821（`jobs=1`，冷） | 没命中 | 432 | 84 | 550 | 203 | 120 | 30 分 |
| 11-18 44286058752（冷） | 没命中 | 442 | 153 | 627 | 146 | 86 | 30 分 |
| 11-18 44345778316（热） | 完全命中 | 75 | 169 | 232 | 115 | 0 | 12 分 |
| 11-23 45117439775（热） | 完全命中 | 95 | 297（136 个） | 278 | 135 | 0 | 16 分 |
| 11-23 45224416505 | 部分命中 | 158 | 305（145 个） | 352 | 145 | 186 | 22 分 |
| **11-25 45612535648（几乎全冷）** | 部分命中，重编了 471 个依赖 | **655** | **631（206 个）** | **1010（挂了）** | 没跑到 | 没跑到 | 41 分 |

- 09-25 到 10-04 代码量差不多。`jobs` 从 2 改成 1 以后，冷的 clippy 277 → 432 秒（1.56 倍），cargo test 313 → 550 秒（1.76 倍）。
- 11-25 虽然“部分命中”，clippy 还是编了 471 个包、cargo test 编了 415 个：新加的依赖（rmcp 等）改了几个公共依赖统一后的 feature，下游全部重编。以后加依赖还会碰到，只能让冷编译本身快一些。

### 一次推送编了几遍

- **clippy**：依赖和工作区 crate 按 check 各过一遍，产物是 rmeta，`cargo test` 用不上。
- **cargo test**：完整编译一遍，再**链接约 44 个 exe**（按 cargo metadata：lib 测试 21 个、`tests/*.rs` 13 个、examples 6 个，加上 kotone-tauri、kotone-cli 的 `main.rs` 各一个测试程序——里面 0 个测试，却各把整个应用链接一次）。热的时候这一步 5 分 17 秒，最后约 3.5 分钟全在挨个链接（日志里每个 `(lib test)` 隔 5–40 秒）。
- **tauri build --debug**：依赖和测试共用，但 feature 不同（custom-protocol），11 个工作区 crate 再编一遍，再链接 kotone.exe、打 NSIS 包。
- 所以工作区 crate 每次至少编 3 遍。rust-cache 照设计**不缓存工作区 crate**（README：“the workspace crates themselves are not cached”），这 3 遍每次都免不了。
- `windows` crate 有 4 个版本（0.52 / 0.54 / 0.61 / 0.62），都是间接依赖带进来的；冷的时候光它们就占 clippy 80 秒、test 113 秒。

### rust-cache

- key 由 rustc 版本、`Cargo.lock` 和全部 `Cargo.toml` 的哈希组成。改任何一个 `Cargo.toml` 就是新 key，跑完要花 **3–4.5 分钟上传约 2.1 GB**。这几天加插件常改 `Cargo.toml`，这一项常出现。
- 没设 `save-if`。Tauri 只在 dev 分支存（`save-if: github.ref == 'refs/heads/dev'`），免得 PR 的缓存把主分支的挤掉。仓库缓存上限 10 GB，`release.yml` 还有一份约 2 GB。
- runner 镜像的 stable 一升级（约 6 周一次），所有缓存一起失效，接着一次冷编译——按原来的样子就是超时。钉死工具链以后，升级变成我们自己挑时间的一次提交。

### Playwright

- 206 个测试、1 个 worker、10.4 分钟：2–3 秒的 118 个，3–5 秒的 58 个，超过 5 秒的 12 个（第一个 `about.spec` 10.8 秒，是 vite 第一次转换模块）。每个测试都是新的 BrowserContext 加 `page.goto`，`vite dev` 下约 250 个模块重新拉一遍。
- spec 之间没有共享状态：没有固定端口、不写文件（截图除外）、没有 `serial`。浏览器预览的假数据放在各自 context 的 localStorage 里，并行是安全的。
- **本机实测**（无头、低优先级、16 线程）：60 个测试 1 个 worker 92 秒、4 个 worker 36 秒（快 2.6 倍），同样 60 个在 CI 上 164 秒；124 个桌面端测试 4 个 worker 加 fully-parallel 67 秒全过，CI 上 1 个 worker 要 356 秒。写固定截图路径（`target/plugin-shots/`）的 14 个 spec 没跑，免得覆盖别人的审查截图。CI 只有 4 个 vCPU，保守算 10.4 分钟能降到 **4–5 分钟**。
- 配置用的是 `channel: "chrome"`（系统装的 Chrome），CI 却每次 `playwright install chromium`（约 15 秒），装了没用上。

### 能并行的、能跳过的

- 前端（`pnpm check`、`vite build`、Playwright）完全不需要 Rust，可以单独一个 job。Rust lint（fmt、clippy）和 Rust 测试也能分开（Zed 的 `clippy_windows` 和测试 job 是分开的），代价是各一份缓存。仓库公开，`windows-latest` 是 4 vCPU / 16 GB，**分 job 不花钱**。
- 最近 39 次 main 推送：只改文档 4 次，只改 Rust 13 次，只改前端 2 次，两边都改 20 次。按路径跳过**省得不多**：只改文档的那 10% 可以整个不跑；分 job 以后只改 Rust 的推送跳过前端 job，省机器时间，不省等待。Tauri 在 PR 上用 `paths` 过滤，Zed 用一个 orchestrate job 算出改了哪些包；我们这么小，工作流级别的 `paths-ignore: [docs/**, '**/*.md', website/**]` 就够了。

## 三、测试是不是过度

### 数量和用时

| crate | `#[test]` 数（静态数，含 ignore） | CI 上运行用时 | 测试代码占比（约） |
| --- | --- | --- | --- |
| kotone-core（lib + orchestrator） | 498 | 1.5 + 4.6 秒 | 45% |
| kotone-tauri（src-tauri） | 171 | 0.04 秒 | 20% |
| plugins/lol | 124 | 约 2 秒 | 41% |
| kotone-platform-windows | 114 | 1.3 秒 | 24% |
| kotone-tools | 113 | 3.0 秒 | 38% |
| kotone-stt | 110 | 2.0 秒 | 31% |
| kotone-agent | 107 | 2.7 秒 | 39% |
| kotone-ai | 93 | 3.5 秒 | 45% |
| kotone-tts | 63 | 0.9 秒 | 32% |
| kotone-cli | 44 | 1.1 + 1.5 + 0.5 + 0.5 秒 | 47% |
| 其余 11 个 | 各 3–36 个 | 都在 5 秒以内（最慢 kotone-exec 4.7 秒） | |
| **合计** | **约 1688（实际跑 1654）** | **约 30 秒** | **Rust 共 18 万行，测试约 6.2 万行（35%）** |

- 最慢的测试程序 4.7 秒，没有单个测试拖时间的情况。没装 nightly，`--report-time` 用不了，没有逐个测试的用时；每个程序都不到 5 秒，逐个排已经没意义。
- 测试代码占 35%，对 Rust 项目正常。测试本身不贵，贵的是“每多一个测试程序就多链接一个带 sherpa-onnx、tauri 的大 exe”。
- Playwright 有 8,004 行 spec；前端代码约 3.9 万行，另有约 3,600 行假后端（`apps/desktop/src/lib/ipc/*Mock.ts`）。

### Playwright 和 Rust 测重了吗

- **测的不是同一层。** Playwright 跑浏览器预览，后端换成 TS 写的假后端，测“界面把后端给的东西显示对、把命令发对”；Rust 测规则本身。
- **真正重复的是假后端。** 有些假后端照抄了后端的规则，例如 `hotkeysMock.ts` 的注释写着“同后端 `parse_hotkey` / `save_custom_hotkey` / `new_conflict_error`”，冲突检查在 TS 里又写了一遍。结果：后端改一条规则，假后端也得改（ADR 040 那几次提交，假后端一共 +561 行）；Playwright 测的其实是 TS 抄的那份，抄错了测试照样过、真机是坏的（原则 22）。
- 成熟做法是 Tauri 官方 `@tauri-apps/api/mocks` 的 `mockIPC`：每个测试自己给固定的返回值，假后端不实现规则。见第五节建议 11。

### 低价值的部分

- 128 处 `page.screenshot()`：35 处写到固定路径 `target/plugin-shots/round7/…`，其余写到 `testInfo.outputPath`。都是给人或 Agent 审查用的，不是断言，CI 也不上传。每处 0.1–0.3 秒，加起来 20–40 秒。
- `waitForTimeout` 只有 3 处（`bubble.spec`，300–400 毫秒），不是问题。
- 一个 spec 只有 1 个测试的有十来个（hotkey-combo、hotkey-focus、overlay-visibility、autostart……），每个约 2 秒，主要是页面加载。并行以后合并意义不大，**不专门去合**。

### 是不是在测实现细节

- 11 月以来 428 个提交：改了界面代码的 125 个里，99 个（79%）同时改了 spec。spec 新增 8,393 行、删除 2,006 行；同期界面代码新增 32,444 行、删除 15,067 行。大部分是**新功能带新测试**，该有的。
- 能确认的“牵一发动全身”：ADR 040 第 2 期第 3 步（`92b4f154`，热键只在「设置 → 热键」改）一次改了 8 个 spec（+194 / −134），因为它们写的是“在 X 页面录热键”。这是用户路径型测试的正常代价，但说明**每个功能的录键流程只要在一个 spec 里完整走一遍**，别的 spec 只断言“显示当前键 + 去改”。`hotkeys-page.spec` 的 “other pages show the current key and 去改 opens its row” 已经这么写了。
- 热键相关的 6 个 spec（hotkeys-page、hotkeys-custom、assistant-hotkey、hotkey-combo、hotkey-focus、post-processing-hotkeys）共 17 个测试，按标题看测的是不同行为（录键归属、冲突标红、WebView 回退、自定义热键……），没看到明显重复。

### 不稳定的测试

- 最近 9 次 main 失败：2 次工具链和本机不一致（11-25 的 clippy 弃用警告、example 找不到）；3 次审计碰上新公告；1 次缓存把 sherpa 的库删空了（已修）；1 次测试不稳定（`shutdown_lets_http_long_polls_deliver_stopping`，`160fd7fe` 改成“等长轮询真的进入等待”以后没再挂）；1 次 kotone-exec 新 crate 的编码测试在英文 Windows 上挂了（CI 发现的真问题）。
- 同类隐患（按套路查的，不一定挂过）：
  - **“bind 端口 0 再 drop，当作没人听的端口”有 5 处**：`crates/kotone-mcp/src/error.rs:204`、`crates/kotone-mcp/tests/http.rs:164`、`crates/kotone-remote-plugin/src/tests.rs:324` 和 `:375`（就是 11-25 挂的 `an_offline_server_says_it_is_not_connected`）、`crates/kotone-tools/src/tests_http.rs:857`。同一进程里并行的测试可能正好拿到这个端口。
  - **断言墙钟时间（`elapsed() <` / `>`）的有 77 处，分布在 20 个文件**，机器一忙就可能挂，例如 `an_offline_server…` 里的 `< 1s`。不一次全改，挂一个修一个（假时钟或放宽时限）。
- 不上 nextest 的重试来盖住不稳定（测试只跑 30 秒，加个工具只为重试不划算）。`--no-fail-fast` 该加：11-25 kotone-mcp 的 stdio 挂了以后，后面的测试程序都没跑，别的问题要等下一个 40 分钟才看得到。

## 四、不能为了快砍掉的

| 留着 | 依据 | 成本 |
| --- | --- | --- |
| `crates/kotone-plugin-api/tests/boundaries.rs`、`event_boundaries.rs`（插件只依赖契约、不碰全局钩子、不自己按键、事件名唯一） | 原则 16 | 毫秒级（扫文件） |
| `terminology.spec.ts`（文案在 messages 里、中英参数一致、禁用词） | 原则 16、17 | 4 个测试共 3 秒 |
| 配置迁移测试（`new_capabilities_never_write_values_old_versions_cannot_read`、热键表从旧字段迁移、`lol.enabled` / music 搬进插件……） | 原则 19，配置格式是单向门 | 毫秒级 |
| `cargo clippy -D warnings --all-targets` | 11-25 就是它拦下了弃用 API | 分 job、多开 jobs 以后 1–2 分钟 |
| cargo test 全量 | 一共只跑 30 秒，贵的是编译，砍测试几乎不省时间 | |
| Playwright 用户路径 | 唯一测到“界面接线”的地方；并行以后不在关键路径上 | |
| 真机脚本 | 原则 22；不在 CI 里，不影响 CI 时间 | |
| `release:verify` / `release:test` / `cargo fmt` | 加起来不到 10 秒，fmt 历史上挂过 5 次 | |

## 五、建议和玩家定了什么

“现在”指 206 个 Playwright 测试、缓存热的情况，单 job 约 **25–30 分钟**；冷缓存约 **48–50 分钟（超时）**。预计用时都是按上面的数据推的，改完以第一次 CI 的实测为准。

| # | 改什么 | 省多少 | 风险 | 定了什么 |
| --- | --- | --- | --- | --- |
| 1 | **CI 设 `CARGO_BUILD_JOBS`**：clippy 4（只 check、不链接、不吃内存），cargo test 和打包 2（10-04 之前 CI 一直是 2）。环境变量优先于 `.cargo/config.toml`（Cargo Book：“Environment variables will take precedence over TOML configuration files”），本机还是 1 | 冷：clippy 655 → 约 300 秒，test 1010 → 约 580 秒，**省约 13 分钟**，不再超时；热：2–3 分钟 | 低。test 用 3–4 没在 CI 上试过，要试再加 `al-cheb/configure-pagefile-action` 调大页面文件 | 玩家同意 |
| 2 | **`rust-toolchain.toml` 钉成 `channel = "1.100.0"`**（加 `components = ["rustfmt","clippy"]`），CI 按这个文件装。照 Zed | 每次 runner 升级 stable 少一次冷编译（约 6 周一次，每次 40 分钟以上）；本机和 CI 一致 | 低。升级工具链变成一次提交，顺带看新 clippy 的警告 | 玩家同意；删掉“推送前记得 `+1.100.0`”这条规矩 |
| 3 | **前端单独一个 job**（pnpm install、`pnpm check`、`pnpm build:web`、Playwright），Playwright 改成 `workers: 4` 加 `fullyParallel: true`（Playwright 文档示例是 `workers: process.env.CI ? 2 : undefined`；本机 4 个全过）；删掉没用的 `playwright install chromium` | Playwright 10.4 → 约 4–5 分钟，而且**不在 Rust 的关键路径上**；整体再省约 10 分钟 | 低。`bubble.spec` 的 3 处 `waitForTimeout` 机器忙时可能不稳，先看两三次 | 玩家同意 |
| 4 | **clippy 和 fmt 单独一个 job**，和 cargo test 并行（照 Zed）；加 `save-if: github.ref == 'refs/heads/main'`（照 Tauri） | 热：总时长从“clippy + test”变成两者里慢的那个，再省 1–3 分钟 | 低。多一份约 1 GB 的缓存 | 玩家同意 |
| 5 | **审计挪到单独的工作流**：改了 `Cargo.lock`、`pnpm-lock.yaml` 时跑，加每天一次定时（rustsec/audit-check 常见用法）；`--ignore` 挪进 `.cargo/audit.toml`，只留一处 | 时间只省约 10 秒；**主要是去掉和提交无关的红叉**（47 次失败里 9 次） | 低。新公告最多晚一天看到，发 RC 前看一眼 | 玩家同意 |
| 6 | **debug 安装包不再每次打**：只在改了 `apps/desktop/src-tauri/tauri*.json`、`installer/**`、`src-tauri/Cargo.toml`、`Cargo.lock`、工作流文件时打 | 热 2–3.5 分钟，冷约 4 分钟 | 中低。NSIS 配置坏了要到发 RC 才发现；发 RC 很频繁，影响小 | 不用玩家定 |
| 7 | **`cargo test --workspace --locked --no-fail-fast`** | 不省时间，省来回：一次看到所有失败（照 Zed） | 无 | 不用玩家定 |
| 8 | **工作流加 `paths-ignore`**：`docs/**`、`**/*.md`、`website/**` | 只改文档的推送（约 10%）不跑 | 低。以后开“必须通过 CI”的保护要另外处理，现在没开 | 不用玩家定 |
| 9 | **少链接几个测试程序**（matklad，“Delete Cargo Integration Tests”：Cargo 自己合并后编译测试快 3 倍）：kotone-tauri 和 kotone-cli 的 `[[bin]]` 设 `test = false`；kotone-cli 的 3 个 `tests/*.rs` 合成 1 个；kotone-stt 的 2 个合成 1 个 | 热 cargo test 少链接 4–5 个大 exe，约 40–90 秒 | 低 | 不用玩家定 |
| 10 | **CI 里不截图**：截图包成一个 helper，`process.env.CI` 时跳过；固定路径 `target/plugin-shots/round7/…` 改成 `testInfo.outputPath` | 20–40 秒 | 无 | 不用玩家定 |
| 11 | **假后端不再照抄后端规则**：照 Tauri 的 `mockIPC`，每个 spec 自己给返回值；规则只在 Rust 里测 | 不省 CI 时间；改后端规则时少改一份 TS；能删掉约 3,600 行假后端里“同后端 xxx”的那部分 | 中。一个 spec 一个 spec 地改，浏览器预览（`dev:web`）也靠这些假数据 | **玩家同意，但排到以后单独做**：不省 CI 时间，是一次前端重构 |
| 12 | `release.yml` 先查这个提交的 CI 过没过，过了就跳过 Playwright 和 cargo test；CI 被跳过或取消时退回全量 | 每次打 RC 省 15–20 分钟（rc.4 用了 60 分钟） | 中 | **玩家同意**（“发 RC 时 CI 已过就不重跑测试”） |
| 13 | （可选，要实测）Windows Dev Drive（`samypr100/setup-dev-drive`，作者说 IO 重的构建平均快约 25%，没有 Rust 的数字）、sccache（Zed 在用，能缓存工作区 crate，链接缓存不了） | 不确定，大概 10%–25% | 中 | 先不做；1–8 做完看实测再说 |

**做完 1–8 的预计**：热缓存总时长看最慢的 Rust 测试 job（准备 2.5 分钟、编译加链接约 3.5 分钟、跑 0.5 分钟），**25–30 分钟降到约 7–9 分钟**；前端 job 约 6 分钟，lint job 约 4 分钟。冷缓存 **48–50 分钟（超时）降到约 15–18 分钟**。

### 推送前本地先跑一遍

11-25 两次失败，用和 CI 一样的工具链跑 clippy 和 cargo test 都能挡住；历史上 fmt 失败 5 次、clippy 失败 4 次也都能在本机挡住。审计和缓存的问题本机挡不住，也不用挡（建议 5 已经挪开了审计）。

最少要跑：`cargo fmt --all -- --check`（几秒）、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked --no-fail-fast`（本机热的时候几分钟）、`pnpm check`（约 30 秒）；改了 `apps/desktop/`、`plugins/*/{app,ui,e2e}`、`messages/` 再跑 Playwright（本机 `--workers=4 --fully-parallel` 全量约 2 分钟）；改了版本号跑 `pnpm release:verify && pnpm release:test`（1 秒）。

写成一个命令（`scripts/prepush.mjs`，用法见 [开发约定](../development.md)；原则 2：把“改了什么要跑什么、用哪个工具链、有游戏时怎么跑”收进一处，只记一条“推送前跑它”）：开头查游戏进程，有游戏就低优先级加 `-j 2`；`TMP` / `TEMP` 设到 `target/tmp`；用 `git diff --name-only origin/main...HEAD` 决定跑不跑 Playwright；前面失败也接着跑后面的，最后一起报告。**不做成 git pre-push hook**：全量要好几分钟，推送的又是 Agent，写进开发约定更直接。

## 六、真机脚本（`scripts/manual-e2e/`）

都**不在 CI 里跑**，不影响 CI 时间；它们是原则 22 的落实，代价只是维护。分析时 56 个文件、约 1.6 万行：46 个测试脚本，8 个库或工具（`kotone.py`、`mock_deepseek.py`、`real_deepseek.py`、`mock_interpreter.py`、`fg.ps1`、`uia.ps1`、`target_window.ps1`、`fixtures/`）。脚本里调的工具名、46 个 Tauri 命令、大部分设置键和命令行参数都还在；过时的是几个界面 testid、一条日志正则、一个首屏选择器、默认呼叫键。10 个脚本被别的脚本 import，不能直接删（`voice_turn_real` 被 13 个 import，`lol_tools_real` 4 个，`background_task_real` 3 个……）。

这次照原则定：**测的功能还在、只是脚本跟不上的就修**（只改到能对上现在的代码）；**功能删了、或者别的测试已经测了同一件事的就删**（原则 2）。每条先读脚本和它测的代码、用 `git log` 看功能什么时候改的。**改过的脚本都只做了语法检查（`python -X utf8 -I -m py_compile`，内嵌的 JS 用 `node --check`），一个都没跑**（玩家在用电脑），下次机器空着时要跑一遍（原则 24）。清理后 52 个文件、约 1.5 万行（删了 4 个文件、702 行）。修完以后把所有脚本里的 `getByTestId` / `[data-testid=…]` 和前端源码对了一遍，都还在（`plugins-card-${id}`、`settings-nav-${id}`、`${testid}-input` 这类拼出来的也核对了）。

### 删了（4 个文件）

| 文件 | 为什么删 | 现在谁测这件事 |
| --- | --- | --- |
| `happy_path_e2e.py` + `mock_deepseek.py`（只有它用） | 和 `happy_path_real.py` 是同一个流程，一个用假 DeepSeek、一个用真的；真的那份证据更强（原则 22）。它查的“系统提示里列了技能、给了 `skill.view`”是单元测试的事。它用的 `settings-nav-general`（`50348033` 删）、`home-call`（`254b2748` 删）也都不在了 | `happy_path_real.py`（修了，见下） |
| `mcp_names_real.py` | `kotone-cli` 的集成测试 `mcp_maps_underscored_plugin_tool_names_back`（分析时在 `crates/kotone-cli/tests/agent_cli.rs:389`）起的就是真的 `kotone-cli mcp` 和真的工具服务，只有宿主是假的，下划线名字还原、不先列也能调、撞名、没有的名字都测了；`kotone.exe mcp` 连真实例这一段由 `mcp_app_resources_real.py` 测（rc.9 22/22） | 上面两个 |
| `speech_preview_real.py` | 一次性的复现（玩家报“试听语音没声音”），只打印、没有检查；结论是“没复现”（2026-11-18，3/3 次回环峰值约 0.66，默认设置和玩家自己的设置都有声音）。之后空的合成算错误、每次播放都记设备和长度（`ffb8c469`） | 真 DeepSeek 语音出声：`voice_turn_real.py`、`call_mode_real.py` |

### 修了（5 个）

| 文件 | 哪里跟不上 | 改了什么 |
| --- | --- | --- |
| `copilot_flow_real.py` | 第 1 部分点 `settings-nav-general` 找语音输入卡片——语音输入 `50348033` 起是插件的 App，左侧栏不再单列；**分析没看到的两处**：默认呼叫键 `2fd7c2eb`（11-21）从 `Backquote` 改成 `Ctrl+Backquote`，第 1 部分还在查界面显示“~”（现在是“Ctrl + ~”），第 3 部分还在查 `companionBindings.hotkey == "Backquote"` | 改走「插件 → 语音输入」（`settings-nav-plugins` → `plugins-card-voice-input-open`，和 Playwright 的 `openVoiceInput` 一样）；呼叫键查“Ctrl + ~”和 `Ctrl+Backquote`（和 `copilot-flow.spec.ts` 一致） |
| `roles_real.py` | 正则 `wake: listening \(phrase=`，`wake.rs` 从 `ce4e66ca`（ADR 033，11-22）起打的是 `wake: listening (wake=…, phrase=…`，永远匹配不上；**分析没看到的一处**：ADR 033 起喊醒就是接通，通话缺省开 30 秒，通话里开口就是下一句、不认唤醒词（`wake.rs` 里 `call::active()` 时不判唤醒词），脚本只等 7 秒就喊下一个角色的唤醒词 | 正则加上 `wake=true, `；隔离配置加 `callIdleSeconds: 0`（不通话，一问一答，和写脚本时的行为一样） |
| `measure_upgrade_startup.py` | 等的 `legacy-upgrade-notice`（`254b2748` 删）和 `#hotkey-input`（`92b4f154` 删）都不在了。ADR 027 把冷启动、常驻内存列为**必须测**，0.2.0 还没放量，还要再测，所以修 | 0.2 的首屏改认引导里的升级说明（`onboarding-upgrade`），点 `onboarding-keep-voice`（现在的字是“继续使用语音输入法”），等语音输入插件的页面（`voice-input`）；0.1.13 照旧认 `#hotkey-input` |
| `upgrade_transition_native.py` | 分析说“`hotkeys_real` 加 `voice_input_real` 覆盖了”——**核对下来没有**：那两个用的都是新用户配置（`legacyUpgrade: notNeeded`），老用户模式（`dismissed`）下“按住热键 → 识别 → 打进前台”这条链只有它测，README“跨版本过渡”的升级验收（ADR 027）里有它这一步。它自己跑不起来：只认 PE 版本号 `0.2.0`，现在的构建是 `0.2.0-rc.8`（本机 `target/debug/kotone.exe` 读出来的）；用的 `pwsh` 这台机器上没装 | 接受 0.2.x 和候选版（`0.2.0-rc.N`）；改用系统自带的 `powershell`（测试窗口加 `-ExecutionPolicy Bypass`） |
| `happy_path_real.py` | 分析建议删；**核对下来功能还在**：《Kotone 配置》技能还在内置（`crates/kotone-agent/skills/kotone-config/`，11-21 还改过），而且它是真机里唯一真调 DeepSeek 文字识别的脚本、唯一查“真模型会不会照着技能做”的脚本（ADR 022 当时没验证的那条）。跟不上的是界面：`settings-nav-assistant` + `assistant-nav-chat`、`settings-nav-general` + `home-call` 都不在了；从 `happy_path_e2e.py` import 的几个函数随它删了 | 聊天直接用 `assistant-request`（应用开在聊天页，ADR 028）；呼叫点聊天页的按钮（照 `voice_turn_real.py`：`chat-call` 或 `call-kotone`）；几个函数抄进来 |

### 没合并：`input_gate_e2e.py` 和 `send_to_game_e2e.py`

分析建议把前者并进后者（同一个测试窗口、同一份游戏配置，153 行一样）。**没并**：两个测的是不同的决定——前者是 ADR 032 决定 1（Kotone 发送期间吞掉玩家的按键，经 `companion.type` 走 `kotone.exe tools call`，同时用不带标记的 `SendInput` 当玩家连按），后者是 ADR 041 第 1 期（等修饰键松开、两条同时发不交错、多行、频道，经工具服务的 HTTP 直接调 `game_chat.send`）。合并等于把两边的主流程都重写一遍，改完又没法跑（要玩家让出前台）；两个 11-25 都刚跑过、都过（`send_to_game_e2e` 5/5、`input_gate_e2e` PASS，见 [11-23 交接](2026-11-23-session-handoff.md)）。一样的那 153 行是按键和测试窗口日志的工具函数，以后要动时抽成一个 helper 模块就行（和 `hotkeys_real` / `voice_input_real` 那 288 行一样）。

### 也没做的（分析里列为可选）

- `hotkeys_real` 和 `voice_input_real` 有 288 行一样的准备代码，抽成 helper、两个测试都留。
- `mcp_remote_app_real` 并进 `mcp_app_real`，`background_focus_real` 并进 `focus_steal_real`。

### 怀疑重复、其实不重复

两个同传脚本（`interpreter_e2e` 测后台任务那条路、`interpreter_asr_real` 测用本体的识别）；三个英雄联盟脚本（`lol_probe` 录的数据是 `lol_tools_real` 的默认输入）；四个 `mcp_app*`（宿主不同、方向不同）；`voice_chat_real` 和 `voice_input_real`。评估用的工具留着：`lol_probe`、`sensevoice_lang_samples`、`wake_eval`、`cache_real`、`search_real`、`recap_real`。

### 文档对不上的

- README 少了 6 个脚本：`happy_path_e2e` 删了；补了 `browser_real`、`browser_probe_real`、`browser_tabs_real`、`call_mode_real`、`lol_client_real`。README 的“跨版本过渡”一节跟着改：按钮的字“继续使用语音输入法”，`upgrade_transition_native.py` 测的是老用户模式、接受候选版。
- 分析说“文档提到 3 个从没写过的脚本”，核对下来不全对：
  - `sessions_e2e.py`（[11-18 交接](2026-11-18-next-session-handoff.md)）**写过**：`89eae2da`（11-18）加、`99d091e0`（11-20，同传改成后台任务）删，换成了 `interpreter_e2e.py`。交接里补了一句。
  - `wake_e2e.py`（同一份交接）在 `target/agent-smoke/` 里，是没进仓库的临时脚本；后来写成了 `wake_eval.py`（ADR 025）。交接里补了一句。
  - `key_programs_real.py`（[插件重构](../plugin-refactor.md)第 6 步“下一步”第 1 条）是还没做的计划，写的是“写 …”，不是说它已经有了。没动。
- 引用了删掉的脚本的地方都补了去向：[ADR 022](../adr/022-sessions-memory-soul.md)（`happy_path_e2e.py`）、[11-18 交接](2026-11-18-next-session-handoff.md)（`happy_path_e2e.py` 两处）、[11-23 交接](2026-11-23-session-handoff.md)和 [MCP 客户端方案](2026-11-24-mcp-client.md)（`mcp_names_real.py`）。这些是有日期的记录，原文不改，只在后面括号里写“删了，改看 …”。

## 出处

- Swatinem/rust-cache README（不缓存工作区 crate、key 的组成、`save-if`）：https://github.com/Swatinem/rust-cache
- Tauri `test-core.yml`（PR 按 `paths` 过滤、只在 dev 分支存缓存、工具链钉 `'1.90'`）：https://github.com/tauri-apps/tauri/blob/dev/.github/workflows/test-core.yml
- Zed `run_tests.yml`（orchestrate 按改动的包跑、clippy 单独 job、`cargo nextest run --workspace --no-fail-fast`、sccache）：https://github.com/zed-industries/zed/blob/main/.github/workflows/run_tests.yml
- Zed `rust-toolchain.toml`（`channel = "1.100.0"`）：https://github.com/zed-industries/zed/blob/main/rust-toolchain.toml
- matklad，“Delete Cargo Integration Tests”（Cargo 测试编译快 3 倍）：https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html
- Cargo Book 配置（环境变量优先于配置文件；`build.jobs` / `CARGO_BUILD_JOBS`）：https://doc.rust-lang.org/cargo/reference/config.html
- Playwright 并行（workers、fullyParallel、shard、每个 worker 自己的 BrowserContext）：https://playwright.dev/docs/test-parallel
- GitHub 托管 runner 规格（公开仓库 Windows 4 CPU / 16 GB）：https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- 审计改成按锁文件加每天定时（osohq/oso、OpenPoolProject/stratum 的 audit.yml 都这样）：https://github.com/rustsec/audit-check
- setup-dev-drive：https://github.com/samypr100/setup-dev-drive
