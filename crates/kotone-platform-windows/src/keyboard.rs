//! 导出诊断包时的修饰键 / CapsLock 快照。只记是否按下或锁定，不记普通字符键。

#[derive(Debug, Clone, Default)]
pub struct ModifierSnapshot {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
    pub caps_lock_down: bool,
    pub caps_lock_toggled: bool,
}

#[cfg(windows)]
pub fn modifier_snapshot() -> ModifierSnapshot {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    fn down(vk: i32) -> bool {
        unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
    }
    fn toggled(vk: i32) -> bool {
        unsafe { GetAsyncKeyState(vk) as u16 & 1 != 0 }
    }

    ModifierSnapshot {
        shift: down(0x10),
        ctrl: down(0x11),
        alt: down(0x12),
        win: down(0x5B) || down(0x5C),
        caps_lock_down: down(0x14),
        caps_lock_toggled: toggled(0x14),
    }
}

#[cfg(not(windows))]
pub fn modifier_snapshot() -> ModifierSnapshot {
    ModifierSnapshot::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_snapshot_is_all_false() {
        let snap = ModifierSnapshot::default();
        assert!(!snap.shift && !snap.ctrl && !snap.alt && !snap.win);
        assert!(!snap.caps_lock_down && !snap.caps_lock_toggled);
    }
}
