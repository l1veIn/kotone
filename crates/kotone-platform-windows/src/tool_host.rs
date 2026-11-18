//! 工具服务实例发现的 Windows 宿主：会话 ID、进程身份与仅当前用户可访问的文件。
//!
//! 私有目录/文件使用受保护（不继承）的 DACL，只授予当前用户 SID 与 SYSTEM 完全控制。
//! 读取端校验文件所有者为当前用户（或提权进程写入时的 Administrators），并拒绝任何向
//! 其他账户授予访问的 ACE，避免读取被他人篡改或可被他人读取的凭证文件。

use std::io;
use std::path::Path;

use kotone_core::tools::ToolHost;

/// Windows 实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsToolHost;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::io::Write as _;
    use std::os::windows::ffi::OsStrExt;

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        AclSizeInformation, GetAce, GetAclInformation, GetSecurityDescriptorDacl,
        GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcessToken,
    };

    const SID_SYSTEM: &str = "S-1-5-18";
    const SID_ADMINISTRATORS: &str = "S-1-5-32-544";
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn os_error(context: &str, error: impl std::fmt::Display) -> io::Error {
        io::Error::other(format!("{context}: {error}"))
    }

    unsafe fn sid_to_string(sid: PSID) -> io::Result<String> {
        let mut out = PWSTR::null();
        ConvertSidToStringSidW(sid, &mut out).map_err(|e| os_error("ConvertSidToStringSidW", e))?;
        let text = out.to_string().map_err(|e| os_error("SID 字符串", e));
        let _ = LocalFree(Some(HLOCAL(out.0 as *mut c_void)));
        text
    }

    /// 当前进程 token 的用户 SID（字符串形式）。
    pub fn current_user_sid() -> io::Result<String> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
                .map_err(|e| os_error("OpenProcessToken", e))?;
            let result = (|| {
                let mut len = 0u32;
                let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
                if len == 0 {
                    return Err(io::Error::other("TokenUser 长度为 0"));
                }
                // u64 对齐的缓冲区，满足 TOKEN_USER 的对齐要求
                let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                GetTokenInformation(
                    token,
                    TokenUser,
                    Some(buf.as_mut_ptr() as *mut c_void),
                    len,
                    &mut len,
                )
                .map_err(|e| os_error("GetTokenInformation", e))?;
                let user = &*(buf.as_ptr() as *const TOKEN_USER);
                sid_to_string(user.User.Sid)
            })();
            let _ = CloseHandle(token);
            result
        }
    }

    /// 对路径应用受保护的 SDDL DACL。
    fn apply_protected_dacl(path: &Path, sddl: &str) -> io::Result<()> {
        unsafe {
            let sddl = wide_str(sddl);
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(|e| os_error("ConvertStringSecurityDescriptor", e))?;
            let result = (|| {
                let mut present = windows::core::BOOL(0);
                let mut defaulted = windows::core::BOOL(0);
                let mut dacl: *mut ACL = std::ptr::null_mut();
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
                    .map_err(|e| os_error("GetSecurityDescriptorDacl", e))?;
                if !present.as_bool() || dacl.is_null() {
                    return Err(io::Error::other("SDDL 未产生 DACL"));
                }
                let path = wide(path);
                let status = SetNamedSecurityInfoW(
                    PCWSTR(path.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(dacl as *const ACL),
                    None,
                );
                if status.is_err() {
                    return Err(io::Error::from_raw_os_error(status.0 as i32));
                }
                Ok(())
            })();
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            result
        }
    }

    pub fn session_id() -> Option<u32> {
        let mut session = 0u32;
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session).ok()? };
        Some(session)
    }

    pub fn prepare_private_dir(dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let user = current_user_sid()?;
        apply_protected_dacl(dir, &format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)"))
    }

    pub fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("私有文件缺少父目录"))?;
        let user = current_user_sid()?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or_default();
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            std::process::id(),
            nonce
        ));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            // 先收紧 ACL 再写入内容
            apply_protected_dacl(&temp, &format!("D:P(A;;FA;;;{user})(A;;FA;;;SY)"))?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }

    pub fn verify_private_file(path: &Path) -> io::Result<()> {
        let user = current_user_sid()?;
        unsafe {
            let wide_path = wide(path);
            let mut owner = PSID::default();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            let status = GetNamedSecurityInfoW(
                PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                Some(&mut dacl),
                None,
                &mut descriptor,
            );
            if status.is_err() {
                return Err(io::Error::from_raw_os_error(status.0 as i32));
            }
            let result = (|| {
                let owner_sid = sid_to_string(owner)?;
                if owner_sid != user && owner_sid != SID_ADMINISTRATORS {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "描述文件不属于当前用户",
                    ));
                }
                if dacl.is_null() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "描述文件没有 DACL（任何人可访问）",
                    ));
                }
                let mut size = ACL_SIZE_INFORMATION::default();
                GetAclInformation(
                    dacl,
                    &mut size as *mut ACL_SIZE_INFORMATION as *mut c_void,
                    std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                )
                .map_err(|e| os_error("GetAclInformation", e))?;
                for index in 0..size.AceCount {
                    let mut ace: *mut c_void = std::ptr::null_mut();
                    GetAce(dacl, index, &mut ace).map_err(|e| os_error("GetAce", e))?;
                    let header = &*(ace as *const ACE_HEADER);
                    match header.AceType {
                        ACCESS_DENIED_ACE_TYPE => continue,
                        ACCESS_ALLOWED_ACE_TYPE => {
                            let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
                            let sid = PSID(&allowed.SidStart as *const u32 as *mut c_void);
                            let sid = sid_to_string(sid)?;
                            if sid != user && sid != SID_SYSTEM && sid != SID_ADMINISTRATORS {
                                return Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    format!("描述文件向其他账户授予了访问权限（{sid}）"),
                                ));
                            }
                        }
                        other => {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                format!("描述文件包含无法核实的 ACE 类型 {other}"),
                            ))
                        }
                    }
                }
                Ok(())
            })();
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            result
        }
    }
}

