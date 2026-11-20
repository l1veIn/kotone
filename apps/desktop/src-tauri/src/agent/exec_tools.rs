//! 后台任务的执行命令、读写文件（ADR 030）：“读过外面的东西”标记与执行。

use super::*;

/// 玩家这一轮说的原话和时间（只有玩家说的话能交给后台，ADR 030 第 2 节）。
pub(super) type PlayerWords = Arc<Mutex<Option<(String, u64)>>>;

/// 这一轮主会话从玩家这句话开始读到过外面的东西（不可信的工具结果、附带的截图或数据）：
/// 第一次的说明。交给后台时它继承这个标记（ADR 030 第 3 节）。
pub(super) type TurnTaint = Arc<Mutex<Option<String>>>;

// ---------- 后台执行命令、读写文件（ADR 030） ----------

/// kotone-exec 的配置：Kotone 的 agent 目录；游戏目录来自游戏配置里填的程序路径
/// （`.exe` 所在的文件夹；启动器链接、快捷方式不算），正在运行的不写。
fn exec_config() -> kotone_agent::exec::ExecConfig {
    use kotone_agent::exec::{ExecConfig, GameDir};
    let running: std::collections::HashSet<String> =
        kotone_platform_windows::inject::process_list()
            .into_iter()
            .map(|(_, _, name)| name.to_lowercase())
            .collect();
    let game_dirs = kotone_core::profile::list()
        .into_iter()
        .filter_map(|game| {
            let path = std::path::Path::new(game.launch_path.trim());
            let exe = path.is_absolute()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
            if !exe {
                return None;
            }
            let dir = path.parent()?.to_path_buf();
            let running = game
                .process_names
                .iter()
                .any(|p| running.contains(&p.to_lowercase()));
            Some(GameDir {
                game_id: game.id,
                display: game.display_name,
                dir,
                running,
            })
        })
        .collect();
    ExecConfig {
        home: kotone_core::settings::kotone_dir().join("agent"),
        game_dirs,
    }
}

/// 一件后台的事的“读过外面的东西”（存在它的登记里）。
fn subagent_taint(app: &AppHandle, spec: &ChildSpec) -> kotone_agent::exec::Taint {
    subagents_of(app, &spec.parent)
        .iter()
        .find(|a| a.name == spec.name)
        .map(kotone_agent::subagents::taint_of)
        .unwrap_or_default()
}

/// 后台读到了外面的东西（只记第一次）。
pub(super) fn mark_subagent_taint(app: &AppHandle, spec: &ChildSpec, why: &str) {
    let marked = subagents_of(app, &spec.parent)
        .iter()
        .any(|a| a.name == spec.name && a.taint.is_some());
    if !marked {
        update_subagent(app, &spec.parent, &spec.name, |a| {
            a.taint.get_or_insert_with(|| why.to_string());
        });
    }
}

/// 后台执行命令、读写文件：判断、经主会话问玩家（语音时开口问，打字时是卡片按钮）、执行、
/// 记下读没读到外面的东西（流程在 [`kotone_agent::subagents::run_exec`]）。主会话没有这些工具
/// （清单里就没给），这里再挡一遍。`confirm`：玩家把这个工具设成了“每次询问”。
pub(super) async fn exec_tool(
    app: &AppHandle,
    caller: &str,
    tool: &str,
    arguments: &Value,
    confirm: bool,
    cancel: &CancellationToken,
) -> ToolOutcome {
    let Some(spec) = child_spec(app, caller) else {
        return ToolOutcome::error("执行命令、读写文件只在后台做：用 agent.start 交给后台");
    };
    let cfg = exec_config();
    let mut taint = subagent_taint(app, &spec);
    let before = taint.clone();
    let outcome = kotone_agent::subagents::run_exec(
        &cfg,
        &mut taint,
        tool,
        arguments,
        confirm,
        cancel,
        |question| {
            log::log(&format!("agent: {} asks before {tool}", spec.name));
            subagent_ask(app, &spec, caller, question, cancel)
        },
    )
    .await;
    if outcome.is_error && outcome.content.starts_with("没有执行") {
        log::log(&format!("agent: {} refused {tool}", spec.name));
    }
    if taint != before {
        update_subagent(app, &spec.parent, &spec.name, |a| {
            kotone_agent::subagents::store_taint(a, &taint)
        });
    }
    outcome
}
