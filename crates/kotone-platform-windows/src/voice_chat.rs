//! 游戏语音的系统一侧（ADR 020）：虚拟麦克风 VB-CABLE 的检测与静默安装、Windows 默认录音设备的
//! 读取与切换。转发与混音在 [`crate::voice_router`]。
//!
//! - 检测：cpal 枚举设备名（VB-CABLE 装好后有播放设备“CABLE Input (VB-Audio Virtual Cable)”与
//!   录音设备“CABLE Output (VB-Audio Virtual Cable)”）；
//! - 安装：照 Soundux 的安装脚本，以管理员身份运行官方 `VBCABLE_Setup_x64.exe -h -i` 并等它结束
//!   （下载与校验在桌面端，复用模型下载）；
//! - 默认录音设备：MMDevice API 列出录音设备；切换用 `IPolicyConfig::SetDefaultEndpoint`
//!   （未公开的 COM 接口，AudioDeviceCmdlets、EarTrumpet、SoundSwitch 都用它），三个角色
//!   （控制台、多媒体、通信）一起设。

/// VB-CABLE 的播放端（Kotone 往这里放声音）。
pub const CABLE_INPUT: &str = "CABLE Input";
/// VB-CABLE 的录音端（游戏选它当麦克风）。
pub const CABLE_OUTPUT: &str = "CABLE Output";

/// 设备名是不是 VB-CABLE 的某一端（只认标准 VB-CABLE，不认 A+B / C+D 等其他版本）。
pub fn is_cable(name: &str, end: &str) -> bool {
    name.starts_with(end) && name.contains("VB-Audio Virtual Cable")
}

/// 两端的设备名（没装时为 None）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CableDevices {
    pub input: Option<String>,
    pub output: Option<String>,
}

impl CableDevices {
    pub fn installed(&self) -> bool {
        self.input.is_some() && self.output.is_some()
    }
}

/// 从设备名清单里找出 VB-CABLE 的两端（纯逻辑，便于测试）。
pub fn find_cable(outputs: &[String], inputs: &[String]) -> CableDevices {
    CableDevices {
        input: outputs.iter().find(|n| is_cable(n, CABLE_INPUT)).cloned(),
        output: inputs.iter().find(|n| is_cable(n, CABLE_OUTPUT)).cloned(),
    }
}

/// 检测 VB-CABLE（按 cpal 的设备名）。
pub fn detect() -> CableDevices {
    let names = |list: Vec<kotone_core::audio::AudioDevice>| -> Vec<String> {
        list.into_iter()
            .filter(|d| d.id != "default")
            .map(|d| d.name)
            .collect()
    };
    find_cable(
        &names(crate::audio::list_output_devices()),
        &names(crate::audio::list_devices()),
    )
}

