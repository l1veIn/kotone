//! 启动 BetterGI 的任务（ADR 047 第 3 期）：只走它的命令行（0.67.0 `Helpers/CommandLineOptions.cs`）——
//!
//! - 一条龙：`startOneDragon [配置名]`（不写配置名跑它选中的那个）；
//! - 配置组：`--startGroups <组名…>`；
//! - 计划（0.67 起的新任务系统）：`--pulonia-run <计划 ID>`。
//!
//! 只传这三种，名字只能是它文件夹里现有的（不让 `--instance` 这类参数混进来）。BetterGI 已经开着时，第二次启动会经命名管道
//! 把参数转给开着的那个（`InstanceService.DispatchActivation`）；照 0.67.0 的源码，开着的那个只认 `--pulonia-run`
//! （交给计划的队列），`startOneDragon`、`--startGroups` 只是把窗口拿到前面（`HomePageViewModel.HandleActivation` 只认
//! `start`）——所以它开着时只交计划，一条龙和配置组如实说（真机未验证：玩家的 BetterGI 每次启动都以管理员身份重开自己，
//! 这一轮的测试环境没法起它，见 ADR 047 实施记录）。
//!
//! 启动是降权的（ADR 044：`kotone_platform_windows::detached::spawn_detached`，Kotone 是管理员时以桌面为父进程）。
//! BetterGI 一起来就会以管理员身份重开自己（`RuntimeHelper.EnsureElevated` → `runas`），所以 Windows 可能问一次 UAC；
//! Kotone 拿到的进程号是马上退出的那一个（退出码 553）。

use std::path::Path;

use kotone_core::profile::GameProfile;
use serde_json::Value;

use crate::bgi::{self, Install};
use crate::runs::Kind;

/// 原神的两个进程名（游戏配置 `genshin` 里的）。
pub const GAME_PROCESSES: [&str; 2] = ["YuanShen.exe", "GenshinImpact.exe"];

/// 一次启动要做什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    /// 一条龙的配置名（空 = 它选中的那个）、计划的 ID；配置组用 `groups`。
    pub name: String,
    pub groups: Vec<String>,
}

impl Request {
    /// 工具参数 → 请求：`kind` 是 dragon / group / plan，`name` 是配置名 / 组名 / 计划 ID 或计划名，`names` 是几个组名。
    pub fn from_args(args: &Value) -> Result<Request, String> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string()
        };
        let kind = Kind::parse(&text("kind"))
            .ok_or("kind 要写 dragon（一条龙）、group（配置组）或 plan（计划）")?;
        let mut groups: Vec<String> = args
            .get("names")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let name = text("name");
        if kind == Kind::Group && groups.is_empty() && !name.is_empty() {
            groups.push(name.clone());
        }
        groups.dedup();
        if groups.len() > 10 {
            return Err("一次最多跑 10 个配置组".into());
        }
        match kind {
            Kind::Group if groups.is_empty() => {
                Err("配置组要写名字（name，或者几个组的 names）".into())
            }
            Kind::Plan if name.is_empty() => Err("计划要写名字或 ID（name）".into()),
            _ => Ok(Request { kind, name, groups }),
        }
    }

    /// 给卡片和她看的名字。
    pub fn display_name(&self, plans: &[bgi::Plan]) -> String {
        match self.kind {
            Kind::Group => self.groups.join("、"),
            Kind::Plan => plans
                .iter()
                .find(|p| p.id == self.name)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| self.name.clone()),
            _ => self.name.clone(),
        }
    }
}

