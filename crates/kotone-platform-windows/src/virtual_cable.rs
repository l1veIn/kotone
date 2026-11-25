//! VB-CABLE（ADR 020：她往 CABLE Input 放声音，游戏把 CABLE Output 当麦克风读）的设备名判断，
//! 和“收音时用哪个录音设备”这条规矩。没有状态：本体的收音（[`crate::audio`]、[`crate::echo_cancel`]）、
//! 游戏语音插件的转发、命令行都用这里的判断，别处不再各写一份（共享资源评估第二节第 4 条）。

/// VB-CABLE 的播放端（Kotone 往这里放声音）。
pub const CABLE_INPUT: &str = "CABLE Input";
/// VB-CABLE 的录音端（游戏选它当麦克风）。
pub const CABLE_OUTPUT: &str = "CABLE Output";

/// 设备名是不是 VB-CABLE 的某一端（只认标准 VB-CABLE，不认 A+B / C+D 等其他版本）。
pub fn is_cable(name: &str, end: &str) -> bool {
    name.starts_with(end) && name.contains("VB-Audio Virtual Cable")
}

/// 是不是 CABLE Output：她说给队友听的话、转给队友的麦克风都从这里出来。
pub fn is_cable_output(name: &str) -> bool {
    is_cable(name, CABLE_OUTPUT)
}

/// 看名字像不像虚拟声卡（VB-CABLE 各版本、VoiceMeeter、名字里带 Virtual / 虚拟的）。只用来挑真麦克风和
/// 提示，不拦玩家明确选的设备。
pub fn looks_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("cable") || n.contains("virtual") || n.contains("vb-audio") || name.contains("虚拟")
}

/// 设置里的录音设备是不是“跟随系统默认”。
pub fn follows_default(device_id: &str) -> bool {
    device_id.is_empty() || device_id == "default"
}

const NO_DEFAULT: &str = "未找到系统默认音频输入设备（请检查麦克风连接与系统权限）";

/// 收音实际用哪个录音设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputChoice {
    /// 跟随系统默认，系统默认就是它。
    SystemDefault(String),
    /// 跟随系统默认，但系统默认是 CABLE Output（`cable`），改用真麦克风 `mic`。
    InsteadOfCable { mic: String, cable: String },
    /// 玩家明确选的设备。
    Named(String),
}

impl InputChoice {
    /// 要打开的设备名。
    pub fn name(&self) -> &str {
        match self {
            Self::SystemDefault(name) | Self::Named(name) => name,
            Self::InsteadOfCable { mic, .. } => mic,
        }
    }
}

/// 选收音设备（纯逻辑）。`requested` 是设置里的设备（`default` / 空 = 跟随系统默认，其余是设备名），
/// `system_default` 是 Windows 现在的默认录音设备名，`inputs` 列出全部录音设备，最近当过默认的在前
/// （[`crate::default_order`]；只在要用时才调）。
///
/// 跟随系统默认、而系统默认是 CABLE Output 时（`voice_chat.default_mic` 为只认 Windows 默认麦克风的游戏
/// 改的），改用上一个默认——排在前面的第一个看着不像虚拟声卡的录音设备，和 Windows 自己在默认设备被拔掉
/// 时的选法一样；一个也没有就报错、说怎么办。CABLE Output 里是她说给队友的话和转过去的麦克风，听它就是听
/// 她自己（游戏语音没在转时什么也听不到）。玩家明确选的设备照用（真机脚本就是明确选 CABLE Output、往
/// CABLE Input 里放录音来测的）。
pub fn choose_input(
    requested: &str,
    system_default: Option<&str>,
    inputs: impl FnOnce() -> Result<Vec<String>, String>,
) -> Result<InputChoice, String> {
    if follows_default(requested) {
        let name = system_default.ok_or_else(|| NO_DEFAULT.to_string())?;
        if !is_cable_output(name) {
            return Ok(InputChoice::SystemDefault(name.to_string()));
        }
        let mic = inputs()?
            .into_iter()
            .find(|n| !looks_virtual(n))
            .ok_or_else(|| {
                format!(
                    "系统默认麦克风是 {name}（她说给队友听的虚拟麦克风），没找到别的麦克风：\
                     接上麦克风再试，或者在「设置 → 声音」里选你的麦克风"
                )
            })?;
        return Ok(InputChoice::InsteadOfCable {
            mic,
            cable: name.to_string(),
        });
    }
    inputs()?
        .into_iter()
        .find(|name| name == requested)
        .map(InputChoice::Named)
        .ok_or_else(|| format!("未找到音频输入设备: {requested}"))
}

