//! `file.read` / `file.write` / `file.edit` / `file.find`（形状照 Claude Code 的 Read / Write / Edit / Glob
//! 和 OpenCode 的 read / write / edit / glob / grep）。

use std::path::Path;

use serde_json::{json, Value};

use crate::paths::{self, within};
use crate::text::{self, FileEncoding};
use crate::{ExecConfig, ExecOutcome};

const READ_DEFAULT_LIMIT: usize = 2_000;
const READ_LINE_CHARS: usize = 2_000;
const READ_MAX_BYTES: u64 = 20 * 1024 * 1024;
const FIND_MAX_RESULTS: usize = 200;
const FIND_MAX_ENTRIES: usize = 100_000;
const FIND_CONTENT_MAX_BYTES: u64 = 2 * 1024 * 1024;
const FIND_MATCHES_PER_FILE: usize = 3;

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn arg_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

pub fn read(cfg: &ExecConfig, args: &Value) -> ExecOutcome {
    let path = match arg_str(args, "path").map(|p| paths::resolve(cfg, p)) {
        Some(Ok(p)) => p,
        Some(Err(e)) => return ExecOutcome::error(e),
        None => return ExecOutcome::error("缺少 path"),
    };
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return ExecOutcome::error(format!("读不到 {}：{e}", path.display())),
    };
    if meta.is_dir() {
        return ExecOutcome::error(format!("{} 是文件夹；列文件用 file.find", path.display()));
    }
    if meta.len() > READ_MAX_BYTES {
        return ExecOutcome::error(format!(
            "文件太大（{} MB），最多读 20 MB",
            meta.len() / 1024 / 1024
        ));
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ExecOutcome::error(format!("读不到 {}：{e}", path.display())),
    };
    if text::looks_binary(&bytes) {
        return ExecOutcome::error(format!("{} 是二进制文件，不能按文本读", path.display()));
    }
    let (content, encoding) = text::decode_file(&bytes);
    let lines: Vec<&str> = content
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let total = if content.is_empty() {
        0
    } else if content.ends_with('\n') {
        lines.len() - 1
    } else {
        lines.len()
    };
    let from = arg_usize(args, "offset").unwrap_or(1).max(1);
    let limit = arg_usize(args, "limit")
        .unwrap_or(READ_DEFAULT_LIMIT)
        .clamp(1, READ_DEFAULT_LIMIT);
    let mut body = String::new();
    let mut to = from.saturating_sub(1);
    for (i, line) in lines
        .iter()
        .enumerate()
        .take(total)
        .skip(from - 1)
        .take(limit)
    {
        let shown: String = line.chars().take(READ_LINE_CHARS).collect();
        let cut = if line.chars().count() > READ_LINE_CHARS {
            "…"
        } else {
            ""
        };
        body.push_str(&format!("{:>6}\t{shown}{cut}\n", i + 1));
        to = i + 1;
    }
    let more = to < total;
    let mut result = json!({
        "path": path.display().to_string(),
        "content": body,
        "totalLines": total,
        "fromLine": from,
        "toLine": to,
        "encoding": encoding.label(),
    });
    if more {
        result["note"] = json!(format!("后面还有，offset 用 {} 接着读", to + 1));
    }
    let taints = (!within(&path, &cfg.files_dir()))
        .then(|| format!("读了 Kotone 文件区以外的文件 {}", path.display()));
    ExecOutcome {
        result,
        is_error: false,
        taints,
    }
}

/// 写 / 改之前的检查：不能写的地方（Kotone 自己的设置、记忆…；正在运行的游戏的目录）。
pub fn write_blocked(cfg: &ExecConfig, path: &Path) -> Option<String> {
    let files = cfg.files_dir();
    if within(path, cfg.kotone_home()) && !within(path, &files) {
        return Some(
            "这是 Kotone 自己的文件：改设置用 settings.update，改记忆用 memory，改技能用 skill.manage，不要直接写文件"
                .into(),
        );
    }
    if let Some(game) = paths::game_of(cfg, path) {
        if game.running {
            return Some(format!(
                "《{}》正在运行，退出游戏后再改它目录里的文件",
                game.display
            ));
        }
    }
    None
}