/// 对着这份安装核对名字（只能是它文件夹里有的），计划名换成 ID。回核对好的请求。
pub fn resolve(
    request: &Request,
    install: &Install,
    version: Option<&str>,
) -> Result<Request, String> {
    let mut out = request.clone();
    let no_dash = |name: &str| !name.starts_with('-');
    match request.kind {
        Kind::Dragon => {
            if !request.name.is_empty() {
                let names = bgi::one_dragon(install);
                if !names.contains(&request.name) || !no_dash(&request.name) {
                    return Err(format!(
                        "BetterGI 里没有叫「{}」的一条龙配置（有：{}）",
                        request.name,
                        list(&names)
                    ));
                }
            }
        }
        Kind::Group => {
            let names: Vec<String> = bgi::groups(install).into_iter().map(|g| g.name).collect();
            if let Some(bad) = request
                .groups
                .iter()
                .find(|g| !names.contains(g) || !no_dash(g))
            {
                return Err(format!(
                    "BetterGI 里没有叫「{bad}」的配置组（有：{}）",
                    list(&names)
                ));
            }
        }
        Kind::Plan => {
            if crate::install::at_least(version, 0, 67) == Some(false) {
                return Err(format!(
                    "这个 BetterGI 是 {}，计划要 0.67 以后的版本",
                    version.unwrap_or("旧版本")
                ));
            }
            let plans = bgi::plans(install);
            let found = plans
                .iter()
                .find(|p| p.id == request.name)
                .or_else(|| plans.iter().find(|p| p.name == request.name))
                .ok_or_else(|| {
                    let names: Vec<String> = plans
                        .iter()
                        .map(|p| format!("{}（{}）", p.name, p.id))
                        .collect();
                    format!(
                        "BetterGI 里没有叫「{}」的计划（有：{}）",
                        request.name,
                        list(&names)
                    )
                })?;
            if !bgi::valid_plan_id(&found.id) {
                return Err(format!("计划 ID「{}」写法不对", found.id));
            }
            out.name = found.id.clone();
        }
        Kind::Task => return Err("kind 要写 dragon、group 或 plan".into()),
    }
    Ok(out)
}

fn list(names: &[String]) -> String {
    if names.is_empty() {
        "一个都没有".into()
    } else {
        names
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("、")
    }
}

/// 命令行参数（只有这三种）。
pub fn args(request: &Request) -> Vec<String> {
    match request.kind {
        Kind::Dragon => {
            let mut args = vec!["startOneDragon".to_string()];
            if !request.name.is_empty() {
                args.push(request.name.clone());
            }
            args
        }
        Kind::Group => std::iter::once("--startGroups".to_string())
            .chain(request.groups.iter().cloned())
            .collect(),
        Kind::Plan => vec!["--pulonia-run".to_string(), request.name.clone()],
        Kind::Task => Vec::new(),
    }
}

/// 原神允不允许自动化输入（游戏配置的开关；找不到原神的配置时和本体一样算允许，`game_tools::check_input`）。
pub fn automation_allowed(profiles: &[GameProfile]) -> Result<(), String> {
    let blocked = GAME_PROCESSES.iter().find_map(|exe| {
        kotone_core::profile::find_by_process(profiles, exe).filter(|p| !p.allow_automation_input)
    });
    match blocked {
        Some(game) => Err(format!(
            "「{}」的游戏配置设成了不允许自动化输入。BetterGI 会替你操作游戏的键盘鼠标，要先在「游戏适配」里打开“允许自动化输入”",
            game.display_name
        )),
        None => Ok(()),
    }
}

