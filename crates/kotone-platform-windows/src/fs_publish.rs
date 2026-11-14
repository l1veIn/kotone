//! 原子“不覆盖”发布：把已经写完的临时文件改名到目标路径，目标已存在时失败且不替换。
//!
//! `std::fs::rename` 在 Windows 上会替换已存在的目标（MOVEFILE_REPLACE_EXISTING），先
//! `exists()` 再 rename 有检查到使用的竞态。这里用不带 REPLACE_EXISTING 的 MoveFileExW，
//! 由文件系统在一次操作内判定目标是否存在。

use std::io;
use std::path::Path;

/// 把 `from` 改名为 `to`；`to` 已存在时返回 `ErrorKind::AlreadyExists` 且不修改任何文件。
#[cfg(windows)]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let (from_w, to_w) = (wide(from), wide(to));
    unsafe {
        MoveFileExW(
            PCWSTR(from_w.as_ptr()),
            PCWSTR(to_w.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        let code = error.code();
        if code == ERROR_ALREADY_EXISTS.to_hresult() || code == ERROR_FILE_EXISTS.to_hresult() {
            io::Error::new(io::ErrorKind::AlreadyExists, "目标文件已存在")
        } else {
            io::Error::other(format!("MoveFileExW: {error}"))
        }
    })
}

/// 非 Windows：硬链接（目标存在即失败）后删除临时名。
#[cfg(not(windows))]
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_existing_target_and_moves_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.tmp");
        let target = dir.path().join("out.png");
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        let err = rename_no_replace(&a, &target).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(a.exists(), "失败时不动源文件");
        std::fs::remove_file(&target).unwrap();
        rename_no_replace(&a, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!a.exists());
    }
}