pub fn write(cfg: &ExecConfig, args: &Value, stamp: u128) -> ExecOutcome {
    let path = match arg_str(args, "path").map(|p| paths::resolve(cfg, p)) {
        Some(Ok(p)) => p,
        Some(Err(e)) => return ExecOutcome::error(e),
        None => return ExecOutcome::error("缺少 path"),
    };
    let Some(content) = arg_str(args, "content") else {
        return ExecOutcome::error("缺少 content");
    };
    if let Some(why) = write_blocked(cfg, &path) {
        return ExecOutcome::error(why);
    }
    if path.is_dir() {
        return ExecOutcome::error(format!("{} 是文件夹", path.display()));
    }
    let existing = path.exists();
    let (encoding, backup) = if existing {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return ExecOutcome::error(format!("读不到 {}：{e}", path.display())),
        };
        let encoding = if text::looks_binary(&bytes) {
            FileEncoding::Utf8
        } else {
            text::decode_file(&bytes).1
        };
        match backup_if_outside(cfg, &path, stamp) {
            Ok(b) => (encoding, b),
            Err(e) => return ExecOutcome::error(e),
        }
    } else {
        (FileEncoding::Utf8, None)
    };
    let (bytes, used) = encoding.encode(content);
    if let Err(e) = paths::write_atomic(&path, &bytes) {
        return ExecOutcome::error(e);
    }
    ExecOutcome::ok(json!({
        "path": path.display().to_string(),
        "created": !existing,
        "bytes": bytes.len(),
        "encoding": used.label(),
        "backup": backup.map(|b| b.display().to_string()),
    }))
}

pub fn edit(cfg: &ExecConfig, args: &Value, stamp: u128) -> ExecOutcome {
    let path = match arg_str(args, "path").map(|p| paths::resolve(cfg, p)) {
        Some(Ok(p)) => p,
        Some(Err(e)) => return ExecOutcome::error(e),
        None => return ExecOutcome::error("缺少 path"),
    };
    let (Some(old), Some(new)) = (arg_str(args, "old"), arg_str(args, "new")) else {
        return ExecOutcome::error("缺少 old 或 new");
    };
    if old.is_empty() {
        return ExecOutcome::error("old 不能为空；新建文件用 file.write");
    }
    if old == new {
        return ExecOutcome::error("old 和 new 一样，不用改");
    }
    if let Some(why) = write_blocked(cfg, &path) {
        return ExecOutcome::error(why);
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ExecOutcome::error(format!("读不到 {}：{e}", path.display())),
    };
    if text::looks_binary(&bytes) {
        return ExecOutcome::error(format!("{} 是二进制文件，不能改", path.display()));
    }
    let (content, encoding) = text::decode_file(&bytes);
    let mut old = old.to_string();
    let mut new = new.to_string();
    let mut count = content.matches(old.as_str()).count();
    // 文件是 CRLF、模型给的是 LF：换成 CRLF 再找一次。
    if count == 0 && content.contains("\r\n") && old.contains('\n') && !old.contains("\r\n") {
        let crlf = old.replace('\n', "\r\n");
        let n = content.matches(crlf.as_str()).count();
        if n > 0 {
            old = crlf;
            new = new.replace("\r\n", "\n").replace('\n', "\r\n");
            count = n;
        }
    }
    if count != 1 {
        return ExecOutcome::error(format!(
            "old 在文件里出现了 {count} 次，要恰好一次：先 file.read 看原文，多带几行上下文"
        ));
    }
    let next = content.replacen(old.as_str(), new.as_str(), 1);
    let backup = match backup_if_outside(cfg, &path, stamp) {
        Ok(b) => b,
        Err(e) => return ExecOutcome::error(e),
    };
    let (out, used) = encoding.encode(&next);
    if let Err(e) = paths::write_atomic(&path, &out) {
        return ExecOutcome::error(e);
    }
    ExecOutcome::ok(json!({
        "path": path.display().to_string(),
        "replaced": 1,
        "encoding": used.label(),
        "backup": backup.map(|b| b.display().to_string()),
    }))
}

fn backup_if_outside(
    cfg: &ExecConfig,
    path: &Path,
    stamp: u128,
) -> Result<Option<std::path::PathBuf>, String> {
    if within(path, &cfg.files_dir()) || !path.exists() {
        return Ok(None);
    }
    paths::backup(cfg, path, stamp).map(Some)
}

