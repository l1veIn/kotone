# Agent 底座第二批实现与验证记录（2026-11-15）

范围：[演进计划 §5.5](../product-evolution-game-tools.md) 的第二批“只读面与管理面”，以**首次引导对齐**为验收标准——引导页与设置页里人能做的配置，Agent 都能通过工具完成。它是 DSH 插件 `dsh-game-communicate` 设想中“装好插件 → 一键装 Kotone → Agent 帮用户配置”的第 2 步（第 1 步是 `kotone.exe` 兼任命令行与 MCP 服务器，见 [CLI 参考](../cli.md)）。分支 `claude/game-tools-m2`。现行用法见[工具服务手册 §5“配置与首次引导”](../tool-service.md#5-已实现工具)。

## 1. 设计

| 决定 | 内容 |
| --- | --- |
| 一个委托入口 | 不为 35 个工具各加一个宿主方法：`AppHost` 新增 `supports_invoke()` 与 `invoke(tool, args)`。`kotone-tools` 只负责目录与 schema（`catalog_host.rs` 的 `HOST_TOOLS`）、“参数是 JSON 对象”、执行名额、期限与关闭竞争；宿主按工具严格解析参数 |
| 与设置页同一实现 | 桌面端 `tool_config.rs` 直接调用设置页的 `#[tauri::command]` 函数（`update_settings`、`download_model`、`upsert_connection`、`set_models_dir` …），校验、原子落盘、热键重注册、运行时状态推送完全一致，不另写一套逻辑 |
| 错误分类 | 宿主返回 `HostError::{InvalidArguments, Busy, Failed}`，映射为 `INVALID_ARGUMENTS` / `BUSY` / `HOST_ACTION_FAILED`（设置命令的“配置项不合法”归为参数错误） |
| 就绪如实 | 没有桌面宿主的实例里这些工具 `ready=false` 并给出原因，调用返回 `HOST_ACTION_FAILED` |
| 长任务 | `models.download` / `speech.model.download` 受理即返回，进度（至多每秒一次）与结果经事件总线发布；预先检查唯一的下载名额（新增 `kotone_stt::download::is_active`），已有下载时直接 `BUSY` |
| 提权重启 | `elevation.restart` 先返回 `restarting`，再弹 UAC（等待用户不受 30 秒调用上限约束）；同意后与设置页按钮相同立即退出，拒绝发布 `elevation.restart.failed` |
| 设置窗口同步 | 工具改动设置后发 `kotone://settings-changed`，设置窗口重新读取，不显示过期值；历史变更沿用 `kotone://history-updated` |
| 凭据隔离 | 设置了 `KOTONE_HOME` 时系统凭据库服务名为 `kotone@<目录>`：隔离实例（测试、Agent 沙箱）读不到也写不进用户主实例的 API key；未设置时仍为 `kotone`，已保存的 key 不受影响 |
| 不收任意路径 | 只有 `models.dir.set`（配置 Kotone 自己的模型目录，与设置页相同）接受目录；`diagnostics.export` 固定写到 `<KOTONE_HOME>/exports/` 并返回路径 |

工具清单与参数见手册。与演进计划表中暂定命名的差异：`models.setActive` → `models.activate`，`system.elevation.*` → `elevation.*`，`system.autostart.*` → `autostart.*`，`system.resources` → `app.resources`，`text.processors/process` → `postprocess.processors/test`；选择麦克风、引擎、profile 统一走 `settings.update`。

## 2. 验证

**自动化**（`TMP/TEMP` 指向 `target/tmp`）：`cargo test --workspace` 全部通过（`kotone-tools` 88 项，新增：委托工具参数缺省为空对象并原样交给宿主、非对象参数在服务端拒绝、三类宿主错误映射、与期限竞争；无宿主时 `settings.get` / `models.download` 明确失败；capabilities 中配置类工具在无宿主实例里如实 `ready=false`；目录与 `HOST_TOOLS` 一一对应；凭据服务名按 `KOTONE_HOME` 区分）；`cargo clippy --workspace --all-targets -D warnings`、改动文件 `rustfmt --check`、`pnpm check`（0 错误）通过。

**真机冒烟**（调试构建的 `kotone.exe` 经管道调用，隔离 `KOTONE_HOME` 与 `WEBVIEW2_USER_DATA_FOLDER`）：35 项全部通过——

| 场景 | 结果 |
| --- | --- |
| capabilities | 59 个工具；配置类全部 `ready=true`（只有未安装模型的 `ocr.recognize` 未就绪） |
| 设置 | `settings.get`；`settings.update` 设 `assistant.route=agent` 后 `app.status` 立即显示 `agent`，并已写入隔离目录的 `config.json`；非法取值、经 `settings.update` 改连接、未知参数均为 `INVALID_ARGUMENTS`；`ui.firstRunCompleted` 可写 |
| 设备与模型 | `audio.devices`（9 个设备）、`stt.engines`（6 个引擎）、`models.list`（6 个模型，目录在隔离目录内）；未知模型 `INVALID_ARGUMENTS` |
| 下载 | `models.download silero-vad`（约 2 MB，选它是为了不下载大模型）立即 `started`，紧接着的第二次调用 `BUSY`；收到 `model.download.progress` 与 `model.download.finished`（2.4 s），文件落在隔离模型目录 |
| 连接 | `connections.presets`（7 个）；`connections.upsert` 带测试 key 后 `hasApiKey=true`，**key 不出现在任何工具输出与 `config.json` 中**；`connections.delete` 删除连接与 key，事后确认凭据库中没有残留（只查询该测试项名称） |
| 其他 | `profiles.list`、`hotkeys.conflicts/status`、`input.check`、`elevation.status`、`autostart.status`、`history.list/clear`（隔离目录内为空）、`diagnostics.export`（ZIP 写到 `<KOTONE_HOME>/exports/`，非法 `window` 为 `INVALID_ARGUMENTS`）、`app.resources`、`postprocess.processors/test`、`speech.status` |
| MCP | `kotone.exe mcp` 列出 63 个工具（6 个静态 + 57 个映射），`inputSchema` 全部为 object，新工具均可调用；退出后无残留进程 |

## 3. 未验证 / 未做

- **会改变本机系统状态的工具没有在真机上执行**：`autostart.set`（写真实的 HKCU Run 项）、`elevation.restart`（弹 UAC 并重启）、`models.dir.set`（迁移目录；单元路径与设置页共用）。只验证了它们的只读对应工具。
- **大模型下载**：只下载了约 2 MB 的 silero VAD；几百 MB 的识别模型与 Kokoro 语音包（约 147 MB）的下载、取消与续传未在本批真机验证（下载实现与设置页共用）。
- **设置窗口自动刷新**：后端事件与前端监听已实现、`pnpm check` 通过，但未在打开着的设置窗口里目视确认。
- 本机环境的两个观察（与本批改动无关）：`input.check` 报告低级钩子只收到 0/2 个探测事件；`speech.status` 报告系统没有可用的 Windows 默认语音（`0x80045040`）。
- 演进计划第二批剩余：`settings.warnings`、`connections.test`、profile 导入导出 / `profile.detect` / 图标、热词与屏蔽词导入导出、`history.get/audio`、`eval.*`、`hotkey.capture`、`ocr.models.install`、`app.restart`。
