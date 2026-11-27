//! BetterGI 写的文件（**只读**，0.67.0 核对过格式，ADR 047 第 3 期）。
//!
//! 只读格式和键名，不碰玩家的私人内容以外的东西；玩家的配置、日志是私人数据，不进仓库、不进测试样例（样例是合成的）。
//! 格式出处（只看、不抄：BetterGI 是 GPL-3.0）：`babalae/better-genshin-impact` 的 0.67.0 标签——
//!
//! - `User/config.json`：System.Text.Json，camelCase，缩进 2，UTF-8 不带 BOM（`Service/ConfigService.cs`）。用到的键：
//!   `notificationConfig.webhookEnabled` / `webhookEndpoint`、`selectedOneDragonFlowConfigName`、
//!   `externalAccessConfig.httpApiEnabled` / `webSocketEnabled` / `mcpEnabled` / `port`（默认 30648）/ `accessToken`、
//!   `hotKeyConfig.cancelTaskHotkey`（“停止任意独立任务”的热键，默认空）。
//! - `User/OneDragon/<配置名>.json`：一条龙的配置（Newtonsoft，文件名就是配置名，`Core/Config/OneDragonFlowConfigStore.cs`）。
//! - `User/ScriptGroup/<组名>.json`：配置组（camelCase：`name`、`projects`，`Core/Script/Group/ScriptGroup.cs`）。
//! - `User/Pulonia/`（新任务系统，`Pulonia/Services/PuloniaTaskStore.cs`，Newtonsoft + snake_case，枚举写成数字）：
//!   `plans/<计划 ID>.json`（`id`、`name`、`description`）、`plan-order.json`（`plan_order`）、`instances/<实例>/state.json`（`active_run`、`pending_requests`）、
//!   `history/<计划 ID>/<运行 ID>.json`（`plan_name`、`status`、`message`、`submitted_at`、`finished_at`）。
//!   计划 ID：1–32 位小写字母、数字、下划线、短横线（`PuloniaTaskValidator.ValidateId`）。
//! - `log/better-genshin-impact<yyyyMMdd>.log`：每条 `[HH:mm:ss.fff] [INF] [实例:会话:进程:线程] 来源` 一行，下面是正文，空行隔开。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// 读 JSON 文件的上限（BetterGI 的配置约 30 KB；计划和运行记录更小）。
const JSON_MAX: u64 = 8 * 1024 * 1024;
/// 每种列表最多列几项。
pub const LIST_MAX: usize = 100;
/// 日志只读尾部这么多字节。
const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// 一份 BetterGI 安装：目录和 `BetterGI.exe`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

impl Install {
    pub fn user(&self) -> PathBuf {
        self.dir.join("User")
    }

    pub fn config_path(&self) -> PathBuf {
        self.user().join("config.json")
    }

    pub fn pulonia(&self) -> PathBuf {
        self.user().join("Pulonia")
    }
}

