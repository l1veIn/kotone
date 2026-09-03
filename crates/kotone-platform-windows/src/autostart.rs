//! 开机自启（HKCU Run 键）。
//!
//! 写入当前用户启动项，值名与 NSIS 卸载清理保持一致：
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Kotone`
//! （见 apps/desktop/src-tauri/installer/kotone-installer.nsi 的 Uninstall 段）。
//!
//! 状态以注册表真实为准：用户在任务管理器「启动」里禁用后，读取会返回 false，
//! 设置界面据此与系统保持同步。

/// HKCU Run 键路径
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// 启动项值名 —— 必须与 NSIS 卸载清理（DeleteRegValue HKCU ...\Run "${PRODUCTNAME}"）一致
const RUN_VALUE_NAME: &str = "Kotone";

/// 退化为 utf-16 NUL 结尾字符串（windows-rs 通调用法）
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

/// 当前用户开机自启是否开启；任何一步读取失败按关闭处理（保守，不影响设置页）。
///
/// 成功（ERROR_SUCCESS = 0）即认为值存在 = 已开启。
#[cfg(windows)]
pub fn autostart_enabled() -> bool {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
    };

    unsafe {
        let mut hkey = HKEY::default();
        let opened = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(wide(RUN_KEY).as_ptr()),
            None,
            KEY_QUERY_VALUE,
            &mut hkey,
        );
        if opened.0 != 0 {
            return false;
        }
        let present = RegQueryValueExW(
            hkey,
            windows::core::PCWSTR(wide(RUN_VALUE_NAME).as_ptr()),
            None,
            None,
            None,
            None,
        )
        .0 == 0;
        let _ = RegCloseKey(hkey);
        present
    }
}

/// 写入 / 移除开机自启。
///
/// 开启：`<当前 exe 绝对路径（带引号）>`，无额外参数；exe 路径取运行中的当前
/// 可执行文件，安装版与便携版同一套逻辑，也保证开发者构建指向 dev exe。
/// 关闭：删除值；值已不存在（ERROR_FILE_NOT_FOUND）= 已经是关闭状态，也视为成功。
#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_CREATE_KEY_DISPOSITION, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    unsafe {
        let mut hkey = HKEY::default();
        let mut disposition = REG_CREATE_KEY_DISPOSITION(0);
        let opened = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(wide(RUN_KEY).as_ptr()),
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            Some(&mut disposition),
        );
        if opened.0 != 0 {
            return Err(format!(
                "打开启动项注册表失败: win32 error 0x{:08X}",
                opened.0
            ));
        }

        let result = if enabled {
            let exe = std::env::current_exe()
                .map_err(|error| format!("获取自身 exe 路径失败: {error}"))?;
            let command = format!("\"{}\"", exe.to_string_lossy());
            let data = wide(&command);
            // REG_SZ 数据按字节写入（含结尾 NUL）：内存布局与 utf16 切片一致
            let data_bytes: &[u8] =
                std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            let value = windows::core::PCWSTR(wide(RUN_VALUE_NAME).as_ptr());
            RegSetValueExW(hkey, value, None, REG_SZ, Some(data_bytes))
        } else {
            let value = windows::core::PCWSTR(wide(RUN_VALUE_NAME).as_ptr());
            RegDeleteValueW(hkey, value)
        };
        let _ = RegCloseKey(hkey);

        if result.0 == 0 || result.0 == ERROR_FILE_NOT_FOUND.0 {
            Ok(())
        } else {
            Err(format!(
                "更新开机启动项失败: win32 error 0x{:08X}",
                result.0
            ))
        }
    }
}

#[cfg(not(windows))]
pub fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err("开机自启仅 Windows 支持（Kotone MVP Windows-first）".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// 真实注册表往返（会短暂写入本机 Run 键，写完还原原状态）。
    /// 手动执行：`cargo test -p kotone-platform-windows -- --ignored autostart`
    #[test]
    #[ignore = "真实注册表往返：需要本机授权写入 HKCU Run 键"]
    fn real_registry_round_trip() {
        let original = autostart_enabled();
        set_autostart(true).expect("开启自启失败");
        assert!(autostart_enabled(), "写入后应已开启");
        set_autostart(false).expect("关闭自启失败");
        assert!(!autostart_enabled(), "删除后应已关闭");
        if original {
            set_autostart(true).expect("还原失败");
        }
    }
}
