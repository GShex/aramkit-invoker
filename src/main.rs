// Hide the console window in release builds; this is a tray application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod theme;
mod tray;

use std::sync::Arc;

fn main() -> eframe::Result {
    #[cfg(windows)]
    let Some(instance) = SingleInstance::acquire() else {
        return Ok(());
    };

    let window_icon = Arc::new(icon::egui_icon(true));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ARAMKit Invoker")
            .with_inner_size([560.0, 640.0])
            .with_min_inner_size([480.0, 520.0])
            .with_icon(window_icon),
        ..Default::default()
    };

    eframe::run_native(
        "ARAMKit Invoker",
        native_options,
        Box::new(move |creation_context| {
            Ok(Box::new(app::InvokerApp::new(
                creation_context,
                #[cfg(windows)]
                instance,
            )))
        }),
    )
}

#[cfg(windows)]
struct SingleInstance {
    _mutex: std::os::windows::io::OwnedHandle,
    show_event: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl SingleInstance {
    fn acquire() -> Option<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent};

        let wide = |name: &str| name.encode_utf16().chain([0]).collect::<Vec<_>>();
        // Open the event before the mutex so it is ready during startup.
        let event_name = wide(r"Local\eliozh.aramkit-invoker.show-window");
        // SAFETY: the name is NUL-terminated and null selects default security.
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, event_name.as_ptr()) };
        if event.is_null() {
            return None;
        }
        let show_event = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(event) };

        let mutex_name = wide(r"Local\eliozh.aramkit-invoker.single-instance");
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            return None;
        }
        let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(mutex) };
        if already_running {
            unsafe { SetEvent(event) };
            return None;
        }
        Some(Self {
            _mutex: mutex,
            show_event,
        })
    }

    fn take_show_request(&self) -> bool {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;

        // The auto-reset event consumes the request without blocking the UI.
        (unsafe { WaitForSingleObject(self.show_event.as_raw_handle(), 0) }) == WAIT_OBJECT_0
    }
}
