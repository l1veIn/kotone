//! 把英雄联盟 App（`app/`，Vite + vite-plugin-singlefile 打成的单个 HTML）编进插件（ADR 039 D5）。
//!
//! - 打好了（`app/dist/index.html` 在）：编进去，`resource()` 给的就是它；
//! - 没打（只跑 `cargo test`、CI 的 Rust 检查）：编一个占位页，告诉看到它的人怎么打。所以 Rust 的构建和测试
//!   不依赖前端；发布构建由 tauri 的 `beforeBuildCommand` 先跑 `pnpm build:apps`。
//!
//! `app/dist/` 不进 git（`app/.gitignore`）。这里先把它建出来（空目录），好让 cargo 盯着它：打包以后里面的文件变了，
//! 下次 cargo 构建就会重跑这个脚本（盯一个不存在的路径会让 cargo 每次都重跑、每次都重编插件）。

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist = dir.join("app").join("dist");
    let _ = std::fs::create_dir_all(&dist);
    println!("cargo:rerun-if-changed=app/dist");
    println!("cargo:rerun-if-changed=build.rs");

    let bundle = dist.join("index.html");
    let code = if bundle.is_file() {
        format!(
            "/// `app/dist/index.html`（打好的 App）。\npub const BUNDLE: &str = include_str!({:?});\n/// App 打好了没有。\npub const BUILT: bool = true;\n",
            bundle.display().to_string()
        )
    } else {
        "/// App 还没打：占位页。\npub const BUNDLE: &str = PLACEHOLDER;\n/// App 打好了没有。\npub const BUILT: bool = false;\n"
            .to_string()
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("app_bundle.rs");
    std::fs::write(out, code).expect("写 app_bundle.rs");
}
