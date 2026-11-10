#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::icon;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    const TOOLTIP_WAITING: &str = "等待客户端连接";
    const TOOLTIP_CONNECTED: &str = "客户端已连接";

    pub struct Tray {
        icon: TrayIcon,
        connected: bool,
    }

    impl Tray {
        pub fn new(ctx: &egui::Context, quitting: Arc<AtomicBool>) -> Result<Self, String> {
            let show_item = MenuItem::with_id("show", "显示主窗口", true, None);
            let quit_item = MenuItem::with_id("quit", "退出", true, None);
            let menu = Menu::with_items(&[&show_item, &quit_item])
                .map_err(|error| format!("创建托盘菜单失败：{error}"))?;

            let tray_icon = tray_icon::Icon::from_rgba(icon::rgba(false), icon::SIZE, icon::SIZE)
                .map_err(|error| format!("创建托盘图标失败：{error}"))?;

            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip(TOOLTIP_WAITING)
                .with_icon(tray_icon)
                .build()
                .map_err(|error| format!("创建系统托盘失败：{error}"))?;

            let click_ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                ) {
                    show_window(&click_ctx);
                }
            }));

            let menu_ctx = ctx.clone();
            let menu_quitting = quitting;
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
                "show" => show_window(&menu_ctx),
                "quit" => {
                    menu_quitting.store(true, Ordering::SeqCst);
                    menu_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    menu_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    menu_ctx.request_repaint();
                }
                _ => {}
            }));

            Ok(Self {
                icon: tray,
                connected: false,
            })
        }

        pub fn set_connected(&mut self, connected: bool) -> Result<(), String> {
            if self.connected == connected {
                return Ok(());
            }

            self.icon
                .set_tooltip(Some(if connected {
                    TOOLTIP_CONNECTED
                } else {
                    TOOLTIP_WAITING
                }))
                .map_err(|error| format!("更新托盘提示失败：{error}"))?;

            let tray_icon =
                tray_icon::Icon::from_rgba(icon::rgba(connected), icon::SIZE, icon::SIZE)
                    .map_err(|error| format!("更新托盘图标失败：{error}"))?;
            self.icon
                .set_icon(Some(tray_icon))
                .map_err(|error| format!("更新托盘图标失败：{error}"))?;

            self.connected = connected;
            Ok(())
        }
    }

    pub(crate) fn show_window(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// Development-only stub. The tray is a Windows feature.
    pub struct Tray;

    impl Tray {
        pub fn new(_ctx: &egui::Context, _quitting: Arc<AtomicBool>) -> Result<Self, String> {
            Err("当前平台不支持系统托盘（仅 Windows 构建可用）".to_owned())
        }

        pub fn set_connected(&mut self, _connected: bool) -> Result<(), String> {
            Ok(())
        }
    }
}

#[cfg(windows)]
pub(crate) use imp::show_window;
pub use imp::Tray;
