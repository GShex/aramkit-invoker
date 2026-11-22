use aramkit_invoker::config::WindowBounds;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindowVisible, IsZoomed, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
};

/// Physical screen coordinates preserve negative positions on secondary monitors.
pub struct MainWindow {
    handle: isize,
    last_bounds: Option<WindowBounds>,
}

impl MainWindow {
    pub fn new(context: &eframe::CreationContext<'_>, saved: Option<WindowBounds>) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = context.window_handle().ok()?.as_raw() else {
            return None;
        };
        let mut window = Self {
            handle: handle.hwnd.get(),
            last_bounds: None,
        };
        if let Some(bounds) = saved.filter(|b| b.width > 0 && b.height > 0) {
            let title_area = RECT {
                left: bounds.x,
                top: bounds.y,
                right: bounds.x.saturating_add(bounds.width),
                bottom: bounds.y.saturating_add(32),
            };
            // SAFETY: the live eframe window owns this handle. All calls are
            // made on its UI thread, with valid rectangle storage.
            unsafe {
                if !MonitorFromRect(&title_area, MONITOR_DEFAULTTONULL).is_null() {
                    if SetWindowPos(
                        window.hwnd(),
                        std::ptr::null_mut(),
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    ) != 0
                    {
                        window.last_bounds = Some(bounds);
                    }
                }
            }
        }
        Some(window)
    }

    fn hwnd(&self) -> HWND {
        self.handle as HWND
    }

    pub fn bounds(&mut self) -> Option<WindowBounds> {
        // Keep the last normal rectangle when hidden, minimized or maximized.
        // A failed read during shutdown also keeps the last valid position.
        unsafe {
            let hwnd = self.hwnd();
            if IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 && IsZoomed(hwnd) == 0 {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect) != 0 {
                    let width = rect.right - rect.left;
                    let height = rect.bottom - rect.top;
                    if width > 0 && height > 0 {
                        self.last_bounds = Some(WindowBounds {
                            x: rect.left,
                            y: rect.top,
                            width,
                            height,
                        });
                    }
                }
            }
        }
        self.last_bounds
    }
}
