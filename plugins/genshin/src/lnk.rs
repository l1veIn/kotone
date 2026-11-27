//! 读 Windows 快捷方式（`.lnk`）指向的本机路径：只读 LinkInfo 里的 LocalBasePath（+ CommonPathSuffix）。
//!
//! 照微软公开的格式说明 [MS-SHLLINK] 2.1 ShellLinkHeader、2.3 LinkInfo
//! （<https://learn.microsoft.com/openspecs/windows_protocols/ms-shllink/>）自己写的，不调 COM（`IShellLink`）：
//! 找 BetterGI 只要路径，不该为它在插件里起 COM。只认本机路径；路径不是 ASCII 又没有 Unicode 那一份时读不出（返回 None）。

const HEADER_SIZE: usize = 0x4c;
const HAS_LINK_TARGET_ID_LIST: u32 = 0x1;
const HAS_LINK_INFO: u32 = 0x2;
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 0x1;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// 以 0 结尾的单字节串（只收 ASCII：系统代码页不一定是什么）。
fn ansi_at(bytes: &[u8], at: usize) -> Option<String> {
    let rest = bytes.get(at..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    let text = &rest[..end];
    text.is_ascii()
        .then(|| String::from_utf8_lossy(text).into_owned())
}

/// 以 0 结尾的 UTF-16 串。
fn wide_at(bytes: &[u8], at: usize) -> Option<String> {
    let rest = bytes.get(at..)?;
    let units: Vec<u16> = rest
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    (units.len() * 2 < rest.len()).then(|| String::from_utf16_lossy(&units))
}

/// 快捷方式指向的本机路径；不是快捷方式、指向网络路径或读不出时 None。
pub fn target(bytes: &[u8]) -> Option<String> {
    if u32_at(bytes, 0)? as usize != HEADER_SIZE {
        return None;
    }
    let flags = u32_at(bytes, 0x14)?;
    let mut at = HEADER_SIZE;
    if flags & HAS_LINK_TARGET_ID_LIST != 0 {
        at += 2 + u16_at(bytes, at)? as usize;
    }
    if flags & HAS_LINK_INFO == 0 {
        return None;
    }
    let info = at;
    let info_size = u32_at(bytes, info)? as usize;
    let header_size = u32_at(bytes, info + 4)? as usize;
    let info_flags = u32_at(bytes, info + 8)?;
    if info_flags & VOLUME_ID_AND_LOCAL_BASE_PATH == 0 || info_size < header_size {
        return None;
    }
    let base_offset = u32_at(bytes, info + 16)? as usize;
    let suffix_offset = u32_at(bytes, info + 24)? as usize;
    let wide = (header_size >= 0x24)
        .then(|| {
            let base = u32_at(bytes, info + 28)? as usize;
            let suffix = u32_at(bytes, info + 32)? as usize;
            let mut path = wide_at(bytes, info + base)?;
            if suffix != 0 {
                path.push_str(&wide_at(bytes, info + suffix)?);
            }
            Some(path)
        })
        .flatten();
    let path = match wide {
        Some(path) => path,
        None => {
            let mut path = ansi_at(bytes, info + base_offset)?;
            if suffix_offset != 0 {
                path.push_str(&ansi_at(bytes, info + suffix_offset)?);
            }
            path
        }
    };
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 合成一个最小的快捷方式：头 + 一段假的 IDList + LinkInfo（只有 VolumeID 占位和 LocalBasePath）。
    pub(crate) fn make_lnk(path: &str, unicode: bool) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_SIZE];
        out[0..4].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        out[0x14..0x18].copy_from_slice(&(HAS_LINK_TARGET_ID_LIST | HAS_LINK_INFO).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&[1, 2, 3, 4]);
        let header: u32 = if unicode { 0x24 } else { 0x1c };
        let volume = vec![0x10u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0];
        // 单字节那一份照写 UTF-8 的字节（不是 ASCII 时就像别的代码页的字节）
        let ansi: Vec<u8> = path.bytes().chain([0]).collect();
        let wide: Vec<u8> = path
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        let volume_at = header as usize;
        let base_at = volume_at + volume.len();
        let suffix_at = base_at + ansi.len();
        let wide_at = suffix_at + 1;
        let wide_suffix_at = wide_at + wide.len();
        let size = wide_suffix_at + 2;
        let mut info = Vec::new();
        for v in [
            size as u32,
            header,
            VOLUME_ID_AND_LOCAL_BASE_PATH,
            volume_at as u32,
            base_at as u32,
            0,
            suffix_at as u32,
        ] {
            info.extend_from_slice(&v.to_le_bytes());
        }
        if unicode {
            info.extend_from_slice(&(wide_at as u32).to_le_bytes());
            info.extend_from_slice(&(wide_suffix_at as u32).to_le_bytes());
        }
        info.extend_from_slice(&volume);
        info.extend_from_slice(&ansi);
        info.push(0);
        info.extend_from_slice(&wide);
        info.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&info);
        out
    }

    #[test]
    fn reads_the_local_base_path() {
        assert_eq!(
            target(&make_lnk("D:\\BetterGI\\BetterGI.exe", false)).as_deref(),
            Some("D:\\BetterGI\\BetterGI.exe")
        );
        assert_eq!(
            target(&make_lnk("D:\\游戏\\BetterGI\\BetterGI.exe", true)).as_deref(),
            Some("D:\\游戏\\BetterGI\\BetterGI.exe"),
            "有 Unicode 那一份时用它"
        );
        // 不是 ASCII 又没有 Unicode 那一份：读不出（不猜代码页）
        assert!(target(&make_lnk("D:\\游戏\\a.exe", false)).is_none());
    }

    #[test]
    fn garbage_is_not_a_shortcut() {
        assert!(target(b"").is_none());
        assert!(target(&[0u8; 200]).is_none());
        let mut no_info = make_lnk("C:\\x.exe", false);
        no_info[0x14] = HAS_LINK_TARGET_ID_LIST as u8;
        assert!(target(&no_info).is_none());
        let truncated = &make_lnk("C:\\x.exe", true)[..HEADER_SIZE + 10];
        assert!(target(truncated).is_none());
    }
}
