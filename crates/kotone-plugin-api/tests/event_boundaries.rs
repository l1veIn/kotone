//! 游戏事件的登记只有一种方式（插件重构第 7 步；ADR 036 D4、ADR 038 D3 第 10 条；软件哲学原则 15、16）：
//! 本体只登记任何游戏都有的 `game.*`；游戏专用的事由带适配器的插件在 `plugins/<id>/plugin.toml` 的 `[[events]]`
//! 里登记。所以插件登记的事件名在本体的代码里一个都不出现——本体的校验、`game.state`、界面里的选择都从合并后的
//! 目录来，不另写一份名单。

use std::path::{Path, PathBuf};

use kotone_plugin_api::Manifest;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// 每个插件清单（`plugins/*/plugin.toml`）。
fn manifests() -> Vec<Manifest> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(repo().join("plugins")).unwrap() {
        let file = entry.unwrap().path().join("plugin.toml");
        if file.is_file() {
            let text = std::fs::read_to_string(&file).unwrap();
            out.push(Manifest::parse(&text).unwrap_or_else(|e| panic!("{}：{e}", file.display())));
        }
    }
    out
}

/// 目录下（递归）后缀在 `exts` 里的文件。跳过生成的文案代码（`paraglide`）和浏览器预览里假装插件后端的
/// `*Mock.ts`（它们是插件的替身，不是本体）。
fn files(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "paraglide" && name != "node_modules" && name != "target" {
                files(&path, exts, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| exts.contains(&e.to_string_lossy().as_ref()))
            && !name.ends_with("Mock.ts")
        {
            out.push(path);
        }
    }
}

/// 本体的代码：`crates/*/src`、`crates/*/tests`、桌面端的后端和界面、本体的文案。
fn body_files() -> Vec<PathBuf> {
    let root = repo();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).unwrap() {
        let krate = entry.unwrap().path();
        files(&krate.join("src"), &["rs"], &mut out);
        files(&krate.join("tests"), &["rs"], &mut out);
    }
    files(&root.join("apps/desktop/src-tauri/src"), &["rs"], &mut out);
    files(&root.join("apps/desktop/src"), &["ts", "svelte"], &mut out);
    files(&root.join("apps/desktop/messages"), &["json"], &mut out);
    out
}

/// 在这些文件里出现的（文件，事件名）。
fn mentions(files: &[PathBuf], kinds: &[String]) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for kind in kinds {
            if text.contains(kind.as_str()) {
                out.push((file.clone(), kind.clone()));
            }
        }
    }
    out
}

fn plugin_event_names() -> Vec<String> {
    manifests()
        .iter()
        .flat_map(|m| m.events.iter().map(|e| e.kind.clone()))
        .collect()
}

#[test]
fn plugin_event_names_never_appear_in_the_body() {
    let kinds = plugin_event_names();
    assert!(
        !kinds.is_empty(),
        "英雄联盟插件应该登记了自己的事（plugins/lol/plugin.toml 的 [[events]]）"
    );
    let found = mentions(&body_files(), &kinds);
    assert!(
        found.is_empty(),
        "本体里写着插件登记的事件名（应该从合并后的事件目录读）：{:#?}",
        found
    );
    // 本体自己只登记通用的 game.*
    assert!(kotone_core::games::generic_events()
        .iter()
        .all(|e| e.kind.starts_with("game.")));
}

#[test]
fn every_plugin_event_name_is_unique_and_under_its_adapter() {
    let mut seen = std::collections::HashSet::new();
    for m in manifests() {
        for e in &m.events {
            let adapter = m.adapter.as_deref().unwrap();
            assert!(e.kind.starts_with(&format!("{adapter}.")), "{}", e.kind);
            assert!(seen.insert(e.kind.clone()), "两个插件都登记了 {}", e.kind);
        }
    }
}

/// 规则真能抓到违反：插件自己的源码里当然写着这些名字。
#[test]
fn the_scan_catches_a_mention() {
    let kinds = plugin_event_names();
    let mut plugin_files = Vec::new();
    files(&repo().join("plugins/lol/src"), &["rs"], &mut plugin_files);
    assert!(!mentions(&plugin_files, &kinds).is_empty());
}