pub fn find(cfg: &ExecConfig, args: &Value) -> ExecOutcome {
    let Some(pattern) = arg_str(args, "pattern")
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return ExecOutcome::error("缺少 pattern（例如 **/*.lua 或 *.toc）");
    };
    let base = match arg_str(args, "path") {
        Some(p) => match paths::resolve(cfg, p) {
            Ok(p) => p,
            Err(e) => return ExecOutcome::error(e),
        },
        None => {
            let files = cfg.files_dir();
            let _ = std::fs::create_dir_all(&files);
            paths::real(&files)
        }
    };
    if !base.is_dir() {
        return ExecOutcome::error(format!("{} 不是文件夹", base.display()));
    }
    let by_path = pattern.contains('/') || pattern.contains('\\');
    let glob = match glob::Pattern::new(&pattern.replace('\\', "/")) {
        Ok(g) => g,
        Err(e) => return ExecOutcome::error(format!("pattern 写法不对：{e}")),
    };
    let options = glob::MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    let contains = arg_str(args, "contains")
        .filter(|c| !c.is_empty())
        .map(str::to_lowercase);
    let mut found = Vec::new();
    let mut truncated = false;
    let mut any_match = false;
    for (seen, entry) in walkdir::WalkDir::new(&base)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .enumerate()
    {
        if seen >= FIND_MAX_ENTRIES || found.len() >= FIND_MAX_RESULTS {
            truncated = true;
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&base)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        let hit = if by_path {
            glob.matches_with(&rel, options)
        } else {
            glob.matches_with(&entry.file_name().to_string_lossy(), options)
        };
        if !hit {
            continue;
        }
        let meta = entry.metadata().ok();
        let mut item = json!({
            "path": entry.path().display().to_string(),
            "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
            "modifiedMs": meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64),
        });
        if let Some(needle) = &contains {
            let matches = content_matches(entry.path(), needle);
            if matches.is_empty() {
                continue;
            }
            any_match = true;
            item["matches"] = Value::Array(matches);
        }
        found.push(item);
    }
    let taints = (any_match && !within(&base, &cfg.files_dir()))
        .then(|| format!("读了 Kotone 文件区以外的文件内容（{}）", base.display()));
    ExecOutcome {
        result: json!({
            "base": base.display().to_string(),
            "files": found,
            "truncated": truncated,
        }),
        is_error: false,
        taints,
    }
}

