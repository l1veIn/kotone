# ADR 002：apps/ + crates/ 产品 monorepo 归位

- 状态：已采纳（随「归位重构」落地）
- 上下文：ADR-001 完成五 crate workspace 拆分后，前端文件（index.html、src/、
  vite/svelte/ts 配置、package.json）与 `src-tauri/` 仍留在仓库根目录——根目录同时
  是 JS 项目根和 Rust workspace 根，双重身份导致：工具链配置互相干扰（vite 需要
  ignore 根级 target/）、新人无法一眼分辨「应用」与「库」、后续增加第二个应用
  （如 CLI 打包、落地页）无位置可放。

## 决策

归位为标准 `apps/` + `crates/` 产品 monorepo：

- `apps/desktop/`：Tauri 桌面应用（canonical 形态）——前端（Svelte/vite 配置）、
  `src-tauri/`（kotone-tauri 壳）、应用级 package.json 全部迁入；
- `crates/`：Rust 库与 CLI（不动）；
- 仓库根：纯 workspace 根——Cargo.toml/Cargo.lock（Rust workspace）、
  rust-toolchain.toml（钉 stable）、仅含转发脚根的 package.json、
  pnpm-workspace.yaml（`packages: ["apps/*"]`）、单 pnpm-lock.yaml。

配套归拢：

- Cargo `[workspace.dependencies]` 收编共有依赖（内部 crate 路径、serde /
  serde_json / tokio / windows / tempfile），成员改 `xxx.workspace = true`，
  版本不变只归拢；windows 的 features 各成员按需追加（可叠加）。
- 根 package.json 只留 name/private/scripts：`dev`/`build`/`tauri` 等经
  `pnpm -C apps/desktop ...` 转发，新增 `build:rust`/`test:rust`。

## 被否决项

- **保持根目录混合**：零搬迁成本，但根目录双重身份是持续的认知与工具链负担
  （vite watch ignore、.editorconfig/格式化工具作用域、未来多应用冲突），随
  应用增多只会更糟；搬迁成本一次性且低（git mv 保历史）。
- **apps/desktop 独立 pnpm-lock**：多 lock 意味着依赖版本可能漂移、CI/安装命令
  分裂；pnpm workspace 单 lock 是社区标准做法，且本次依赖集合不变、迁移零成本。

## 后果

- 正向：根目录职责单一（workspace 根）；应用与库边界清晰；vite 不再需要
  ignore 任何 Rust 产物路径（target 在根、前端在 apps/desktop，watch 范围天然
  隔离——保留既有 ignore 配置作为防御）；`pnpm install/dev/build` 命令在根目录
  不变（README 无需改）。
- 路径修正：Cargo members 指向 `apps/desktop/src-tauri`；kotone-tauri 的内部
  crate path 依赖 `../crates/*` → `../../crates/*`；.gitignore 的 src-tauri 路径
  前缀同步；tauri.conf.json 的 `frontendDist: ../dist` 相对关系不变（仍指向
  应用级 dist）。
- 清理：删除拆分前遗留的 `src-tauri/target` 陈旧构建缓存约 15GB。
- 验证基线不变：cargo test --workspace 73/73、cargo build 0 警告、
  pnpm build:web / check / tauri dev 全部通过。
