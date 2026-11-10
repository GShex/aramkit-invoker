use std::path::{Path, PathBuf};

use crate::champion::ARAMKIT_CHAMPION_BASE;
use crate::config::{GameLauncher, WindowBounds};

#[cfg(windows)]
mod edge_navigation;

#[derive(Default)]
pub struct GameInstall {
    pub tcls: Option<PathBuf>,
    pub wegame: Option<PathBuf>,
}

impl GameInstall {
    pub fn detect() -> Self {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Registry::{
                HKEY_CLASSES_ROOT, HKEY_LOCAL_MACHINE, RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY,
            };

            let tcls = [RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY]
                .into_iter()
                .find_map(|view| {
                    let root = registry_value(
                        HKEY_LOCAL_MACHINE,
                        r"SOFTWARE\Tencent\LOL",
                        "InstallPath",
                        view,
                    )?;
                    let path = PathBuf::from(root).join(r"TCLS\Client.exe");
                    detected_launcher_path(path)
                });
            let wegame = registry_value(HKEY_CLASSES_ROOT, r"wegame\shell\open\command", "", 0)
                .and_then(|command| {
                    let executable = command
                        .strip_prefix('"')
                        .and_then(|quoted| quoted.split_once('"').map(|(path, _)| path))
                        .or_else(|| command.split_once(" \"%1\"").map(|(path, _)| path))?;
                    let path = PathBuf::from(executable);
                    detected_launcher_path(path)
                });
            Self { tcls, wegame }
        }
        #[cfg(not(windows))]
        Self::default()
    }
}

#[cfg(windows)]
fn detected_launcher_path(path: PathBuf) -> Option<PathBuf> {
    validate_app_path(&path, true).ok()?;
    // Read the directory names from the filesystem instead of trusting the
    // spelling recorded by each installer.
    let normalized = path.canonicalize().ok().and_then(|canonical| {
        let text = canonical.to_str()?.strip_prefix(r"\\?\")?;
        let normalized = PathBuf::from(text);
        validate_app_path(&normalized, true).ok()?;
        Some(normalized)
    });
    Some(normalized.unwrap_or(path))
}

#[cfg(windows)]
fn registry_value(
    root: windows_sys::Win32::System::Registry::HKEY,
    key: &str,
    name: &str,
    view: u32,
) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};

    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut value = vec![0u16; 32_768];
    let mut bytes = (value.len() * size_of::<u16>()) as u32;
    // SAFETY: the names are NUL-terminated and bytes bounds the writable buffer.
    let result = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | view,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let end = value.iter().position(|unit| *unit == 0)?;
    String::from_utf16(&value[..end]).ok()
}

pub fn launch_game(launcher: GameLauncher, path: &Path) -> Result<(), String> {
    validate_app_path(path, true)?;
    if launcher == GameLauncher::WeGame
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        #[cfg(windows)]
        return launch_windows_with_args(path, Some("wegame://StartFor=26"));
    }
    launch_desktop_app(path)
}

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

/// Owns the browser window used for champion pages.
#[derive(Default)]
pub(crate) struct ChampionWindow {
    closed: bool,
    bounds: Option<WindowBounds>,
    #[cfg(windows)]
    edge: Option<EdgeWindow>,
}

impl ChampionWindow {
    pub(crate) fn new(bounds: Option<WindowBounds>) -> Self {
        Self {
            bounds,
            ..Self::default()
        }
    }

    pub(crate) fn bounds(&mut self) -> Option<WindowBounds> {
        #[cfg(windows)]
        if let Some(edge) = self.edge.as_mut() {
            if let Some(bounds) = edge.update_bounds(self.bounds) {
                self.bounds = Some(bounds);
            }
        }
        self.bounds
    }

    pub(crate) fn open(&mut self, url: &str) -> Result<(), String> {
        if self.closed {
            return Err("程序正在退出".to_owned());
        }
        if !is_allowed_champion_url(url) {
            return Err("拒绝打开非 ARAMKit 英雄页面链接".to_owned());
        }

        #[cfg(windows)]
        {
            if let Some(edge) = self.edge.as_mut() {
                match edge_navigation::navigate(edge.profile.path(), &mut edge.target_id, url) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {
                        if edge.started.elapsed() < std::time::Duration::from_secs(5)
                            || !edge.windows().is_empty()
                        {
                            return Err("ARAMKit 专用窗口暂时没有可更新的页面".to_owned());
                        }
                    }
                    Err(error) => {
                        if edge.started.elapsed() < std::time::Duration::from_secs(5)
                            || !edge.windows().is_empty()
                        {
                            return Err(format!("无法更新 ARAMKit 专用窗口：{error}"));
                        }
                    }
                }
            }

            let executable = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
                .into_iter()
                .filter_map(std::env::var_os)
                .map(|root| PathBuf::from(root).join(r"Microsoft\Edge\Application\msedge.exe"))
                .find(|path| path.is_file())
                .ok_or_else(|| "找不到 Microsoft Edge，无法打开专用窗口".to_owned())?;

            let profile = tempfile::Builder::new()
                .prefix("aramkit-edge-")
                .tempdir()
                .map_err(|error| format!("无法创建专用浏览器配置目录：{error}"))?;
            let mut profile_arg = std::ffi::OsString::from("--user-data-dir=");
            profile_arg.push(profile.path());

            // Recreate the browser only when the dedicated page is gone.
            self.bounds();
            if let Some(edge) = self.edge.as_mut() {
                edge.close()?;
            }
            self.edge = None;
            let child = std::process::Command::new(executable)
                .arg(profile_arg)
                .arg(format!("--app={url}"))
                .args([
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-background-mode",
                    "--disable-extensions",
                    "--disable-sync",
                    "--remote-debugging-address=127.0.0.1",
                    "--remote-debugging-port=0",
                ])
                .spawn()
                .map_err(|error| format!("无法启动 ARAMKit 专用窗口：{error}"))?;
            self.edge = Some(EdgeWindow {
                child,
                profile,
                restored: false,
                target_id: None,
                started: std::time::Instant::now(),
            });
            Ok(())
        }

