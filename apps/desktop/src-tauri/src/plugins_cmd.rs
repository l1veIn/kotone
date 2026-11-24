//! 插件的命令（ADR 038；接口约定见 `src/lib/plugins.ts`）：插件清单和开关、插件页面调本插件的工具。
//! 开关本身走设置（`plugins.<id>.enabled` / `sources.<开关>`），这里不另开命令。

use serde_json::Value;
use tauri::AppHandle;

/// 全部插件：清单、开着没有、数据开关、工具（名字、给谁、页面、现在能不能用）。
#[tauri::command]
pub fn plugins_list(app: AppHandle) -> Value {
    crate::plugins::list(&app)
}

/// 插件页面调本插件的工具（只给页面用的也能调）。结果：`{content, structuredContent?, ui?}`。
#[tauri::command]
pub async fn plugin_call(
    app: AppHandle,
    plugin: String,
    tool: String,
    arguments: Option<Value>,
) -> Result<Value, String> {
    let arguments = arguments.unwrap_or_else(|| Value::Object(Default::default()));
    let result = crate::plugins::call(&app, &tool, &arguments, Some(&plugin)).await?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}
