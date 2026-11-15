# 陪玩脚本：实现与验证记录（2026-11-16 起）

设计见[演进计划 §12](../product-evolution-game-tools.md#12-陪玩脚本大模型写脚本沙盒执行2026-11-16-定方向)。分支 `claude/companion-scripts`，从 `main`（`90113d8`）开出。

## 第一阶段：语言实验（`crates/kotone-script`）

**结论：采用 Luau**（`mlua` 0.10，`luau` + `vendored` 特性，Luau 0.663 随构建一起编译）。

- **能构建**：在本机 MSVC 工具链下直接编译通过，不新增运行时 DLL。
- **沙盒**：只加载 string / table / math / utf8 标准库，移除 `loadstring`、`require`、`getfenv` / `setfenv`、`collectgarbage`、`print`，再开启 Luau `sandbox` 模式（全局只读、脚本独立环境）。测试确认 `io.open`、`os.execute`、`loadstring`、`require`、`getfenv`、`dofile` 都不可用；`string.rep('x', 1e9)` 被 16 MB 内存上限拦下。脚本给内置函数重新赋值只会在它自己的环境里遮蔽，不影响宿主函数，每次运行也是新 VM。
- **限时与防死循环**：Luau 的中断回调在函数调用与循环回边处触发，用来检查取消、总时长与执行步数。`while true do end` 被步数上限终止；持续按键的脚本被总时长或按键次数上限终止。
- **数据不是指令**：`ocr()` 返回不透明对象，只提供 `:has(文字)` 与 `:count()`，`tostring` 与字段访问都拿不到原文，无法把屏幕上的字传给 `type_text` / `say`。
- **两个目标场景**（假宿主、虚拟时钟）：`every(1000, function() key("Q") end, { for_ms = 10000 })` 恰好按 10 次、间隔 1000 ms；“左上角变黑按回车”在变黑后连续 3 帧（每 200 ms）满足时只按一次回车。
- **其他护栏**：两次按键至少间隔 50 ms（过快时自动等待）；游戏不在前台时按键暂停、回到前台继续；取消立即生效；参数不合法、调用不存在的函数、语法错误都如实报告；`check_syntax` 用于大模型写完后先校验。
- **为什么不是 Rhai**：Rhai 纯 Rust、沙盒更省事，但大模型对它远不如对 Lua 熟悉；Luau 的沙盒与中断能力已经覆盖需要的护栏，因此不再试 Rhai。
- 验证：`cargo test -p kotone-script`（10 个用例），`cargo clippy -p kotone-script --all-targets` 无警告。
- **未验证**：真实按键注入与截图（第二阶段接入）；在真实大模型手里写出脚本的成功率。
