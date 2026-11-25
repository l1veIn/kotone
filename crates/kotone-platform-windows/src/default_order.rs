//! Windows 记的“哪个录音设备最近当过默认”。MMDevices 注册表里每个录音设备有一个 `Level:0`（控制台角色，
//! cpal 的“系统默认”就是这个角色）：设成默认时它变成最大的，所以按它从大到小排就是“现在的默认、上一个默认、
//! 再上一个……”。默认设备被拔掉时，Windows 自己也是改用剩下里最大的那个。
//!
//! 用处：收音跟随系统默认、而系统默认被设成 CABLE Output 时，照这个先后找“上一个默认”
//! （[`crate::virtual_cable::choose_input`]）。这几项注册表没有公开文档（和切默认设备用的 `IPolicyConfig`
//! 一样），所以只拿它排先后：读不到、对不上名字的设备照枚举顺序排在后面，不报错。
//! 2026-11-25 在开发机上只读核对过：现在的默认录音设备、默认播放设备的 `Level:0` 都是最大的。

/// 按最近当过默认的先后排设备名（纯逻辑）：`levels` 里有的按 Level 从大到小；一样大的、没有的照原来的顺序，
/// 没有的排在最后。
pub fn by_recent_default(mut names: Vec<String>, levels: &[(String, u64)]) -> Vec<String> {
    let level = |name: &String| levels.iter().find(|(n, _)| n == name).map(|(_, l)| *l);
    names.sort_by_key(|name| std::cmp::Reverse(level(name)));
    names
}

#[cfg(windows)]
mod imp {
    use crate::registry::{self, HKEY_LOCAL_MACHINE};

    const CAPTURE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture";
    /// `PKEY_Device_DeviceDesc`（例如“麦克风”）。
    const DEVICE_DESC: &str = "{a45c254e-df1c-4efd-8020-67d146a850e0},2";
    /// `PKEY_DeviceInterface_FriendlyName`（例如“PD200X Podcast Microphone”）。
    const INTERFACE_NAME: &str = "{b3f8fa53-0004-438e-9003-51a46e139bfc},6";
    /// `DeviceState` 的低 4 位是状态，1 = 正在用（高位是别的标记）。
    const STATE_MASK: u64 = 0xF;
    const DEVICE_STATE_ACTIVE: u64 = 1;

    /// 本机正在用的录音设备的（名字，`Level:0`）。名字照 Windows 拼友好名的写法“设备说明 (接口名)”，和
    /// cpal 给的设备名一样（开发机上 8 个都对得上）。
    pub fn capture_levels() -> Vec<(String, u64)> {
        registry::subkeys(HKEY_LOCAL_MACHINE, CAPTURE)
            .into_iter()
            .filter_map(|guid| {
                let key = format!(r"{CAPTURE}\{guid}");
                let state = registry::number(HKEY_LOCAL_MACHINE, &key, "DeviceState")?;
                if state & STATE_MASK != DEVICE_STATE_ACTIVE {
                    return None;
                }
                let level = registry::number(HKEY_LOCAL_MACHINE, &key, "Level:0")?;
                let properties = format!(r"{key}\Properties");
                let desc = registry::string(HKEY_LOCAL_MACHINE, &properties, DEVICE_DESC)?;
                let name = registry::string(HKEY_LOCAL_MACHINE, &properties, INTERFACE_NAME)?;
                Some((format!("{desc} ({name})"), level))
            })
            .collect()
    }
}

/// 本机正在用的录音设备的（名字，`Level:0`）；读不到时是空的。
#[cfg(windows)]
pub use imp::capture_levels;

#[cfg(not(windows))]
pub fn capture_levels() -> Vec<(String, u64)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    /// 开发机的样子（2026-11-25 只读看到的）：默认是 PD200X（Level 144），枚举顺序里 EDIFIER 在前。
    /// `voice_chat.default_mic` 把默认切成 CABLE Output 以后，CABLE 变成最大的（145）。
    #[test]
    fn the_previous_default_comes_right_after_the_current_one() {
        let enumerated = names(&[
            "麦克风 (EDIFIER M30 Plus)",
            "麦克风 (PD200X Podcast Microphone)",
            "Headset Microphone (Oculus Virtual Audio Device)",
            "CABLE Output (VB-Audio Virtual Cable)",
            "麦克风 (Maono AI Microphone)",
            "麦克风 (没有记录的设备)",
        ]);
        let levels = [
            ("CABLE Output (VB-Audio Virtual Cable)".to_string(), 145),
            ("麦克风 (PD200X Podcast Microphone)".to_string(), 144),
            ("麦克风 (EDIFIER M30 Plus)".to_string(), 141),
            ("麦克风 (Maono AI Microphone)".to_string(), 141),
            (
                "Headset Microphone (Oculus Virtual Audio Device)".to_string(),
                140,
            ),
            ("麦克风 (已经拔掉的)".to_string(), 999),
        ];
        assert_eq!(
            by_recent_default(enumerated.clone(), &levels),
            names(&[
                "CABLE Output (VB-Audio Virtual Cable)",
                "麦克风 (PD200X Podcast Microphone)",
                "麦克风 (EDIFIER M30 Plus)",
                "麦克风 (Maono AI Microphone)",
                "Headset Microphone (Oculus Virtual Audio Device)",
                "麦克风 (没有记录的设备)",
            ])
        );
        // 读不到注册表：照枚举顺序
        assert_eq!(by_recent_default(enumerated.clone(), &[]), enumerated);
    }
}