/// 一个录音设备（MMDevice 的 ID 与显示名）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureEndpoint {
    pub id: String,
    pub name: String,
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::time::Duration;

    use windows::core::{IUnknown, Interface, GUID, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_CANCELLED, HWND, WAIT_OBJECT_0,
    };
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eConsole, eMultimedia, ERole, IMMDevice, IMMDeviceEnumerator,
        MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED, STGM_READ,
    };
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    use super::CaptureEndpoint;

    /// `PKEY_Device_FriendlyName`。
    const FRIENDLY_NAME: windows::Win32::Foundation::PROPERTYKEY =
        windows::Win32::Foundation::PROPERTYKEY {
            fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
            pid: 14,
        };

    /// `CPolicyConfigClient`。
    const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

    /// `IPolicyConfig`（Windows 7 起，未公开）的 IID。方法顺序照 AudioDeviceCmdlets / EarTrumpet
    /// 的声明：IUnknown 的 3 个之后依次是 GetMixFormat、GetDeviceFormat、ResetDeviceFormat、
    /// SetDeviceFormat、GetProcessingPeriod、SetProcessingPeriod、GetShareMode、SetShareMode、
    /// GetPropertyValue、SetPropertyValue、**SetDefaultEndpoint**、SetEndpointVisibility。
    const IID_POLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    /// `SetDefaultEndpoint` 在虚表里的位置。
    const SET_DEFAULT_ENDPOINT_SLOT: usize = 3 + 10;
    type SetDefaultEndpoint = unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT;

    /// 在独立线程里初始化 COM（多线程套间）再做事，不影响调用方线程的套间。
    fn with_com<T: Send + 'static>(
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        std::thread::spawn(move || unsafe {
            let inited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let result = work();
            if inited {
                CoUninitialize();
            }
            result
        })
        .join()
        .map_err(|_| "音频设备线程异常".to_string())?
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, String> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| format!("无法打开音频设备列表：{e}"))
    }

    fn endpoint(device: &IMMDevice) -> Result<CaptureEndpoint, String> {
        unsafe {
            let raw = device.GetId().map_err(|e| e.to_string())?;
            let id = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as *const c_void));
            let store = device
                .OpenPropertyStore(STGM_READ)
                .map_err(|e| e.to_string())?;
            let value = store.GetValue(&FRIENDLY_NAME).map_err(|e| e.to_string())?;
            let name = match PropVariantToStringAlloc(&value) {
                Ok(text) => {
                    let s = text.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(text.0 as *const c_void));
                    s
                }
                Err(_) => String::new(),
            };
            Ok(CaptureEndpoint { id, name })
        }
    }

    pub fn capture_endpoints() -> Result<Vec<CaptureEndpoint>, String> {
        with_com(|| unsafe {
            let devices = enumerator()?
                .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
                .map_err(|e| format!("无法列出录音设备：{e}"))?;
            let count = devices.GetCount().map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for i in 0..count {
                if let Ok(device) = devices.Item(i) {
                    out.push(endpoint(&device)?);
                }
            }
            Ok(out)
        })
    }

    /// 当前默认录音设备（控制台角色）与默认通信录音设备。
    pub fn default_capture() -> Result<(Option<CaptureEndpoint>, Option<CaptureEndpoint>), String> {
        with_com(|| unsafe {
            let e = enumerator()?;
            let console = e
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .ok()
                .map(|d| endpoint(&d))
                .transpose()?;
            let communications = e
                .GetDefaultAudioEndpoint(eCapture, eCommunications)
                .ok()
                .map(|d| endpoint(&d))
                .transpose()?;
            Ok((console, communications))
        })
    }

    /// 把这个录音设备设成默认（控制台、多媒体）；`communications` 为 true 时也设成默认通信设备。
    pub fn set_default_capture(id: &str, communications: bool) -> Result<(), String> {
        let id = id.to_string();
        with_com(move || unsafe {
            let client: IUnknown = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
                .map_err(|e| format!("无法修改默认设备（IPolicyConfig 不可用）：{e}"))?;
            let mut raw: *mut c_void = std::ptr::null_mut();
            client
                .query(&IID_POLICY_CONFIG, &mut raw)
                .ok()
                .map_err(|e| format!("无法修改默认设备（IPolicyConfig 不可用）：{e}"))?;
            // 这个引用交给 IUnknown 管，离开作用域时 Release。
            let policy = IUnknown::from_raw(raw);
            let vtable = *(raw as *const *const *const c_void);
            let set_default: SetDefaultEndpoint =
                std::mem::transmute(*vtable.add(SET_DEFAULT_ENDPOINT_SLOT));
            let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
            let mut roles = vec![eConsole, eMultimedia];
            if communications {
                roles.push(eCommunications);
            }
            for role in roles {
                set_default(raw, PCWSTR(wide.as_ptr()), role)
                    .ok()
                    .map_err(|e| format!("设置默认录音设备失败：{e}"))?;
            }
            drop(policy);
            Ok(())
        })
    }

    /// 以管理员身份运行一个程序并等它结束（Windows 弹一次授权）；返回退出码。
    pub fn run_elevated_and_wait(
        exe: &str,
        params: &str,
        timeout: Duration,
    ) -> Result<u32, String> {
        let exe_w: Vec<u16> = exe.encode_utf16().chain([0]).collect();
        let verb_w: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
        let params_w: Vec<u16> = params.encode_utf16().chain([0]).collect();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb_w.as_ptr()),
            lpFile: PCWSTR(exe_w.as_ptr()),
            lpParameters: PCWSTR(params_w.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        unsafe {
            if ShellExecuteExW(&mut info).is_err() {
                let err = GetLastError();
                return Err(if err == ERROR_CANCELLED {
                    "你没有允许管理员权限，没有安装".into()
                } else {
                    format!("无法以管理员身份运行安装程序：{err:?}")
                });
            }
            let process = info.hProcess;
            if process.is_invalid() {
                return Err("安装程序没有启动".into());
            }
            let wait =
                WaitForSingleObject(process, timeout.as_millis().min(u32::MAX as u128) as u32);
            let mut code = 0u32;
            let finished = wait == WAIT_OBJECT_0 && GetExitCodeProcess(process, &mut code).is_ok();
            let _ = CloseHandle(process);
            if finished {
                Ok(code)
            } else {
                Err("安装程序长时间没有结束".into())
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::CaptureEndpoint;
    use std::time::Duration;

    pub fn capture_endpoints() -> Result<Vec<CaptureEndpoint>, String> {
        Err("只支持 Windows".into())
    }
    pub fn default_capture() -> Result<(Option<CaptureEndpoint>, Option<CaptureEndpoint>), String> {
        Err("只支持 Windows".into())
    }
    pub fn set_default_capture(_id: &str, _communications: bool) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
    pub fn run_elevated_and_wait(
        _exe: &str,
        _params: &str,
        _timeout: Duration,
    ) -> Result<u32, String> {
        Err("只支持 Windows".into())
    }
}

pub use imp::{capture_endpoints, default_capture, run_elevated_and_wait, set_default_capture};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_both_ends_of_the_standard_cable_only() {
        let outputs = vec![
            "扬声器 (Realtek(R) Audio)".to_string(),
            "CABLE-A Input (VB-Audio Cable A)".to_string(),
            "CABLE Input (VB-Audio Virtual Cable)".to_string(),
        ];
        let inputs = vec![
            "麦克风 (USB Audio)".to_string(),
            "CABLE Output (VB-Audio Virtual Cable)".to_string(),
        ];
        let found = find_cable(&outputs, &inputs);
        assert_eq!(
            found.input.as_deref(),
            Some("CABLE Input (VB-Audio Virtual Cable)")
        );
        assert_eq!(
            found.output.as_deref(),
            Some("CABLE Output (VB-Audio Virtual Cable)")
        );
        assert!(found.installed());
        assert!(!find_cable(&outputs[..2], &inputs).installed());
    }

    #[test]
    #[ignore = "reads this machine's audio devices"]
    fn lists_capture_endpoints_and_defaults() {
        let list = capture_endpoints().unwrap();
        assert!(!list.is_empty());
        println!("{list:#?}\n{:#?}", default_capture().unwrap());
    }
}
