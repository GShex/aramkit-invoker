use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use aramkit_invoker::config::{self, Config};
use aramkit_invoker::lcu::{ChampionPage, LcuEvent, Status, WatcherHandle};
use aramkit_invoker::platform::{launch_desktop_app, open_url, pick_app_path, validate_app_path};

use crate::theme::{
    card, champion_display, draw_logo, phase_label, queue_label, section_title, setup_style,
    status_dot, status_pill, ACCENT, BAD, BG, MUTED, OK, TEXT, WARN,
};
use crate::tray::Tray;

pub struct InvokerApp {
    config: Config,
    config_path: Option<PathBuf>,
    path_input: String,
    settings_note: Option<(String, bool)>,
    rx: Receiver<LcuEvent>,
    _watcher: WatcherHandle,
    tray: Option<Tray>,
    quitting: Arc<AtomicBool>,
    status: Status,
    last_page: Option<ChampionPage>,
    logs: VecDeque<String>,
    tray_connected: Option<bool>,
    tray_error_logged: bool,
    pending_pick: Option<Receiver<Option<PathBuf>>>,
    watcher_dead: bool,
}

impl InvokerApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        setup_style(&creation_context.egui_ctx);

        let config_path = config::default_config_path();
        let (config, config_error) = match &config_path {
            Some(path) => match Config::load(path) {
                Ok(config) => (config, None),
                Err(error) => (Config::default(), Some(error)),
            },
            None => (Config::default(), Some("无法确定配置目录".to_owned())),
        };

        let quitting = Arc::new(AtomicBool::new(false));
        let tray_result = Tray::new(&creation_context.egui_ctx, Arc::clone(&quitting));

        let (tx, rx) = mpsc::channel();
        let wake_context = creation_context.egui_ctx.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || wake_context.request_repaint());
        let watcher = WatcherHandle::spawn(tx, wake);

        let path_input = config
            .aramkit_app_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();

        let mut app = Self {
            config,
            config_path,
            path_input,
            settings_note: None,
            rx,
            _watcher: watcher,
            tray: None,
            quitting,
            status: Status::disconnected("请启动英雄联盟客户端"),
            last_page: None,
            logs: VecDeque::new(),
            tray_connected: None,
            tray_error_logged: false,
            pending_pick: None,
            watcher_dead: false,
        };

        app.push_log("ARAMKit Invoker 已启动");
        app.push_log("等待英雄联盟客户端连接");

        if let Some(error) = config_error {
            app.push_log(error);
        }

        match tray_result {
            Ok(tray) => app.tray = Some(tray),
            Err(error) => app.push_log(format!("系统托盘不可用：{error}")),
        }

        if let Some(path) = app.config.aramkit_app_path.clone() {
            match launch_desktop_app(&path) {
                Ok(()) => app.push_log(format!("已自动启动 ARAMKit：{}", path.display())),
                Err(error) => {
                    app.push_log(format!("自动启动 ARAMKit 失败：{error}"));
                }
            }
        }

        app
    }

    fn drain_events(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(LcuEvent::Status(status)) => self.status = status,
                Ok(LcuEvent::Log(line)) => self.push_log(line),
                Ok(LcuEvent::ChampionOpened(page)) => {
                    match &page.error {
                        Some(error) => self.push_log(format!(
                            "打开 {} 的 ARAMKit 页面失败：{error}",
                            page.champion_name
                        )),
                        None => {
                            self.push_log(format!("已打开 {} 的 ARAMKit 页面", page.champion_name))
                        }
                    }
                    self.last_page = Some(page);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.watcher_dead {
                        self.watcher_dead = true;
                        self.status = Status::disconnected("后台监控线程已停止");
                        self.push_log("后台监控线程已停止，请重启程序");
                    }
                    break;
                }
            }
        }
    }

    fn refresh_tray(&mut self) {
        let connected = self.status.connected;
        if self.tray_connected == Some(connected) {
            return;
        }

        let result = match self.tray.as_mut() {
            Some(tray) => tray.set_connected(connected),
            None => Ok(()),
        };

        match result {
            Ok(()) => {
                self.tray_connected = Some(connected);
                self.tray_error_logged = false;
            }
            Err(error) => {
                // Leave the cached state unchanged so the next tick retries,
                // but do not flood the log with the same error.
                if !self.tray_error_logged {
                    self.push_log(error);
                    self.tray_error_logged = true;
                }
            }
        }
    }

    fn poll_pending_pick(&mut self) {
        let result = match self.pending_pick.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(Some(path)) => {
                self.pending_pick = None;
                self.path_input = path.display().to_string();
                self.save_config();
            }
            Ok(None) => {
                self.pending_pick = None;
                self.settings_note = None;
                #[cfg(not(windows))]
                {
                    self.settings_note = Some(("当前开发构建不支持文件选择".to_owned(), true));
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pending_pick = None;
                self.settings_note = Some(("文件选择器异常退出".to_owned(), true));
            }
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if !close_requested {
            return;
        }

        if self.quitting.load(Ordering::SeqCst) || self.tray.is_none() {
            return;
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.push_log("已最小化到系统托盘");
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        self.header(ui);
        ui.add_space(14.0);
        self.status_card(ui);
        ui.add_space(10.0);
        self.last_page_card(ui);
        ui.add_space(10.0);
        self.settings_card(ui);
        ui.add_space(10.0);
        self.logs_card(ui);
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let connected = self.status.connected;
        ui.horizontal(|ui| {
            draw_logo(ui);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("ARAMKit Invoker")
                        .size(22.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    egui::RichText::new("海克斯大乱斗 · 自动打开 ARAMKit 英雄页面")
                        .size(12.0)
                        .color(MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                status_pill(ui, connected)
            });
        });
    }

    fn status_card(&mut self, ui: &mut egui::Ui) {
        let connected = self.status.connected;
        let phase = self.status.phase.clone();
        let queue_id = self.status.queue_id;
        let champion_id = self.status.champion_id;
        let champion_name = self.status.champion_name.clone();
        let message = self.status.message.clone();

        card(ui, |ui| {
            ui.label(section_title("客户端状态"));
            ui.add_space(8.0);

            let (main_text, color) = if connected {
                ("客户端已连接", OK)
            } else {
                ("等待客户端连接", WARN)
            };

            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(
                    egui::RichText::new(main_text)
                        .size(20.0)
                        .strong()
                        .color(TEXT),
                );
                if connected && !phase.is_empty() {
                    ui.label(
                        egui::RichText::new(phase_label(&phase))
                            .size(12.0)
                            .color(MUTED),
                    );
                }
            });

            if connected {
                let mut details = Vec::new();
                if queue_id > 0 {
                    details.push(queue_label(queue_id));
                }
                if champion_id > 0 {
                    details.push(format!(
                        "当前英雄：{}",
                        champion_display(champion_id, &champion_name)
                    ));
                }
                if !details.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(details.join(" · "))
                            .size(13.0)
                            .color(MUTED),
                    );
                }
            }

            if !message.is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(message).size(13.0).color(TEXT));
            }
        });
    }

    fn last_page_card(&mut self, ui: &mut egui::Ui) {
        let page = self.last_page.clone();
        let mut reopen_url = None;

        card(ui, |ui| {
            ui.label(section_title("最近打开"));
            ui.add_space(8.0);

            match page {
                Some(page) => match &page.error {
                    Some(error) => {
                        ui.label(
                            egui::RichText::new(format!("打开失败：{error}"))
                                .size(13.0)
                                .color(BAD),
                        );
                        if !page.url.is_empty() {
                            ui.add_space(4.0);
                            ui.hyperlink_to(
                                egui::RichText::new(&page.url).size(11.0).color(MUTED),
                                &page.url,
                            );
                            ui.add_space(6.0);
                            if ui.button("重试").clicked() {
                                reopen_url = Some(page.url.clone());
                            }
                        }
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(&page.champion_name)
                                    .size(17.0)
                                    .strong()
                                    .color(TEXT),
                            );
                            ui.label(egui::RichText::new("ARAMKit").size(11.0).color(MUTED));
                        });
                        ui.add_space(4.0);
                        ui.hyperlink_to(
                            egui::RichText::new(&page.url).size(11.5).color(ACCENT),
                            &page.url,
                        );
                        ui.add_space(8.0);
                        if ui.button("再次打开").clicked() {
                            reopen_url = Some(page.url.clone());
                        }
                    }
                },
                None => {
                    ui.label(
                        egui::RichText::new(
                            "尚未打开过英雄页面。进入海克斯大乱斗并确认英雄后会自动打开。",
                        )
                        .size(13.0)
                        .color(MUTED),
                    );
                }
            }
        });

        if let Some(url) = reopen_url {
            match open_url(&url) {
                Ok(()) => self.push_log(format!("已重新打开 {url}")),
                Err(error) => self.push_log(format!("打开页面失败：{error}")),
            }
        }
    }

    fn settings_card(&mut self, ui: &mut egui::Ui) {
        let mut browse = false;
        let mut save = false;
        let mut launch = false;
        let mut edited = false;
        let note = self.settings_note.clone();
        let config_path = self.config_path.clone();

        card(ui, |ui| {
            ui.label(section_title("ARAMKit 桌面端"));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("配置程序路径后，本程序每次启动都会自动启动它。")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                let width = (ui.available_width() - 96.0).max(120.0);
                let response = ui.add_sized(
                    [width, 30.0],
                    egui::TextEdit::singleline(&mut self.path_input)
                        .hint_text(r"例如 C:\Apps\ARAMKit\ARAMKit.exe"),
                );
                if response.changed() {
                    edited = true;
                }
                if ui.button("浏览…").clicked() {
                    browse = true;
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let save_button = egui::Button::new(
                    egui::RichText::new("保存")
                        .size(13.0)
                        .color(egui::Color32::WHITE),
                )
                .fill(ACCENT)
                .corner_radius(8);
                if ui.add(save_button).clicked() {
                    save = true;
                }
                if ui.button("立即启动").clicked() {
                    launch = true;
                }
                if let Some((text, is_error)) = &note {
                    ui.label(egui::RichText::new(text).size(11.5).color(if *is_error {
                        BAD
                    } else {
                        OK
                    }));
                }
            });

            if let Some(path) = &config_path {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("配置文件：{}", path.display()))
                        .size(10.5)
                        .color(MUTED),
                );
            }
        });

        if edited {
            self.settings_note = None;
        }
        if browse && self.pending_pick.is_none() {
            let (tx, rx) = mpsc::channel();
            // Never block the egui/winit event loop on a native modal dialog.
            let _ = std::thread::Builder::new()
                .name("file-picker".to_owned())
                .spawn(move || {
                    let _ = tx.send(pick_app_path());
                });
            self.pending_pick = Some(rx);
            self.settings_note = Some(("正在打开文件选择器…".to_owned(), false));
        }
        if save {
            self.save_config();
        }
        if launch {
            self.launch_now();
        }
    }

    fn save_config(&mut self) {
        let path_text = self.path_input.trim().trim_matches('"').to_owned();
        self.path_input = path_text.clone();
        let new_path = if path_text.is_empty() {
            None
        } else {
            Some(PathBuf::from(path_text))
        };

        if let Some(path) = &new_path {
            if let Err(error) = validate_app_path(path, false) {
                self.settings_note = Some((error, true));
                return;
            }
        }
        self.config.aramkit_app_path = new_path;

        let Some(config_path) = self.config_path.clone() else {
            self.settings_note = Some(("无法确定配置目录".to_owned(), true));
            return;
        };

        match self.config.save(&config_path) {
            Ok(()) => {
                let missing = self
                    .config
                    .aramkit_app_path
                    .as_ref()
                    .is_some_and(|path| !path.is_file());
                self.settings_note = Some(if missing {
                    ("已保存，但路径当前不存在".to_owned(), true)
                } else {
                    ("已保存".to_owned(), false)
                });
            }
            Err(error) => {
                self.settings_note = Some((error.clone(), true));
                self.push_log(error);
            }
        }
    }

    fn launch_now(&mut self) {
        self.save_config();

        let path_text = self.path_input.trim().trim_matches('"').to_owned();
        if path_text.is_empty() {
            self.settings_note = Some(("请先配置路径".to_owned(), true));
            return;
        }

        let path = PathBuf::from(path_text);
        match launch_desktop_app(&path) {
            Ok(()) => {
                self.settings_note = Some(("已启动".to_owned(), false));
                self.push_log(format!("已启动 ARAMKit：{}", path.display()));
            }
            Err(error) => {
                self.settings_note = Some((error.clone(), true));
                self.push_log(error);
            }
        }
    }

    fn logs_card(&mut self, ui: &mut egui::Ui) {
        let mut clear = false;

        card(ui, |ui| {
            egui::CollapsingHeader::new(section_title("运行日志"))
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("清空").clicked() {
                            clear = true;
                        }
                        ui.label(
                            egui::RichText::new("展开后可查看连接与触发记录")
                                .size(10.5)
                                .color(MUTED),
                        );
                    });
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical()
                        .max_height(112.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            if self.logs.is_empty() {
                                ui.label(egui::RichText::new("暂无日志").size(11.0).color(MUTED));
                            } else {
                                for line in &self.logs {
                                    ui.label(egui::RichText::new(line).size(11.0).color(MUTED));
                                }
                            }
                        });
                });
        });

        if clear {
            self.logs.clear();
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.add_space(6.0);

        let tray_available = self.tray.is_some();
        let mut hide = false;
        let mut quit = false;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(if tray_available {
                    "关闭窗口会最小化到系统托盘"
                } else {
                    "系统托盘不可用，关闭窗口将退出程序"
                })
                .size(11.0)
                .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("退出").clicked() {
                    quit = true;
                }
                if ui
                    .add_enabled(tray_available, egui::Button::new("隐藏到托盘"))
                    .clicked()
                {
                    hide = true;
                }
            });
        });

        if quit {
            self.quitting.store(true, Ordering::SeqCst);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            ui.ctx().request_repaint();
        }

        if hide {
            self.push_log("已最小化到系统托盘");
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        if self.logs.len() >= 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(message.into());
    }
}

impl eframe::App for InvokerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.poll_pending_pick();
        self.refresh_tray();
        self.handle_close_request(ctx);

        // Keeps tray events and the status tooltip live even while the
        // window is hidden.
        ctx.request_repaint_after(Duration::from_millis(350));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("app_footer")
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(24, 10)),
            )
            .show(ui, |ui| self.footer(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(24, 16))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                self.body(ui);
                            });
                    });
            });
    }
}
