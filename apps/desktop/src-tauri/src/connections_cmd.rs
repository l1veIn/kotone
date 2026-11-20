//! 连接与文字处理命令：预设、增删连接、测试文字处理。

use super::*;

/// 已注册后处理模块的只读发现接口；设置页不维护硬编码模块清单。
#[tauri::command]
pub(crate) fn list_post_processors(
    state: tauri::State<SharedState>,
) -> Vec<kotone_core::postprocess::ProcessorDescriptor> {
    state.processors.list_info()
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionPresetInfo {
    id: String,
    display_name: String,
    /// `recommended` / `china` / `global` / `aggregator` / `custom`
    group: String,
    description: String,
    default_base_url: String,
    default_model: String,
    models: Vec<String>,
    api_key_url: Option<String>,
}

#[tauri::command]
pub(crate) fn list_connection_presets() -> Vec<ConnectionPresetInfo> {
    kotone_core::connection::CONNECTION_PRESETS
        .iter()
        .map(|preset| ConnectionPresetInfo {
            id: preset.id.into(),
            display_name: preset.display_name.into(),
            group: preset.group.into(),
            description: preset.description.into(),
            default_base_url: preset.default_base_url.into(),
            default_model: preset.default_model.into(),
            models: preset.models.iter().map(|m| m.to_string()).collect(),
            api_key_url: preset.api_key_url.map(str::to_string),
        })
        .collect()
}

#[tauri::command]
pub(crate) fn list_connections(
    state: tauri::State<SharedState>,
) -> Result<Vec<kotone_ai::connections::ConnectionInfo>, String> {
    let settings = state.settings.read().unwrap();
    kotone_ai::connections::list_connection_info(&settings, state.secrets.as_ref())
}

#[tauri::command]
pub(crate) fn upsert_connection(
    state: tauri::State<SharedState>,
    connection: Connection,
    api_key: Option<String>,
) -> Result<Settings, String> {
    kotone_ai::connections::upsert_connection(
        &state.settings_repository,
        state.secrets.as_ref(),
        connection,
        api_key,
    )
}

#[tauri::command]
pub(crate) fn delete_connection(
    state: tauri::State<SharedState>,
    id: String,
) -> Result<Settings, String> {
    kotone_ai::connections::delete_connection(
        &state.settings_repository,
        state.secrets.as_ref(),
        &id,
    )
}

/// 独立试跑一条后处理 pipeline；不经过 orchestrator，因此不会写历史或触发注入。
/// 未传 pipeline 时使用当前设置的 pipeline 快照，即使总开关关闭也会执行其步骤。
#[tauri::command]
pub(crate) async fn test_post_processing(
    state: tauri::State<'_, SharedState>,
    text: String,
    pipeline: Option<kotone_core::postprocess::PipelineConfig>,
) -> Result<kotone_core::postprocess::PostProcessingTestResult, String> {
    let pipeline = pipeline.unwrap_or_else(|| {
        state
            .settings
            .read()
            .unwrap()
            .post_processing
            .active_pipeline()
            .cloned()
            .unwrap_or_default()
    });
    kotone_core::postprocess::test_post_processing(text, pipeline, &state.processors).await
}
