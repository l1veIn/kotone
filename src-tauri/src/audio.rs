//! 音频采集：设备枚举、16kHz mono 采集、PCM 流推送、wav 编码（录档用）
//! 计划依赖 cpal + hound（docs/development.md §5.1）

/// 音频输入设备
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

/// 枚举可用输入设备（占位实现）
pub fn list_devices() -> Vec<AudioDevice> {
    todo!("cpal 设备枚举")
}

/// 启动采集：重采样到 16kHz mono f32 并推入会话（占位实现）
pub fn start_capture(_device_id: &str) -> Result<(), String> {
    todo!("cpal 采集 + 重采样，PCM 流推送 orchestrator；同时推送 RMS 电平事件")
}

/// 停止采集（占位实现）
pub fn stop_capture() {
    todo!("停止采集并释放设备")
}