fn content_matches(path: &Path, needle: &str) -> Vec<Value> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > FIND_CONTENT_MAX_BYTES {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    if text::looks_binary(&bytes) {
        return Vec::new();
    }
    let (content, _) = text::decode_file(&bytes);
    content
        .lines()
        .enumerate()
        .filter(|(_, l)| l.to_lowercase().contains(needle))
        .take(FIND_MATCHES_PER_FILE)
        .map(|(i, l)| json!({"line": i + 1, "text": l.chars().take(200).collect::<String>()}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameDir;

    struct Env {
        _dir: tempfile::TempDir,
        root: std::path::PathBuf,
        cfg: ExecConfig,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = paths::real(dir.path());
        let cfg = ExecConfig {
            home: root.join("kotone").join("agent"),
            game_dirs: vec![GameDir {
                game_id: "wow".into(),
                display: "魔兽世界".into(),
                dir: root.join("games").join("WoW"),
                running: false,
            }],
        };
        std::fs::create_dir_all(root.join("games").join("WoW")).unwrap();
        Env {
            _dir: dir,
            root,
            cfg,
        }
    }

    #[test]
    fn read_pages_with_line_numbers_and_marks_outside_files() {
        let e = env();
        let inside = e.cfg.files_dir().join("a.txt");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, "一\r\n二\n三\n").unwrap();
        let r = read(&e.cfg, &json!({"path": "a.txt", "offset": 2, "limit": 1}));
        assert!(!r.is_error, "{:?}", r.result);
        assert_eq!(r.result["content"], "     2\t二\n");
        assert_eq!(r.result["totalLines"], 3);
        assert!(r.result["note"].as_str().unwrap().contains("offset 用 3"));
        assert!(r.taints.is_none());

        // 不是 UTF-8 的文件按这台机器的 ANSI 编码读（中文 Windows 是 GBK，CI 是 Windows-1252）。
        let (ansi, sample) = text::ansi_sample();
        let outside = e.root.join("games").join("WoW").join("Config.wtf");
        std::fs::write(&outside, ansi.encode(&format!("{sample}\n")).0).unwrap();
        let r = read(&e.cfg, &json!({"path": outside.display().to_string()}));
        assert_eq!(r.result["content"], format!("     1\t{sample}\n"));
        assert!(r.taints.unwrap().contains("Config.wtf"));

        std::fs::write(e.cfg.files_dir().join("b.bin"), [0u8, 1, 2, 0]).unwrap();
        assert!(read(&e.cfg, &json!({"path": "b.bin"})).is_error);
    }

    #[test]
    fn write_backs_up_outside_files_keeps_encoding_and_blocks_kotone_and_running_games() {
        let mut e = env();
        let r = write(
            &e.cfg,
            &json!({"path": "notes/plan.md", "content": "计划"}),
            1,
        );
        assert!(!r.is_error, "{:?}", r.result);
        assert_eq!(r.result["created"], true);
        assert!(r.result["backup"].is_null());

        let target = e
            .root
            .join("games")
            .join("WoW")
            .join("WTF")
            .join("Config.wtf");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        // 原文件是这台机器的 ANSI 编码（中文 Windows 是 GBK，CI 是 Windows-1252），改完保持原编码。
        let (ansi, sample) = text::ansi_sample();
        let (old_text, new_text) = (format!("{sample} 1\n"), format!("{sample} 2\n"));
        std::fs::write(&target, ansi.encode(&old_text).0).unwrap();
        let r = write(
            &e.cfg,
            &json!({"path": target.display().to_string(), "content": new_text}),
            7,
        );
        assert!(!r.is_error, "{:?}", r.result);
        let backup = r.result["backup"].as_str().unwrap();
        assert_eq!(
            std::fs::read(backup).unwrap(),
            ansi.encode(&old_text).0.to_vec()
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            ansi.encode(&new_text).0.to_vec()
        );

        let config = e.cfg.kotone_home().join("config.json");
        let r = write(
            &e.cfg,
            &json!({"path": config.display().to_string(), "content": "{}"}),
            8,
        );
        assert!(r.is_error);
        assert!(!config.exists());

        e.cfg.game_dirs[0].running = true;
        let r = write(
            &e.cfg,
            &json!({"path": target.display().to_string(), "content": "x"}),
            9,
        );
        assert!(r.result.as_str().unwrap().contains("正在运行"));
    }

    #[test]
    fn edit_needs_exactly_one_match_and_handles_crlf() {
        let e = env();
        let path = e.cfg.files_dir().join("a.lua");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "local a = 1\r\nlocal b = 1\r\n").unwrap();
        let r = edit(
            &e.cfg,
            &json!({"path": "a.lua", "old": "= 1", "new": "= 2"}),
            1,
        );
        assert!(r.result.as_str().unwrap().contains("2 次"));
        let r = edit(
            &e.cfg,
            &json!({"path": "a.lua", "old": "local a = 1\nlocal b", "new": "local a = 3\nlocal b"}),
            1,
        );
        assert!(!r.is_error, "{:?}", r.result);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "local a = 3\r\nlocal b = 1\r\n"
        );
        assert!(edit(&e.cfg, &json!({"path": "a.lua", "old": "", "new": "x"}), 1).is_error);
    }

    #[test]
    fn find_globs_by_name_or_path_and_filters_by_content() {
        let e = env();
        let addons = e
            .root
            .join("games")
            .join("WoW")
            .join("Interface")
            .join("AddOns");
        for (name, body) in [
            ("DBM/DBM.toc", "## Title: DBM"),
            ("Bagnon/Bagnon.toc", "## Title: Bagnon"),
            ("DBM/core.lua", "x"),
        ] {
            let p = addons.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        let base = addons.display().to_string();
        let r = find(&e.cfg, &json!({"pattern": "*.toc", "path": base}));
        assert_eq!(r.result["files"].as_array().unwrap().len(), 2);
        assert!(r.taints.is_none());
        let r = find(&e.cfg, &json!({"pattern": "dbm/*", "path": base}));
        assert_eq!(r.result["files"].as_array().unwrap().len(), 2);
        let r = find(
            &e.cfg,
            &json!({"pattern": "*.toc", "path": base, "contains": "bagnon"}),
        );
        let files = r.result["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["matches"][0]["line"], 1);
        assert!(r.taints.is_some());
    }
}
