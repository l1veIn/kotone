//! 插件的边界写成测试（ADR 038 D6、“怎么定的” 2026-11-24；软件哲学原则 13、16）：只写在文档里的规则会被悄悄违反。
//!
//! - 插件依赖插件契约、`kotone-core`、自己的第三方库；可以用系统适配层 `kotone-platform-windows` 里没有状态的系统调用；
//! - 不碰 Tauri、桌面端、她（`kotone-agent`、`kotone-ai`）、本体的其他部分（工具服务、识别、语音、文字识别……），
//!   也不依赖别的插件（插件之间不直接调用）；
//! - 全进程只有一份的东西（Esc 和呼叫键的低级钩子、输入闸门）只经宿主，插件的源码里不出现。

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// 每个插件的（目录名，Cargo.toml）。
fn plugins() -> Vec<(String, toml::Table)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(repo().join("plugins")).unwrap() {
        let dir = entry.unwrap().path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).unwrap();
        let table: toml::Table = text.parse().unwrap();
        out.push((
            dir.file_name().unwrap().to_string_lossy().into_owned(),
            table,
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 一个 Cargo.toml 里所有依赖的名字（普通、按平台、开发用的都算；`package = ` 改名的按真名）。
fn dependencies(table: &toml::Table) -> Vec<String> {
    fn names(deps: Option<&toml::Value>, out: &mut Vec<String>) {
        let Some(toml::Value::Table(deps)) = deps else {
            return;
        };
        for (name, spec) in deps {
            let real = spec.get("package").and_then(|p| p.as_str()).unwrap_or(name);
            out.push(real.to_string());
        }
    }
    let mut out = Vec::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        names(table.get(key), &mut out);
    }
    if let Some(toml::Value::Table(targets)) = table.get("target") {
        for target in targets.values() {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                names(target.get(key), &mut out);
            }
        }
    }
    out
}

/// 本体里插件可以直接用的 crate。
const ALLOWED_CORE: &[&str] = &[
    "kotone-plugin-api",
    "kotone-core",
    "kotone-platform-windows",
];

#[test]
fn plugins_depend_only_on_the_contract_core_and_stateless_system_calls() {
    let all = plugins();
    assert!(
        all.len() >= 4,
        "插件目录里应该有英雄联盟、游戏语音、音乐、同传……"
    );
    let plugin_crates: Vec<String> = all
        .iter()
        .filter_map(|(_, t)| t.get("package")?.get("name")?.as_str().map(str::to_string))
        .collect();
    for (dir, table) in &all {
        for dep in dependencies(table) {
            assert!(
                !dep.starts_with("tauri"),
                "插件 {dir} 依赖了 {dep}：插件不碰 Tauri（经宿主）"
            );
            assert!(
                !plugin_crates.contains(&dep),
                "插件 {dir} 依赖了别的插件 {dep}：插件之间不直接调用"
            );
            if dep.starts_with("kotone-") && !dep.starts_with("kotone-plugin-") {
                assert!(
                    ALLOWED_CORE.contains(&dep.as_str()),
                    "插件 {dir} 依赖了本体的 {dep}：只能用插件契约、kotone-core 和系统适配层（ADR 038 D6）"
                );
            }
        }
    }
}

/// 全进程只有一份的东西：低级钩子（Esc、呼叫键）、输入闸门。插件的源码里不出现，要用就经宿主。
const PROCESS_GLOBAL: &[&str] = &["hotkey_ll", "input_gate", "SetWindowsHookEx"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn plugins_never_touch_process_global_hooks() {
    for (dir, _) in plugins() {
        let mut files = Vec::new();
        rust_files(&repo().join("plugins").join(&dir).join("src"), &mut files);
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for word in PROCESS_GLOBAL {
                assert!(
                    !text.contains(word),
                    "{} 用了 {word}：全进程只有一份的钩子 / 闸门只经宿主（ADR 038“怎么定的”）",
                    file.display()
                );
            }
        }
    }
}

#[test]
fn the_rule_catches_a_violation() {
    let bad: toml::Table = r#"
        [package]
        name = "kotone-plugin-bad"
        [dependencies]
        kotone-agent = { workspace = true }
        [target.'cfg(windows)'.dependencies]
        tauri = "2"
        renamed = { package = "kotone-ai", path = "x" }
    "#
    .parse()
    .unwrap();
    let deps = dependencies(&bad);
    assert!(deps.contains(&"kotone-agent".to_string()));
    assert!(deps.contains(&"tauri".to_string()));
    assert!(deps.contains(&"kotone-ai".to_string()), "改名的按真名");
}