/// 设备列表里“系统默认”那一项的写法：系统默认是 CABLE Output 时写明改用了哪个（设置页、诊断、
/// `audio.devices` 都显示这一项）。`inputs` 同 [`choose_input`]。
pub fn default_label(
    system_default: &str,
    inputs: impl FnOnce() -> Result<Vec<String>, String>,
) -> String {
    match choose_input("default", Some(system_default), inputs) {
        Ok(InputChoice::InsteadOfCable { mic, .. }) => {
            format!("系统默认（{CABLE_OUTPUT} 是虚拟麦克风，改用 {mic}）")
        }
        Ok(choice) => format!("系统默认（{}）", choice.name()),
        Err(_) => format!("系统默认（{CABLE_OUTPUT} 是虚拟麦克风，没找到别的麦克风）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CABLE: &str = "CABLE Output (VB-Audio Virtual Cable)";
    const USB: &str = "麦克风 (USB Audio)";

    fn devices(names: &[&str]) -> impl FnOnce() -> Result<Vec<String>, String> {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        move || Ok(names)
    }

    /// 评估第二节第 4 条：`voice_chat.default_mic` 把系统默认麦克风设成 CABLE Output 以后，
    /// 跟随系统默认的收音（呼叫、语音输入、唤醒、游戏语音的转发）不该听 CABLE Output。
    #[test]
    fn following_the_default_never_hears_cable_output() {
        let choice = choose_input(
            "default",
            Some(CABLE),
            devices(&[CABLE, "CABLE-A Output (VB-Audio Cable A)", USB]),
        )
        .unwrap();
        assert_eq!(
            choice,
            InputChoice::InsteadOfCable {
                mic: USB.into(),
                cable: CABLE.into()
            }
        );
        assert_eq!(choice.name(), USB);
        // 空字符串也是跟随系统默认
        assert_eq!(
            choose_input("", Some(CABLE), devices(&[USB, CABLE]))
                .unwrap()
                .name(),
            USB
        );
    }

    #[test]
    fn a_real_default_is_used_as_is_without_listing_devices() {
        let choice = choose_input("default", Some(USB), || -> Result<Vec<String>, String> {
            panic!("系统默认是真麦克风时不用列设备")
        });
        assert_eq!(choice, Ok(InputChoice::SystemDefault(USB.into())));
        // 别的虚拟声卡是玩家自己的安排（例如 VoiceMeeter），不是 Kotone 往里放声音的那个：照用
        let voicemeeter = "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)";
        assert_eq!(
            choose_input("default", Some(voicemeeter), devices(&[USB])).unwrap(),
            InputChoice::SystemDefault(voicemeeter.into())
        );
    }

    #[test]
    fn a_device_the_player_picked_is_used_even_if_it_is_cable_output() {
        // 真机脚本就是明确选 CABLE Output 往里放录音的
        assert_eq!(
            choose_input(CABLE, Some(USB), devices(&[USB, CABLE])).unwrap(),
            InputChoice::Named(CABLE.into())
        );
        assert_eq!(
            choose_input("麦克风 (拔掉了)", Some(USB), devices(&[USB, CABLE])),
            Err("未找到音频输入设备: 麦克风 (拔掉了)".to_string())
        );
    }

    #[test]
    fn cable_as_default_with_no_real_mic_says_what_to_do() {
        let err = choose_input(
            "default",
            Some(CABLE),
            devices(&[CABLE, "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)"]),
        )
        .unwrap_err();
        assert!(
            err.contains("CABLE Output") && err.contains("设置 → 声音"),
            "{err}"
        );
        assert!(choose_input("default", None, devices(&[USB]))
            .unwrap_err()
            .contains("未找到系统默认音频输入设备"));
    }

    #[test]
    fn the_default_entry_says_which_mic_is_really_used() {
        assert_eq!(
            default_label(USB, devices(&[CABLE, USB])),
            format!("系统默认（{USB}）")
        );
        assert_eq!(
            default_label(CABLE, devices(&[CABLE, USB])),
            format!("系统默认（CABLE Output 是虚拟麦克风，改用 {USB}）")
        );
        assert_eq!(
            default_label(CABLE, devices(&[CABLE])),
            "系统默认（CABLE Output 是虚拟麦克风，没找到别的麦克风）"
        );
    }

    /// 有好几个真麦克风时用上一个默认（`inputs` 已经按最近当过默认的排好），不是枚举出来的第一个。
    #[test]
    fn with_several_real_mics_the_previous_default_wins() {
        let by_recent = crate::default_order::by_recent_default(
            vec![
                "麦克风 (EDIFIER M30 Plus)".into(),
                "麦克风 (PD200X Podcast Microphone)".into(),
                CABLE.into(),
            ],
            &[
                (CABLE.into(), 145),
                ("麦克风 (PD200X Podcast Microphone)".into(), 144),
                ("麦克风 (EDIFIER M30 Plus)".into(), 141),
            ],
        );
        assert_eq!(
            choose_input("default", Some(CABLE), || Ok(by_recent))
                .unwrap()
                .name(),
            "麦克风 (PD200X Podcast Microphone)"
        );
    }

    #[test]
    fn only_the_standard_cable_counts_as_cable_but_all_look_virtual() {
        assert!(is_cable_output(CABLE));
        assert!(is_cable(
            "CABLE Input (VB-Audio Virtual Cable)",
            CABLE_INPUT
        ));
        assert!(!is_cable_output("CABLE-A Output (VB-Audio Cable A)"));
        assert!(!is_cable_output(USB));
        for name in [
            CABLE,
            "CABLE Input (VB-Audio Virtual Cable)",
            "CABLE-A Output (VB-Audio Cable A)",
            "VoiceMeeter Output (VB-Audio VoiceMeeter VAIO)",
            "VoiceMeeter Virtual Output",
            "虚拟声卡驱动",
        ] {
            assert!(looks_virtual(name), "{name}");
        }
        assert!(!looks_virtual("麦克风 (Realtek(R) Audio)"));
        assert!(!looks_virtual("Microphone (USB Audio Device)"));
    }
}