/// 读一个 JSON 文件（去掉 BOM；太大、读不了、不是 JSON 都是 None）。
pub fn read_json(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > JSON_MAX {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// 去掉看不见的字符（控制字符、零宽、方向控制），截到 `max` 个字。别人写的字（JS 的通知、日志）按不可信处理。
pub fn clean(text: &str, max: usize) -> String {
    let invisible = |c: char| {
        (c.is_control() && c != '\n')
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}')
    };
    let cleaned: String = text.chars().filter(|c| !invisible(*c)).collect();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= max {
        cleaned.to_string()
    } else {
        let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// 第一行（多行的出错信息只留第一行）。
pub fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

// ---------- config.json ----------

/// 本机外部访问服务（0.67 起有，默认关）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct External {
    pub http_api: bool,
    pub web_socket: bool,
    pub mcp: bool,
    pub port: u16,
    /// 访问令牌：只在本机用来查健康状态，不给页面、不给她。
    #[serde(skip)]
    pub token: String,
}

/// config.json 里插件用到的几项。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigBits {
    pub webhook_enabled: bool,
    pub webhook_endpoint: String,
    /// 一条龙选中的配置（`startOneDragon` 不带配置名时跑它）。
    pub selected_dragon: String,
    /// 没有这一节 = 0.67 以前的版本或者还没保存过。
    pub external: Option<External>,
    /// “停止任意独立任务”的热键（空 = 没设）。
    pub cancel_hotkey: String,
}

pub fn config_bits(config: &Value) -> ConfigBits {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").trim().to_string();
    let notification = config.get("notificationConfig");
    let external = config
        .get("externalAccessConfig")
        .filter(|v| v.is_object())
        .map(|e| External {
            http_api: e
                .get("httpApiEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            web_socket: e
                .get("webSocketEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            mcp: e
                .get("mcpEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            port: e
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p >= 1024)
                .unwrap_or(30648),
            token: text(e.get("accessToken")),
        });
    ConfigBits {
        webhook_enabled: notification
            .and_then(|n| n.get("webhookEnabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        webhook_endpoint: text(notification.and_then(|n| n.get("webhookEndpoint"))),
        selected_dragon: text(config.get("selectedOneDragonFlowConfigName")),
        external,
        cancel_hotkey: text(
            config
                .get("hotKeyConfig")
                .and_then(|h| h.get("cancelTaskHotkey")),
        ),
    }
}

// ---------- 一条龙、配置组、计划 ----------

/// 目录里的 `*.json`（不递归），按文件名排序，最多 [`LIST_MAX`] 个。
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))
                        // BetterGI 原子写的临时文件以点开头
                        && !p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with('.'))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.truncate(LIST_MAX);
    out
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 一条龙的配置名（`User/OneDragon/<配置名>.json`）。
pub fn one_dragon(install: &Install) -> Vec<String> {
    json_files(&install.user().join("OneDragon"))
        .iter()
        .map(|p| stem(p))
        .filter(|n| !n.is_empty())
        .collect()
}

/// 一个配置组。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub name: String,
    /// 里面有几项。
    pub projects: usize,
}

/// 配置组（`User/ScriptGroup/<组名>.json`）。命令行 `--startGroups` 按组名找文件，所以名字用文件名。
pub fn groups(install: &Install) -> Vec<Group> {
    json_files(&install.user().join("ScriptGroup"))
        .iter()
        .filter_map(|p| {
            let name = stem(p);
            (!name.is_empty()).then(|| Group {
                projects: read_json(p)
                    .and_then(|v| v.get("projects").and_then(Value::as_array).map(Vec::len))
                    .unwrap_or(0),
                name,
            })
        })
        .collect()
}

/// 一个计划（Pulonia）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 计划 ID 的写法（和 BetterGI 的 `PuloniaTaskValidator.ValidateId` 一样）：命令行只传这样的 ID。
pub fn valid_plan_id(id: &str) -> bool {
    let reserved = |id: &str| {
        matches!(id, "con" | "prn" | "aux" | "nul")
            || ((id.starts_with("com") || id.starts_with("lpt"))
                && id.len() == 4
                && id.as_bytes()[3].is_ascii_digit())
    };
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && !reserved(id)
}

/// 计划（`User/Pulonia/plans/<ID>.json`）；有 `plan-order.json` 时照它的顺序。
pub fn plans(install: &Install) -> Vec<Plan> {
    let root = install.pulonia();
    let mut plans: Vec<Plan> = json_files(&root.join("plans"))
        .iter()
        .filter_map(|p| {
            let v = read_json(p)?;
            let id = v.get("id").and_then(Value::as_str)?.to_string();
            if !valid_plan_id(&id) || id != stem(p) {
                return None;
            }
            Some(Plan {
                name: clean(v.get("name").and_then(Value::as_str).unwrap_or(&id), 80),
                description: clean(
                    v.get("description").and_then(Value::as_str).unwrap_or(""),
                    200,
                ),
                id,
            })
        })
        .collect();
    let order: Vec<String> = read_json(&root.join("plan-order.json"))
        .and_then(|v| {
            // 0.67：{ "schema_version": 1, "plan_order": [...] }（`PuloniaTaskPlanCatalog`）；不认得的写法就按文件名
            v.get("plan_order").and_then(Value::as_array).cloned()
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if !order.is_empty() {
        plans.sort_by_key(|p| {
            order
                .iter()
                .position(|id| id == &p.id)
                .unwrap_or(usize::MAX)
        });
    }
    plans
}

/// 这个安装有没有新任务系统（0.67 起；目录用到时才建，没有 = 没用过计划）。
pub fn has_pulonia(install: &Install) -> bool {
    install.pulonia().is_dir()
}

// ---------- 计划的运行 ----------

/// Pulonia 的运行状态（`PuloniaTaskRunStatus`，Newtonsoft 写成数字；写成名字的也认）。
pub fn run_status(value: Option<&Value>) -> &'static str {
    const NAMES: [&str; 10] = [
        "queued",
        "running",
        "cancelling",
        "succeeded",
        "failed",
        "cancelled",
        "timed_out",
        "interrupted",
        "needs_attention",
        "expired",
    ];
    match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|i| NAMES.get(i as usize))
            .copied()
            .unwrap_or("unknown"),
        Some(Value::String(s)) => {
            let snake: String = s
                .chars()
                .enumerate()
                .flat_map(|(i, c)| {
                    let lower = c.to_ascii_lowercase();
                    if c.is_ascii_uppercase() && i > 0 {
                        vec!['_', lower]
                    } else {
                        vec![lower]
                    }
                })
                .collect();
            NAMES
                .iter()
                .find(|n| **n == snake)
                .copied()
                .unwrap_or("unknown")
        }
        _ => "unknown",
    }
}