#[cfg(windows)]
pub use imp::current_user_sid;

#[cfg(windows)]
impl ToolHost for WindowsToolHost {
    fn session_id(&self) -> Option<u32> {
        imp::session_id()
    }
    fn process_started_at(&self, pid: u32) -> Option<u64> {
        crate::window_tools::process_started_at(pid)
    }
    fn prepare_private_dir(&self, dir: &Path) -> io::Result<()> {
        imp::prepare_private_dir(dir)
    }
    fn write_private_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        imp::write_private_file(path, bytes)
    }
    fn verify_private_file(&self, path: &Path) -> io::Result<()> {
        imp::verify_private_file(path)
    }
}

#[cfg(not(windows))]
impl ToolHost for WindowsToolHost {
    fn session_id(&self) -> Option<u32> {
        None
    }
    fn process_started_at(&self, _pid: u32) -> Option<u64> {
        None
    }
    fn prepare_private_dir(&self, _dir: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "仅支持 Windows"))
    }
    fn write_private_file(&self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "仅支持 Windows"))
    }
    fn verify_private_file(&self, _path: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "仅支持 Windows"))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn private_file_round_trip_and_acl_check() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        let host = WindowsToolHost;
        host.prepare_private_dir(&run).unwrap();
        let file = run.join("descriptor.json");
        host.write_private_file(&file, b"{\"a\":1}").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"{\"a\":1}");
        host.verify_private_file(&file).unwrap();
        // 覆盖写入保持私有
        host.write_private_file(&file, b"{\"a\":2}").unwrap();
        host.verify_private_file(&file).unwrap();
        // 没有遗留临时文件
        let leftovers: Vec<_> = std::fs::read_dir(&run)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn file_readable_by_everyone_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("open.json");
        std::fs::write(&file, b"{}").unwrap();
        imp_apply_everyone(&file);
        let error = WindowsToolHost.verify_private_file(&file).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    }

    fn imp_apply_everyone(path: &Path) {
        // 通过 icacls 授予 Everyone 读取，模拟被放宽权限的描述文件
        let status = std::process::Command::new("icacls")
            .arg(path)
            .args(["/grant", "*S-1-1-0:R"])
            .output()
            .unwrap();
        assert!(status.status.success(), "icacls 失败: {status:?}");
    }

    #[test]
    fn session_and_process_identity_are_available() {
        let host = WindowsToolHost;
        assert!(host.session_id().is_some());
        let started = host.process_started_at(std::process::id());
        assert!(started.is_some());
        assert_eq!(started, host.process_started_at(std::process::id()));
        assert_eq!(host.process_started_at(0), None);
        assert!(current_user_sid().unwrap().starts_with("S-1-5-"));
    }
}
