//! 麦克风采集期间降低 Windows 默认播放设备的音量。

use std::sync::{Arc, Mutex};

/// 共用一个系统音量快照，直至最后一个录音结束。
#[derive(Default)]
pub struct RecordingOutputVolume {
    state: Mutex<RecordingOutputVolumeState>,
}

#[derive(Default)]
struct RecordingOutputVolumeState {
    lease: Option<OutputVolumeLease>,
    active: usize,
    closed: bool,
}

impl RecordingOutputVolume {
    pub fn acquire(self: &Arc<Self>) -> Result<RecordingOutputVolumeGuard, String> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err("应用正在退出".into());
        }
        if state.lease.is_none() {
            state.lease = Some(OutputVolumeLease::acquire()?);
        }
        state.active += 1;
        Ok(RecordingOutputVolumeGuard(self.clone()))
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        drop(state.lease.take());
    }
}

pub struct RecordingOutputVolumeGuard(Arc<RecordingOutputVolume>);

impl Drop for RecordingOutputVolumeGuard {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.active -= 1;
        if state.active == 0 {
            drop(state.lease.take());
        }
    }
}

/// 释放时恢复同一播放设备的原音量。调用方应在重叠采集期间共用一个句柄。
pub struct OutputVolumeLease {
    #[cfg(windows)]
    original: platform::PlaybackVolume,
    #[cfg(not(windows))]
    _private: (),
}

impl OutputVolumeLease {
    pub fn acquire() -> Result<Self, String> {
        #[cfg(windows)]
        {
            let original = platform::lower_default_volume()?;
            Ok(Self { original })
        }
        #[cfg(not(windows))]
        Err("降低扬声器音量仅支持 Windows".into())
    }
}

#[cfg(windows)]
impl Drop for OutputVolumeLease {
    fn drop(&mut self) {
        if let Err(error) = platform::restore_volume(&self.original) {
            kotone_core::log::log(&error);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ptr::null;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::MMDeviceEnumerator;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDevice, IMMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    };

    const RECORDING_VOLUME_RATIO: f32 = 0.3;

    pub(super) struct PlaybackVolume {
        endpoint_id: HSTRING,
        scalar: f32,
    }

    struct ComApartment(bool);

    impl ComApartment {
        fn initialize() -> Result<Self, String> {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result == RPC_E_CHANGED_MODE {
                // The caller has already initialized COM with a different apartment model.
                return Ok(Self(false));
            }
            result
                .ok()
                .map_err(|error| format!("初始化音频设备 COM 失败：{error}"))?;
            Ok(Self(true))
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, String> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| format!("无法打开音频设备列表：{error}"))
    }

    fn endpoint_volume(device: &IMMDevice) -> Result<IAudioEndpointVolume, String> {
        unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|error| format!("无法打开扬声器音量控制：{error}"))
    }

    pub(super) fn lower_default_volume() -> Result<PlaybackVolume, String> {
        let _com = ComApartment::initialize()?;
        let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole) }
            .map_err(|error| format!("未找到默认的播放设备：{error}"))?;
        let endpoint_id = unsafe {
            let id = device
                .GetId()
                .map_err(|error| format!("无法读取播放设备标识：{error}"))?;
            let endpoint_id = id.to_hstring();
            CoTaskMemFree(Some(id.0.cast()));
            endpoint_id
        };
        let volume = endpoint_volume(&device)?;
        let scalar = unsafe { volume.GetMasterVolumeLevelScalar() }
            .map_err(|error| format!("无法读取扬声器音量：{error}"))?;
        unsafe { volume.SetMasterVolumeLevelScalar(scalar * RECORDING_VOLUME_RATIO, null()) }
            .map_err(|error| format!("降低扬声器音量失败：{error}"))?;
        Ok(PlaybackVolume {
            endpoint_id,
            scalar,
        })
    }

    pub(super) fn restore_volume(original: &PlaybackVolume) -> Result<(), String> {
        let _com = ComApartment::initialize()?;
        // Restore the original device even when the default output has changed.
        let device = unsafe { enumerator()?.GetDevice(PCWSTR(original.endpoint_id.as_ptr())) }
            .map_err(|error| format!("无法找到需要恢复音量的播放设备：{error}"))?;
        let volume = endpoint_volume(&device)?;
        unsafe { volume.SetMasterVolumeLevelScalar(original.scalar, null()) }
            .map_err(|error| format!("恢复扬声器音量失败：{error}"))
    }
}