/// 一次计划的运行（正在跑的，或者历史里的）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRun {
    /// 历史里的才有（目录名）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub plan_id: String,
    pub plan_name: String,
    pub status: String,
    pub message: String,
    /// ISO 8601（BetterGI 写的原样）。
    pub submitted_at: String,
    pub finished_at: String,
    /// 历史里的：这条记录写下的时间（文件的修改时间，毫秒）；正在跑的是 0。
    #[serde(skip_serializing_if = "is_zero")]
    pub recorded_at_ms: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn plan_run(v: &Value, plan_id: &str) -> PlanRun {
    let text = |key: &str, max: usize| clean(v.get(key).and_then(Value::as_str).unwrap_or(""), max);
    PlanRun {
        plan_id: plan_id.to_string(),
        plan_name: text("plan_name", 80),
        status: run_status(v.get("status")).to_string(),
        message: clean(
            first_line(v.get("message").and_then(Value::as_str).unwrap_or("")),
            200,
        ),
        submitted_at: text("submitted_at", 40),
        finished_at: text("finished_at", 40),
        recorded_at_ms: 0,
    }
}

/// 正在跑的计划和排着队的个数（所有实例的 `instances/*/state.json`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanQueue {
    pub active: Vec<PlanRun>,
    pub pending: usize,
}

pub fn plan_queue(install: &Install) -> PlanQueue {
    let mut out = PlanQueue::default();
    let Ok(rd) = std::fs::read_dir(install.pulonia().join("instances")) else {
        return out;
    };
    for dir in rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .take(16)
    {
        let Some(state) = read_json(&dir.join("state.json")) else {
            continue;
        };
        if let Some(active) = state.get("active_run").filter(|v| v.is_object()) {
            out.active.push(plan_run(active, ""));
        }
        out.pending += state
            .get("pending_requests")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
    }
    out
}

/// 最近的计划运行记录（`history/<计划 ID>/<运行 ID>.json`），新的在前，最多 `limit` 条。只读最新的那几个文件。
pub fn plan_history(install: &Install, limit: usize) -> Vec<PlanRun> {
    let root = install.pulonia().join("history");
    let mut files: Vec<(std::time::SystemTime, String, PathBuf)> = Vec::new();
    let Ok(plans) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    for plan in plans
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .take(LIST_MAX)
    {
        let id = plan
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !valid_plan_id(&id) {
            continue;
        }
        for file in json_files_unsorted(&plan) {
            let time = std::fs::metadata(&file)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            files.push((time, id.clone(), file));
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files
        .into_iter()
        .filter_map(|(time, id, path)| {
            let mut run = plan_run(&read_json(&path)?, &id);
            run.recorded_at_ms = time
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Some(run)
        })
        .take(limit)
        .collect()
}

fn json_files_unsorted(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                })
                .take(2000)
                .collect()
        })
        .unwrap_or_default()
}