        #[cfg(not(windows))]
        open_url(url)
    }

    pub(crate) fn close(&mut self) {
        self.bounds();
        self.closed = true;
        #[cfg(windows)]
        {
            if let Some(mut edge) = self.edge.take() {
                let _ = edge.close();
            }
        }
    }
}

#[cfg(windows)]
struct EdgeWindow {
    child: std::process::Child,
    profile: tempfile::TempDir,
    restored: bool,
    target_id: Option<String>,
    started: std::time::Instant,
}

#[cfg(windows)]
impl EdgeWindow {
    fn update_bounds(&mut self, saved: Option<WindowBounds>) -> Option<WindowBounds> {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, IsIconic, IsZoomed, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
        };

        let hwnd = self.windows().first()?.0;
        // SAFETY: hwnd comes from the live browser process and rect is writable.
        unsafe {
            if IsIconic(hwnd) != 0 || IsZoomed(hwnd) != 0 {
                return None;
            }
            if !self.restored {
                self.restored = true;
                if let Some(saved) = saved.filter(|b| b.width > 0 && b.height > 0) {
                    let title_area = RECT {
                        left: saved.x,
                        top: saved.y,
                        right: saved.x.saturating_add(saved.width),
                        bottom: saved.y.saturating_add(32),
                    };
                    if !MonitorFromRect(&title_area, MONITOR_DEFAULTTONULL).is_null() {
                        SetWindowPos(
                            hwnd,
                            std::ptr::null_mut(),
                            saved.x,
                            saved.y,
                            saved.width,
                            saved.height,
                            SWP_NOACTIVATE | SWP_NOZORDER,
                        );
                    }
                }
            }
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            (width > 0 && height > 0).then_some(WindowBounds {
                x: rect.left,
                y: rect.top,
                width,
                height,
            })
        }
    }

    fn windows(&mut self) -> Vec<(windows_sys::Win32::Foundation::HWND, u32)> {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

        let windows = edge_windows();
        let launch_pid = self.child.id();
        if matches!(self.child.try_wait(), Ok(None)) {
            let owned: Vec<_> = windows
                .iter()
                .copied()
                .filter(|&(_, pid)| pid == launch_pid)
                .collect();
            if !owned.is_empty() {
                return owned;
            }
        }

        // Edge may relaunch under a different token and hand the window to
        // another process. Match the unique profile, never Edge's name alone.
        let pids: Vec<_> = windows.iter().map(|&(_, pid)| Pid::from_u32(pid)).collect();
        if pids.is_empty() {
            return Vec::new();
        }
        let Ok(mut system) = System::new() else {
            return Vec::new();
        };
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::Always)
                .without_tasks(),
        );
        let mut profile_arg = std::ffi::OsString::from("--user-data-dir=");
        profile_arg.push(self.profile.path());
        let profile_arg = profile_arg.to_string_lossy();
        windows
            .into_iter()
            .filter(|&(_, pid)| {
                system.process(Pid::from_u32(pid)).is_some_and(|process| {
                    process.name().eq_ignore_ascii_case("msedge.exe")
                        && process
                            .cmd()
                            .iter()
                            .any(|arg| arg.to_string_lossy().eq_ignore_ascii_case(&profile_arg))
                })
            })
            .collect()
    }

    fn close(&mut self) -> Result<(), String> {
        use std::time::{Duration, Instant};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
        };

        let windows = self.windows();
        for &(hwnd, pid) in &windows {
            let mut current_pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut current_pid) };
            if current_pid == pid {
                unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
            }
        }

        // Only terminate the process handle created by this launcher. A
        // relaunched browser is closed through its verified profile windows.
        if matches!(self.child.try_wait(), Ok(None)) && self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while windows.iter().any(|&(hwnd, pid)| {
            let mut current_pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut current_pid) };
            current_pid == pid
        }) {
            if Instant::now() >= deadline {
                return Err("上一个 ARAMKit 专用窗口尚未关闭，请关闭后重试".to_owned());
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn edge_windows() -> Vec<(windows_sys::Win32::Foundation::HWND, u32)> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    };

    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> i32 {
        // EnumWindows is synchronous and passes back the live search state.
        let found = unsafe { &mut *(data as *mut Vec<(HWND, u32)>) };
        if unsafe { IsWindowVisible(hwnd) == 0 } {
            return 1;
        }
        let mut class = [0u16; 32];
        let count = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        if count > 0
            && String::from_utf16_lossy(&class[..count as usize]).starts_with("Chrome_WidgetWin_")
        {
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            if pid != 0 {
                found.push((hwnd, pid));
            }
        }
        1
    }

    let mut data: Vec<(HWND, u32)> = Vec::new();
    unsafe { EnumWindows(Some(find), (&mut data as *mut Vec<(HWND, u32)>) as LPARAM) };
    data
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
    launch_windows_with_args(path, None)
}

#[cfg(windows)]
fn launch_windows_with_args(path: &Path, args: Option<&str>) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    let operation = wide(OsStr::new("open"));
    let file = wide(path.as_os_str());
    let parameters = args.map(|value| wide(OsStr::new(value)));
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
            parameters
                .as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr()),
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
