//! silero-vad ONNX 实现（ADR-007）：core `Vad` 端口的生产实现。
//!
//! 推理后端选型：**复用 sherpa-onnx crate 的 `VoiceActivityDetector`**
//! （其内部即 ONNX Runtime + silero VAD），不引入独立 `ort` crate——
//! 零新增 crate 依赖，与 engine-sherpa 共享同一份原生库（feature 统一
//! 编译时只链接一次）。feature `vad-silero` 控制编译（默认关），
//! 默认构建零原生依赖。
//!
//! sherpa 的 VAD 是 segment 级检测器；本实现默认把内部平滑参数调到最小
//! （min_speech/min_silence = 50ms ≈ 帧级原始判定），判停阈值与最短会话
//! 保护全部在 core 的 `SilenceStopTracker`（纯逻辑可单测）。帧级判定参数
//! （threshold / min_speech / min_silence）可从 settings.vad 覆盖，默认即原值。

/// settings.vad → silero 参数（threshold, min_speech_sec, min_silence_sec），
/// 带防御性 clamp（与 UI 滑块范围一致）。纯函数，便于不加载 ONNX 模型单测。
pub(crate) fn silero_params(v: &kotone_core::settings::VadConfig) -> (f32, f32, f32) {
    (
        v.threshold.clamp(0.1, 0.9),
        v.min_speech_ms.clamp(20, 500) as f32 / 1000.0,
        v.min_silence_ms.clamp(20, 500) as f32 / 1000.0,
    )
}

#[cfg(feature = "vad-silero")]
mod imp {
    use std::sync::Arc;

    use sherpa_onnx::{VadModelConfig, VoiceActivityDetector};

    use kotone_core::vad::{Vad, VadFactory};

    use super::silero_params;

    /// silero VAD：每会话一个实例（orchestrator 经工厂创建）
    pub struct SileroVad {
        inner: VoiceActivityDetector,
    }

    impl SileroVad {
        pub fn new(vad_settings: &kotone_core::settings::VadConfig) -> Result<Self, String> {
            if !crate::model::vad_model_ready() {
                return Err("silero VAD 模型未下载。请运行 kotone-cli download silero-vad".into());
            }
            let model = crate::model::vad_model_path();
            let mut config = VadModelConfig::default();
            config.silero_vad.model = Some(model.to_string_lossy().into_owned());
            // 帧级判定参数来自 orchestrator 的 SharedState 快照；
            // 每会话新建实例，改动即时生效，且不产生额外磁盘读取。
            // 判停逻辑仍归 core tracker（vad_silence_ms）
            let (threshold, min_speech_sec, min_silence_sec) = silero_params(vad_settings);
            config.silero_vad.threshold = threshold;
            config.silero_vad.min_speech_duration = min_speech_sec;
            config.silero_vad.min_silence_duration = min_silence_sec;
            config.silero_vad.window_size = 512; // silero 16kHz 标准窗口（32ms）
            config.sample_rate = 16000;
            config.num_threads = 1;
            config.provider = Some("cpu".into());
            let inner = VoiceActivityDetector::create(&config, 60.0).ok_or_else(|| {
                format!(
                    "silero VAD 创建失败（模型文件损坏？路径：{}）",
                    model.display()
                )
            })?;
            Ok(Self { inner })
        }
    }

    impl Vad for SileroVad {
        fn push_frame(&mut self, frame: &[f32]) -> Result<bool, String> {
            self.inner.accept_waveform(frame);
            // 完成的 segment 直接出队丢弃：音频归 STT 管，这里只要帧级判定；
            // 不出队会让 60s 环形缓冲被 segment 塞满
            while !self.inner.is_empty() {
                self.inner.pop();
            }
            Ok(self.inner.detected())
        }

        fn reset(&mut self) {
            self.inner.reset();
        }
    }

    /// orchestrator 注入用工厂（每会话一个 SileroVad 实例）
    pub fn silero_factory() -> VadFactory {
        Arc::new(|settings| Ok(Box::new(SileroVad::new(settings)?) as Box<dyn Vad>))
    }
}

#[cfg(feature = "vad-silero")]
pub use imp::{silero_factory, SileroVad};

/// VAD 是否编译进当前构建（feature vad-silero）
pub const fn compiled() -> bool {
    cfg!(feature = "vad-silero")
}

#[cfg(all(test, feature = "vad-silero"))]
mod tests {
    use super::*;
    use kotone_core::vad::Vad;

    /// 静音帧不会误判为语音（模型缺失的机器上跳过）
    #[test]
    fn silence_frame_is_not_speech() {
        if !crate::model::vad_model_ready() {
            return;
        }
        let mut vad = SileroVad::new(&kotone_core::settings::VadConfig::default()).unwrap();
        for _ in 0..20 {
            let speech = vad.push_frame(&vec![0.0f32; 480]).unwrap();
            assert!(!speech, "纯静音不应判定为语音");
        }
    }
}

#[cfg(test)]
mod param_tests {
    use super::*;
    use kotone_core::settings::VadConfig;

    #[test]
    fn defaults_pass_through() {
        let (t, sp, si) = silero_params(&VadConfig::default());
        assert_eq!(t, 0.5);
        assert_eq!(sp, 0.05);
        assert_eq!(si, 0.05);
    }

    #[test]
    fn clamps_to_allowed_ranges() {
        let v = VadConfig {
            threshold: 9.0,
            min_speech_ms: 1,
            min_silence_ms: 9999,
        };
        let (t, sp, si) = silero_params(&v);
        assert_eq!(t, 0.9);
        assert_eq!(sp, 0.02);
        assert_eq!(si, 0.5);
    }

    #[test]
    fn ms_converted_to_seconds() {
        let v = VadConfig {
            threshold: 0.3,
            min_speech_ms: 120,
            min_silence_ms: 250,
        };
        let (t, sp, si) = silero_params(&v);
        assert_eq!(t, 0.3);
        assert_eq!(sp, 0.12);
        assert_eq!(si, 0.25);
    }
}