// ---------- 日志 ----------

/// 日志里的一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// `HH:mm:ss`
    pub time: String,
    /// `INF` / `WRN` / `ERR` / `FTL`
    pub level: String,
    pub text: String,
}

/// 最新的日志文件（文件名里带日期，按名字排最后的那个）。
pub fn latest_log(install: &Install) -> Option<PathBuf> {
    std::fs::read_dir(install.dir.join("log"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("better-genshin-impact") && n.ends_with(".log"))
        })
        .max()
}

/// 日志尾部最近 `max` 条（只要 INF / WRN / ERR / FTL：调试行里有 PATH 这类和运行无关的内容）。只读最后 64 KB。
pub fn log_tail(install: &Install, max: usize) -> Vec<LogLine> {
    let Some(path) = latest_log(install) else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(LOG_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(LOG_TAIL_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    parse_log(&String::from_utf8_lossy(&bytes), max)
}

/// `[22:27:24.136] [INF] [Primary:S1:P2592:T1] 来源` 这样的一行 → (时间, 级别)。
fn log_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('[')?;
    let (time, rest) = rest.split_once("] [")?;
    let (level, _) = rest.split_once(']')?;
    let ok_time = time.len() >= 8 && time.as_bytes()[2] == b':' && time.as_bytes()[5] == b':';
    let ok_level = level.len() == 3 && level.chars().all(|c| c.is_ascii_uppercase());
    (ok_time && ok_level).then(|| (time[..8].to_string(), level.to_string()))
}

pub fn parse_log(text: &str, max: usize) -> Vec<LogLine> {
    let mut out: Vec<LogLine> = Vec::new();
    let mut current: Option<(String, String, String)> = None;
    let flush = |entry: Option<(String, String, String)>, out: &mut Vec<LogLine>| {
        if let Some((time, level, text)) = entry {
            if matches!(level.as_str(), "INF" | "WRN" | "ERR" | "FTL") && !text.is_empty() {
                out.push(LogLine {
                    time,
                    level,
                    text: clean(&text, 300),
                });
            }
        }
    };
    for line in text.lines() {
        if let Some((time, level)) = log_header(line) {
            flush(current.take(), &mut out);
            current = Some((time, level, String::new()));
        } else if let Some((_, _, body)) = current.as_mut() {
            // 正文只留第一行（异常的堆栈不要）
            if body.is_empty() && !line.trim().is_empty() {
                *body = line.trim().to_string();
            }
        }
    }
    flush(current.take(), &mut out);
    let skip = out.len().saturating_sub(max);
    out.split_off(skip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn install(dir: &Path) -> Install {
        Install {
            dir: dir.to_path_buf(),
            exe: dir.join("BetterGI.exe"),
        }
    }

    fn write(path: PathBuf, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 合成的 config.json（键名照 0.67.0，值是假的）。
    #[test]
    fn config_bits_read_the_keys_kotone_uses() {
        let config = json!({
            "notificationConfig": {"webhookEnabled": true, "webhookEndpoint": " http://127.0.0.1:1/x ", "webhookSendTo": ""},
            "selectedOneDragonFlowConfigName": "默认配置",
            "externalAccessConfig": {"httpApiEnabled": true, "webSocketEnabled": false, "mcpEnabled": false, "port": 31000, "accessToken": "t"},
            "hotKeyConfig": {"cancelTaskHotkey": "Ctrl + F12", "cancelTaskHotkeyType": "KeyboardMonitor"},
        });
        let bits = config_bits(&config);
        assert!(bits.webhook_enabled);
        assert_eq!(bits.webhook_endpoint, "http://127.0.0.1:1/x");
        assert_eq!(bits.selected_dragon, "默认配置");
        let external = bits.external.unwrap();
        assert_eq!(
            (external.http_api, external.port, external.token.as_str()),
            (true, 31000, "t")
        );
        assert_eq!(bits.cancel_hotkey, "Ctrl + F12");
        // 令牌不往外给
        assert!(serde_json::to_value(&external)
            .unwrap()
            .get("token")
            .is_none());

        // 旧版本：没有外部访问服务那一节；端口不合法按默认
        let old = config_bits(&json!({"notificationConfig": {}}));
        assert!(!old.webhook_enabled && old.external.is_none() && old.cancel_hotkey.is_empty());
        let bad_port = config_bits(&json!({"externalAccessConfig": {"port": 80}}));
        assert_eq!(bad_port.external.unwrap().port, 30648);
    }

    #[test]
    fn lists_come_from_the_folders() {
        let dir = tempfile::tempdir().unwrap();
        let bgi = install(dir.path());
        write(
            bgi.user().join("OneDragon/默认配置.json"),
            r#"{"Name":"默认配置"}"#,
        );
        write(bgi.user().join("OneDragon/周末.json"), "{}");
        write(bgi.user().join("OneDragon/.周末.json.tmp.json"), "{}");
        write(
            bgi.user().join("ScriptGroup/锄地.json"),
            r#"{"index":1,"name":"锄地","projects":[{},{}]}"#,
        );
        write(bgi.user().join("ScriptGroup/坏的.json"), "不是 JSON");
        write(
            bgi.pulonia().join("plans/pln_b.json"),
            r#"{"id":"pln_b","name":"晚上","description":"每天"}"#,
        );
        write(
            bgi.pulonia().join("plans/pln_a.json"),
            r#"{"id":"pln_a","name":"早上"}"#,
        );
        // 文件名和 ID 对不上、ID 写法不对的不列
        write(
            bgi.pulonia().join("plans/other.json"),
            r#"{"id":"pln_c","name":"x"}"#,
        );
        write(
            bgi.pulonia().join("plans/Bad.json"),
            r#"{"id":"Bad","name":"x"}"#,
        );
        write(
            bgi.pulonia().join("plan-order.json"),
            r#"{"schema_version":1,"plan_order":["pln_b","pln_a"]}"#,
        );

        assert_eq!(one_dragon(&bgi), ["周末", "默认配置"]);
        let found = groups(&bgi);
        assert_eq!(found.len(), 2);
        assert_eq!(found.iter().find(|g| g.name == "锄地").unwrap().projects, 2);
        assert_eq!(found.iter().find(|g| g.name == "坏的").unwrap().projects, 0);
        let listed = plans(&bgi);
        assert_eq!(
            listed.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["pln_b", "pln_a"]
        );
        assert_eq!(listed[0].description, "每天");
        assert!(has_pulonia(&bgi));

        // 什么都没有的安装
        let empty = tempfile::tempdir().unwrap();
        let none = install(empty.path());
        assert!(
            one_dragon(&none).is_empty() && groups(&none).is_empty() && plans(&none).is_empty()
        );
        assert!(!has_pulonia(&none));
    }

    #[test]
    fn plan_ids_follow_bettergi() {
        for ok in ["pln_abc", "a", "0-1_x", "pln_0123456789abcdef0123456789ab"] {
            assert!(valid_plan_id(ok), "{ok}");
        }
        for bad in [
            "",
            "Pln",
            "_a",
            "-a",
            "a b",
            "a/b",
            "con",
            "com1",
            "lpt9",
            "--startGroups",
            &"a".repeat(33),
        ] {
            assert!(!valid_plan_id(bad), "{bad}");
        }
        assert!(valid_plan_id("com"), "只有 com + 一位数字是保留名");
    }

    #[test]
    fn run_status_reads_numbers_and_names() {
        assert_eq!(run_status(Some(&json!(1))), "running");
        assert_eq!(run_status(Some(&json!(3))), "succeeded");
        assert_eq!(run_status(Some(&json!(4))), "failed");
        assert_eq!(run_status(Some(&json!(9))), "expired");
        assert_eq!(run_status(Some(&json!(10))), "unknown");
        assert_eq!(
            run_status(Some(&json!("NeedsAttention"))),
            "needs_attention"
        );
        assert_eq!(run_status(Some(&json!("TimedOut"))), "timed_out");
        assert_eq!(run_status(None), "unknown");
    }

    #[test]
    fn plan_queue_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let bgi = install(dir.path());
        write(
            bgi.pulonia().join("instances/primary/state.json"),
            r#"{"schema_version":7,"sequence":3,"pending_requests":[{},{}],"active_run":{"plan_name":"早上","status":1,"message":"正在执行\n第二行","submitted_at":"2026-11-28T08:00:00+08:00"},"pending_archives":[]}"#,
        );
        write(
            bgi.pulonia().join("history/pln_a/run_1.json"),
            r#"{"plan_name":"早上","status":3,"message":"完成","submitted_at":"2026-11-27T08:00:00+08:00","finished_at":"2026-11-27T08:30:00+08:00"}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(
            bgi.pulonia().join("history/pln_b/run_2.json"),
            r#"{"plan_name":"晚上","status":4,"message":"进不去游戏","submitted_at":"2026-11-27T20:00:00+08:00","finished_at":"2026-11-27T20:01:00+08:00"}"#,
        );
        let queue = plan_queue(&bgi);
        assert_eq!(queue.pending, 2);
        assert_eq!(queue.active[0].status, "running");
        assert_eq!(queue.active[0].message, "正在执行", "只留第一行");
        let history = plan_history(&bgi, 10);
        assert_eq!(history.len(), 2);
        assert_eq!(
            (history[0].plan_id.as_str(), history[0].status.as_str()),
            ("pln_b", "failed")
        );
        assert_eq!(history[1].status, "succeeded");
        assert!(
            history[0].recorded_at_ms >= history[1].recorded_at_ms && history[1].recorded_at_ms > 0
        );
        assert_eq!(queue.active[0].recorded_at_ms, 0);
        assert_eq!(plan_history(&bgi, 1).len(), 1);
    }

    /// 合成的日志（格式照 0.67.0）：只要 INF / WRN / ERR，正文只留第一行，最近的在后。
    #[test]
    fn log_tail_keeps_the_last_meaningful_lines() {
        let text =
            "[22:27:24.136] [INF] [Primary:S1:P1:T1] BetterGenshinImpact.Service\n实例已启动\n\n\
[22:27:24.191] [DBG] [Primary:S1:P1:T1] BetterGenshinImpact.View\n修改进程PATH为:C:\\x\n\n\
[22:28:00.000] [ERR] [Primary:S1:P1:T1] BetterGenshinImpact.Task\n任务出错\n   at X.Y()\n\n\
[22:29:00.000] [WRN] [Primary:S1:P1:T1] BetterGenshinImpact.Task\n\u{200b}注意\n";
        let lines = parse_log(text, 10);
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.time.as_str(), l.level.as_str(), l.text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("22:27:24", "INF", "实例已启动"),
                ("22:28:00", "ERR", "任务出错"),
                ("22:29:00", "WRN", "注意")
            ]
        );
        assert_eq!(parse_log(text, 1).len(), 1);
        assert_eq!(parse_log(text, 1)[0].level, "WRN");

        let dir = tempfile::tempdir().unwrap();
        let bgi = install(dir.path());
        write(
            bgi.dir.join("log/better-genshin-impact20261101.log"),
            "[01:00:00.000] [INF] [P] S\n旧的\n",
        );
        write(bgi.dir.join("log/better-genshin-impact20261112.log"), text);
        write(
            bgi.dir.join("log/other.log"),
            "[01:00:00.000] [INF] [P] S\n别的\n",
        );
        assert_eq!(log_tail(&bgi, 10).len(), 3);
        assert!(log_tail(&install(&dir.path().join("none")), 10).is_empty());
    }

    #[test]
    fn clean_strips_invisible_characters_and_clips() {
        assert_eq!(clean(" a\u{200b}b\u{202e}c\u{7} ", 10), "abc");
        assert_eq!(clean("一二三四五", 3), "一二…");
        assert_eq!(first_line("\n  第一行 \n第二行"), "第一行");
    }
}
