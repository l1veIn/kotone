//! 读注册表（只读）：游戏库扫描（`games.rs`）和录音设备当默认的先后（`default_order.rs`）共用。
//! 64 位、32 位两个视图都试。

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, KEY_READ, KEY_WOW64_32KEY,
    KEY_WOW64_64KEY, REG_DWORD, REG_EXPAND_SZ, REG_QWORD, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};

pub use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(root: HKEY, path: &str, view: REG_SAM_FLAGS) -> Option<Key> {
    let mut key = HKEY::default();
    let path = wide(path);
    let ok = unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), None, KEY_READ | view, &mut key) };
    (ok.0 == 0).then_some(Key(key))
}

fn views() -> [REG_SAM_FLAGS; 2] {
    [KEY_WOW64_64KEY, KEY_WOW64_32KEY]
}

fn query(key: &Key, value: &str) -> Option<String> {
    let name = wide(value);
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    unsafe {
        if RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
        .0 != 0
        {
            return None;
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut bytes = (buf.len() * 2) as u32;
        if RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .0 != 0
        {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// 读字符串值（64 位、32 位两个视图都试）。
pub fn string(root: HKEY, path: &str, value: &str) -> Option<String> {
    views()
        .into_iter()
        .find_map(|view| open(root, path, view).and_then(|key| query(&key, value)))
}

/// 子键名（两个视图合并去重）。
pub fn subkeys(root: HKEY, path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for view in views() {
        let Some(key) = open(root, path, view) else {
            continue;
        };
        for index in 0.. {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            let ok = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    Some(PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if ok.0 != 0 {
                break;
            }
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn query_number(key: &Key, value: &str) -> Option<u64> {
    let name = wide(value);
    let mut kind = REG_VALUE_TYPE::default();
    let mut buf = [0u8; 8];
    let mut size = buf.len() as u32;
    let ok = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if ok.0 != 0 {
        return None;
    }
    match (kind, size) {
        (REG_DWORD, 4) => Some(u64::from(u32::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3],
        ]))),
        (REG_QWORD, 8) => Some(u64::from_le_bytes(buf)),
        _ => None,
    }
}

/// 读数值（REG_DWORD 或 REG_QWORD）。
pub fn number(root: HKEY, path: &str, value: &str) -> Option<u64> {
    views()
        .into_iter()
        .find_map(|view| open(root, path, view).and_then(|key| query_number(&key, value)))
}
