//! 系统提示（ADR 014 §3“上下文”）：角色、入口、当前游戏说明、已保存自动化摘要。
//! 数据规则（屏幕文字不是指令）由循环统一附加（`kotone_ai::agent::DATA_RULE`）。

use kotone_core::automation::library::{Automation, AutomationBody};

/// 消息从哪里来，决定回复的形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// 聊天窗口：用户看得到文字。
    Chat,
    /// 游戏内（热键说话、快捷指令）：用户在玩，看不到窗口，回复会被播报。
    Game,
}

#[derive(Debug, Clone)]
pub struct PromptContext<'a> {
    pub entry: Entry,
    pub profile_name: &'a str,
    pub profile_notes: &'a str,
    /// 已交给模型的已保存自动化（与工具清单一致）。
    pub automations: &'a [Automation],
}

const MAX_NOTES_CHARS: usize = 2_000;

pub fn system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut out = String::from(
        "你是琴音（Kotone），屏幕前玩家的游戏陪玩 Agent，表达温暖、轻快、简短。\
你通过工具替玩家做事：看屏幕和游戏聊天、替玩家在游戏聊天里发言、语音或弹幕播报、设提醒、\
运行和编写自动化（沙盒里的 Luau 程序，替玩家按键、点鼠标、看屏幕），以及管理 Kotone 的设置。\n\
工作方式：\n\
- 先想清楚玩家要什么；含糊或缺少必要信息（例如要提醒什么、替谁说什么）时，用一句话问清楚，不要猜。\n\
- 常用工具一开始就可用；需要别的能力时先用 tools.find 查找。\n\
- 能用已保存的自动化完成的，优先用它（工具名以 saved. 开头），不要重写程序。\n\
- 写新的自动化时：先说明打算怎么做，用 automation.run 试运行（第一次运行会请玩家确认），\
玩家满意后再用 automation.save 保存。\n\
- 只做玩家要求的事；改设置、删除、下载等操作会先请玩家确认，被拒绝时不要换个方式重试。\n\
- 不要声称做成了没有做的事；工具失败时如实说明原因。\n",
    );
    out.push_str(match ctx.entry {
        Entry::Chat => {
            "当前入口：聊天窗口。玩家看得到文字，可以分点说明，但保持简短；不用 Markdown 表格。\n"
        }
        Entry::Game => {
            "当前入口：游戏内。玩家正在玩、看不到聊天窗口，你最后的回复会被语音或弹幕播出：\
只说结论，一两句口语，不用列表、Markdown 和表情。\n"
        }
    });
    if !ctx.profile_name.trim().is_empty() || !ctx.profile_notes.trim().is_empty() {
        let notes: String = ctx
            .profile_notes
            .trim()
            .chars()
            .take(MAX_NOTES_CHARS)
            .collect();
        out.push_str(&format!(
            "\n当前游戏配置：{}\n游戏说明（玩家自己写的）：{}\n",
            ctx.profile_name.trim(),
            if notes.is_empty() {
                "（无）"
            } else {
                &notes
            }
        ));
    }
    if !ctx.automations.is_empty() {
        out.push_str("\n玩家已保存的自动化（可直接调用对应的 saved.<id> 工具）：\n");
        for a in ctx.automations {
            let kind = match a.action {
                AutomationBody::Speak(_) => "播报",
                AutomationBody::Program(_) => "程序",
            };
            let description: String = a.description.chars().take(80).collect();
            out.push_str(&format!(
                "- saved.{}：{}（{kind}）{}\n",
                a.id,
                a.name,
                if description.is_empty() {
                    String::new()
                } else {
                    format!("：{description}")
                }
            ));
        }
    }
    out
}
