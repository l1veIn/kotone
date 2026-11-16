# 手动真机测试工具（自动化 / 内置 Agent）

2026-11-16 自动化各阶段真机测试用过的脚本，结果见 [自动化实现记录](../../docs/research/2026-11-16-companion-scripts.md)。只在隔离环境里用：不碰真实的 `~/.kotone`、凭据库、已安装的 Kotone 与 WebView2 用户数据目录。

| 文件 | 作用 |
| --- | --- |
| `target_window.ps1 -Dir <目录>` | WinForms 测试窗口“KotoneScriptTarget”：把按键、鼠标（按窗口比例的位置）、滚轮、焦点变化逐行写进 `<目录>/events.log`；左上角可变黑的面板、40 号字“找到对局”、橙色方块（中心蓝点）、“接受”标签。往 `<目录>/control.txt` 写 `dark` / `light` / `fg` / `blur <ms>` / `close` 控制它；启动完成写 `ready.txt` |
| `fg.ps1 [-Title]` | 把测试窗口切到前台（先敲一下 Alt 绕过前台锁定，再 `SetForegroundWindow`） |
| `uia.ps1` | 用 Windows UI 自动化操作 Kotone 设置窗口（WebView2 把无障碍树暴露给 UIA）：列出元素、点按钮、给输入框填值。经 `kotone.py ui` 调用，中文名用 base64 传，避免控制台编码问题 |
| `kotone.py` | 对隔离实例调用工具（`call` / `runs` / `wait`）和 UI 自动化（`ui`） |

## 准备

```sh
# 1. 嵌入前端的调试构建
cd apps/desktop && pnpm build:web && cd ../..
cargo build -p kotone-tauri --features tauri/custom-protocol
# 2. 复制到仓库外：仓库根目录带“低完整性”标签，从仓库里启动的 exe 发出的模拟输入会被系统静默丢弃
mkdir -p "$LOCALAPPDATA/Temp/kotone-automation-e2e" && cp target/debug/kotone.exe "$LOCALAPPDATA/Temp/kotone-automation-e2e/"
# 3. 隔离目录：开启工具服务、跳过首启向导
mkdir -p target/automation-e2e/home target/automation-e2e/webview target/automation-e2e/target
echo '{"toolService":{"enabled":true},"ui":{"firstRunCompleted":true}}' > target/automation-e2e/home/config.json
```

用后台方式启动（Claude Code 里用 `run_in_background`，不要用 `&`）：

```sh
KOTONE_HOME='<仓库>\target\automation-e2e\home' WEBVIEW2_USER_DATA_FOLDER='<仓库>\target\automation-e2e\webview' \
  "$LOCALAPPDATA/Temp/kotone-automation-e2e/kotone.exe"
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/manual-e2e/target_window.ps1 -Dir '<仓库>\target\automation-e2e\target'
```

## 例子

```sh
python -X utf8 -I scripts/manual-e2e/kotone.py ui invoke "暂不"          # 首次启动的“需要管理员权限”弹窗
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/manual-e2e/fg.ps1
python -X utf8 -I scripts/manual-e2e/kotone.py call automation.run '{"source":"click(0.5, 0.5)"}'
python -X utf8 -I scripts/manual-e2e/kotone.py runs
python -X utf8 -I scripts/manual-e2e/kotone.py ui invoke "自动化"
python -X utf8 -I scripts/manual-e2e/kotone.py ui names
python -X utf8 -I scripts/manual-e2e/kotone.py call app.quit
```

## 已知限制

- Kotone 的热键走低层键盘钩子，钩子按设计忽略模拟按键：**热键只能人工按**。
- 中文输入法会吞掉字母键的按下；测试窗口关闭了输入法。
- 隔离实例的设置窗口启动后会抢前台，后台启动的测试窗口拿不回前台，用 `fg.ps1`。
- UIA 按名字找元素：同名的文字和输入框都会匹配，`setvalue` 只挑输入框，`invoke` 只挑按钮；自定义下拉框（`Select` 组件）还没有封装。
