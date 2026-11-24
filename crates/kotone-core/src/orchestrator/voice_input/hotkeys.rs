//! 语音输入的热键：聊天频道切换、文字处理开关 / 切换流程（壳登记、壳调；本体不调）。

use crate::orchestrator::*;

impl Orchestrator {
    // ---------- 频道切换（ADR-008：sticky 频道态，句间动作） ----------

    /// 频道切换键：按声明顺序循环当前激活 profile 的频道，悬浮窗提示新频道。
    /// sticky：切换后保持到下次切换；频道在发送时刻读取，因此 toggle/one-shot/
    /// solo 模式会话进行中按下自然作用于当前这句（hold 模式该组合物理上按不出）。
    /// 单频道 profile 无操作。任意状态都接受（频道与会话状态正交）。
    pub async fn on_cycle_channel(&self) {
        let settings = self.settings.read().unwrap().clone();
        let profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let current = { self.inner.lock().unwrap().voice.channel.clone() };
        // profile 不符（切了游戏适配）的残留视为「位于默认频道」
        let current_id = current
            .filter(|(pid, _)| *pid == profile.id)
            .map(|(_, cid)| cid);
        let Some(next_id) = profile.next_channel_id(current_id.as_deref()) else {
            return; // 单频道：切换键无操作
        };
        let strategy = profile.channel_strategy(Some(&next_id));
        {
            self.inner.lock().unwrap().voice.channel = Some((profile.id.clone(), next_id.clone()));
        }
        self.emitter.emit(
            "kotone://channel",
            json!({
                "channelId": strategy.id,
                "displayName": strategy.display_name,
                "isDefault": strategy.is_default,
            }),
        );
        self.emit_process(
            "channel_switched",
            json!({ "channelId": strategy.id, "profileId": profile.id }),
        );
    }

    // ---------- 文字处理热键（开关 / 切换流程） ----------

    /// 文字处理启用开关热键：切换 `postProcessing.enabled` 并持久化，悬浮窗显示状态。
    /// 与设置页改配置一致，下一句识别按快照生效；任意状态都可切换（与会话正交）。
    pub async fn on_post_processing_toggle(&self) {
        let Some(repo) = self.settings_repository.clone() else {
            crate::log::log("post-processing toggle: 未注入配置仓库，忽略");
            return;
        };
        let next = match repo.update(|settings| {
            settings.post_processing.enabled = !settings.post_processing.enabled;
            Ok(())
        }) {
            Ok(next) => next,
            Err(e) => {
                crate::log::log(&format!("post-processing toggle 保存失败: {e}"));
                return;
            }
        };
        self.emit_pp_status(
            "toggle",
            next.post_processing.enabled,
            next.post_processing.active_pipeline_id,
        );
    }

    /// 切换文字处理流程热键：按声明顺序把 `active_pipeline_id` 前进到下一个流程并
    /// 持久化；不足两条流程时无操作。悬浮窗显示切换到的新流程名。
    pub async fn on_post_processing_cycle(&self) {
        let Some(repo) = self.settings_repository.clone() else {
            crate::log::log("post-processing cycle: 未注入配置仓库，忽略");
            return;
        };
        if repo.snapshot().post_processing.pipelines.len() < 2 {
            // 单流程甚至无流程：切换键无操作（记日志便于排障）
            crate::log::log("post-processing cycle: 流程不足两条，忽略");
            return;
        }
        let next = match repo.update(|settings| {
            let pipelines = &settings.post_processing.pipelines;
            let active = settings.post_processing.active_pipeline_id.clone();
            let cur = pipelines.iter().position(|p| p.id == active).unwrap_or(0);
            let next_id = pipelines[(cur + 1) % pipelines.len()].id.clone();
            settings.post_processing.active_pipeline_id = next_id;
            Ok(())
        }) {
            Ok(next) => next,
            Err(e) => {
                crate::log::log(&format!("post-processing cycle 保存失败: {e}"));
                return;
            }
        };
        self.emit_pp_status(
            "cycle",
            next.post_processing.enabled,
            next.post_processing.active_pipeline_id,
        );
    }

    /// 向悬浮窗广播一次文字处理状态变化（开灯 / 切换流程；`action` 供前端区分文案）。
    fn emit_pp_status(&self, action: &str, enabled: bool, active_pipeline_id: String) {
        let settings = self.settings.read().unwrap().clone();
        let display_name = settings
            .post_processing
            .active_pipeline()
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        self.emitter.emit(
            "kotone://pp-status",
            json!({
                "action": action,
                "enabled": enabled,
                "pipelineId": active_pipeline_id,
                "displayName": display_name,
            }),
        );
    }
}
