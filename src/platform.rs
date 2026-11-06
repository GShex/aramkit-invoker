use std::path::{Path, PathBuf};

use crate::champion::ARAMKIT_CHAMPION_BASE;

/// Opens a champion page in the default browser.
///
/// Only ARAMKit champion URLs are accepted so that a malformed champion asset
/// cannot turn this into an arbitrary URL opener.
pub fn open_url(url: &str) -> Result<(), String> {
    if !is_allowed_champion_url(url) {
        return Err("拒绝打开非 ARAMKit 英雄页面链接".to_owned());
    }

    webbrowser::open(url)
        .map(|_| ())
        .map_err(|err| format!("无法打开浏览器：{err}"))
}

fn is_allowed_champion_url(url: &str) -> bool {
    url.strip_prefix(ARAMKIT_CHAMPION_BASE)
        .map(|slug| {
            !slug.is_empty()
                && slug
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        })
        .unwrap_or(false)
}

/// Validates the configured ARAMKit desktop path.
///
/// The path must be an absolute local path. On Windows it must name an `.exe`
/// or `.lnk` file. UNC paths are rejected because they can trigger outbound
/// SMB authentication and shell associations for arbitrary file types.
pub fn validate_app_path(path: &Path, require_exists: bool) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("尚未配置 ARAMKit 桌面端路径".to_owned());
    }
    if !path.is_absolute() {
        return Err("请使用绝对路径".to_owned());
    }

    let text = path.to_string_lossy();
    if text.starts_with(r"\\") || text.starts_with("//") {
        return Err("不支持网络路径".to_owned());
    }

    #[cfg(windows)]
    {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        if !extension.eq_ignore_ascii_case("exe") && !extension.eq_ignore_ascii_case("lnk") {
            return Err("仅支持 .exe 或 .lnk 文件".to_owned());
        }
    }

    if require_exists && !path.is_file() {
        return Err(format!("路径不存在或不是文件：{}", path.display()));
    }

    Ok(())
}

/// Launches the configured ARAMKit desktop executable.
pub fn launch_desktop_app(path: &Path) -> Result<(), String> {
    validate_app_path(path, true)?;

    #[cfg(windows)]
    {
        launch_windows(path)
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("无法启动 ARAMKit：{err}"))
    }
}

#[cfg(windows)]
fn launch_windows(path: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    let operation = wide(OsStr::new("open"));
    let file = wide(path.as_os_str());
    let is_exe = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    // For a direct executable, use its directory as the working directory.
    // A shortcut keeps the working directory stored in the .lnk itself.
    let directory = if is_exe {
        path.parent().map(|parent| wide(parent.as_os_str()))
    } else {
        None
    };
    let directory_ptr = directory
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());

    // SAFETY: every pointer passed to ShellExecuteW refers to a live,
    // NUL-terminated UTF-16 buffer that outlives this call. ShellExecuteW does
    // not retain the pointers. The call does not block on the launched process.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            directory_ptr,
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a value <= 32 on failure.
    if (result as isize) <= 32 {
        Err(format!(
            "无法启动 ARAMKit（系统错误码 {}）",
            result as isize
        ))
    } else {
        Ok(())
    }
}

/// Opens a native file picker for the ARAMKit executable on Windows.
#[cfg(windows)]
pub fn pick_app_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("选择 ARAMKit 桌面端")
        .add_filter("应用程序", &["exe", "lnk"])
        .pick_file()
}

/// The target platform is Windows; the Linux build only exists for development.
#[cfg(not(windows))]
pub fn pick_app_path() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_aramkit_champion_urls() {
        assert!(is_allowed_champion_url(
            "https://aramkit.cn/zh-CN/champions/ahri"
        ));
        assert!(is_allowed_champion_url(
            "https://aramkit.cn/zh-CN/champions/monkeyking"
        ));
    }

    #[test]
    fn rejects_other_urls() {
        assert!(!is_allowed_champion_url("https://example.com/"));
        assert!(!is_allowed_champion_url(
            "https://aramkit.cn/zh-CN/champions/"
        ));
        assert!(!is_allowed_champion_url(
            "https://aramkit.cn/zh-CN/champions/../admin"
        ));
        assert!(!is_allowed_champion_url(
            "javascript:alert(1)//https://aramkit.cn/zh-CN/champions/"
        ));
    }

    #[test]
    fn rejects_relative_app_paths() {
        assert!(validate_app_path(Path::new("ARAMKit.exe"), false).is_err());
    }

    #[test]
    fn rejects_network_app_paths() {
        assert!(validate_app_path(Path::new("//server/share/ARAMKit.exe"), false).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn accepts_local_absolute_paths_for_validation() {
        assert!(validate_app_path(Path::new("/opt/aramkit/ARAMKit"), false).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn accepts_windows_exe_paths_for_validation() {
        assert!(validate_app_path(Path::new(r"C:\Apps\ARAMKit\ARAMKit.exe"), false).is_ok());
        assert!(validate_app_path(Path::new(r"C:\Apps\ARAMKit\ARAMKit.lnk"), false).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_non_executable_paths() {
        assert!(validate_app_path(Path::new(r"C:\Apps\ARAMKit\readme.txt"), false).is_err());
    }
}