/// 真的启动（降权，见模块说明）：回进程号。
pub fn spawn(exe: &Path, args: &[String]) -> Result<u32, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    kotone_platform_windows::detached::spawn_detached(exe, &args)
        .map(|child| child.id())
        .map_err(|e| format!("启动 BetterGI 失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn install() -> (tempfile::TempDir, Install) {
        let dir = tempfile::tempdir().unwrap();
        let bgi = Install {
            dir: dir.path().to_path_buf(),
            exe: dir.path().join("BetterGI.exe"),
        };
        for (path, text) in [
            ("User/OneDragon/默认配置.json", "{}"),
            ("User/OneDragon/--instance.json", "{}"),
            (
                "User/ScriptGroup/锄地.json",
                "{\"name\":\"锄地\",\"projects\":[]}",
            ),
            (
                "User/ScriptGroup/钓鱼.json",
                "{\"name\":\"钓鱼\",\"projects\":[]}",
            ),
            (
                "User/Pulonia/plans/pln_a.json",
                "{\"id\":\"pln_a\",\"name\":\"早上\"}",
            ),
        ] {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        (dir, bgi)
    }

    #[test]
    fn arguments_are_only_the_three_kinds() {
        let (_dir, bgi) = install();
        let req = |v: Value| Request::from_args(&v).and_then(|r| resolve(&r, &bgi, Some("0.67.0")));
        assert_eq!(
            args(&req(json!({"kind": "dragon"})).unwrap()),
            ["startOneDragon"]
        );
        assert_eq!(
            args(&req(json!({"kind": "dragon", "name": "默认配置"})).unwrap()),
            ["startOneDragon", "默认配置"]
        );
        assert_eq!(
            args(&req(json!({"kind": "group", "name": "锄地"})).unwrap()),
            ["--startGroups", "锄地"]
        );
        assert_eq!(
            args(&req(json!({"kind": "group", "names": ["锄地", "钓鱼"]})).unwrap()),
            ["--startGroups", "锄地", "钓鱼"]
        );
        // 计划：写名字也行，传的是 ID
        assert_eq!(
            args(&req(json!({"kind": "plan", "name": "早上"})).unwrap()),
            ["--pulonia-run", "pln_a"]
        );
        assert_eq!(
            args(&req(json!({"kind": "plan", "name": "pln_a"})).unwrap()),
            ["--pulonia-run", "pln_a"]
        );

        // 没有的名字、像参数的名字、乱写的 kind：不启动
        assert!(req(json!({"kind": "dragon", "name": "周末"}))
            .unwrap_err()
            .contains("默认配置"));
        assert!(
            req(json!({"kind": "dragon", "name": "--instance"})).is_err(),
            "像参数的名字"
        );
        assert!(req(json!({"kind": "group", "names": ["锄地", "--pulonia-run"]})).is_err());
        assert!(req(json!({"kind": "group"})).is_err());
        assert!(req(json!({"kind": "plan", "name": "晚上"}))
            .unwrap_err()
            .contains("早上（pln_a）"));
        assert!(req(json!({"kind": "plan"})).is_err());
        assert!(req(json!({"kind": "start"})).is_err());
        assert!(req(
            json!({"kind": "group", "names": (0..11).map(|i| i.to_string()).collect::<Vec<_>>()})
        )
        .is_err());
        // 旧版本没有计划
        let old = Request::from_args(&json!({"kind": "plan", "name": "早上"})).unwrap();
        assert!(resolve(&old, &bgi, Some("0.66.0"))
            .unwrap_err()
            .contains("0.67"));
        assert!(
            resolve(&old, &bgi, None).is_ok(),
            "读不出版本时照文件夹里有没有计划"
        );
    }

    #[test]
    fn display_names() {
        let plans = vec![bgi::Plan {
            id: "pln_a".into(),
            name: "早上".into(),
            description: String::new(),
        }];
        let group = Request {
            kind: Kind::Group,
            name: String::new(),
            groups: vec!["锄地".into(), "钓鱼".into()],
        };
        assert_eq!(group.display_name(&plans), "锄地、钓鱼");
        let plan = Request {
            kind: Kind::Plan,
            name: "pln_a".into(),
            groups: vec![],
        };
        assert_eq!(plan.display_name(&plans), "早上");
    }

    /// “允许自动化输入”关着时拒绝；开着、找不到原神的配置时照常。
    #[test]
    fn automation_input_follows_the_genshin_profile() {
        let mut genshin = GameProfile::builtin_genshin();
        assert!(automation_allowed(std::slice::from_ref(&genshin)).is_ok());
        assert!(
            automation_allowed(&[GameProfile::builtin_lol()]).is_ok(),
            "没有原神的配置"
        );
        genshin.allow_automation_input = false;
        let err = automation_allowed(&[GameProfile::builtin_lol(), genshin]).unwrap_err();
        assert!(
            err.contains("允许自动化输入") && err.contains("原神"),
            "{err}"
        );
    }
}
