use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aramkit_invoker::config::{self, Config, GameLauncher, WindowBounds};
use aramkit_invoker::lcu::{ChampionPage, LcuEvent, Status, WatcherHandle};
use aramkit_invoker::platform::{
    launch_desktop_app, launch_game, pick_app_path, validate_app_path, GameInstall,
};

use crate::theme::{
    card, champion_display, draw_logo, phase_label, queue_label, section_title, setup_style,
    status_dot, status_pill, ACCENT, BAD, BG, MUTED, OK, TEXT, WARN,
};
use crate::tray::Tray;

fn game_path_index(launcher: GameLauncher) -> usize {
    match launcher {
        GameLauncher::Tcls => 0,
        GameLauncher::WeGame => 1,
    }
}

fn path_actions(ui: &mut egui::Ui, path: &mut String, hint: &str) -> (bool, bool, bool) {
    let height = 30.0;
    let button_size = [54.0, height];
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let launch = ui
                .add_sized(
                    button_size,
                    egui::Button::new(egui::RichText::new("启动").color(egui::Color32::WHITE))
                        .fill(ACCENT),
                )
                .clicked();
            let browse = ui
                .add_sized(button_size, egui::Button::new("浏览"))
                .clicked();
            let edited = ui
                .add_sized(
                    [ui.available_width(), height],
                    egui::TextEdit::singleline(path)
                        .align(egui::Align2::LEFT_CENTER)
                        .hint_text(hint),
                )
                .changed();
            (edited, browse, launch)
        },
    )
    .inner
}

pub struct InvokerApp {
    #[cfg(windows)]
    instance: crate::SingleInstance,
    config: Config,
    config_path: Option<PathBuf>,
    path_input: String,
    settings_note: Option<(String, bool)>,
    game_install: GameInstall,
    game_paths: [String; 2],
    auto_launch_game: Option<GameLauncher>,
    game_note: Option<(String, bool)>,
    rx: Receiver<LcuEvent>,
    _watcher: WatcherHandle,
    pending_browser_bounds: Option<(WindowBounds, Instant)>,
    tray: Option<Tray>,
    quitting: Arc<AtomicBool>,
    status: Status,
    last_page: Option<ChampionPage>,
    logs: VecDeque<String>,
    tray_connected: Option<bool>,
    tray_error_logged: bool,
    pending_pick: Option<(Option<GameLauncher>, Receiver<Option<PathBuf>>)>,
    watcher_dead: bool,
}

impl InvokerApp {
    pub fn new(
        creation_context: &eframe::CreationContext<'_>,
        #[cfg(windows)] instance: crate::SingleInstance,
    ) -> Self {
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
        let watcher = WatcherHandle::spawn(tx, wake, config.browser_window);

        let path_input = config
            .aramkit_app_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();

        let mut app = Self {
            #[cfg(windows)]
            instance,
            game_install: GameInstall::detect(),
            game_paths: [&config.tcls_path, &config.wegame_path].map(|path| {
                path.as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            }),
            auto_launch_game: config.auto_launch_game,
            game_note: None,
            config,
            config_path,
            path_input,
            settings_note: None,
            rx,
            _watcher: watcher,
            pending_browser_bounds: None,
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

        if let Some(launcher) = app.config.auto_launch_game {
            app.launch_game_now(launcher);
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
        let Some((launcher, receiver)) = &self.pending_pick else {
            return;
        };
        let launcher = *launcher;
        let result = receiver.try_recv();

        match result {
            Ok(Some(path)) => {
                self.pending_pick = None;
                if let Some(launcher) = launcher {
                    self.game_paths[game_path_index(launcher)] = path.display().to_string();
                    self.autosave_game_settings();
                } else {
                    self.path_input = path.display().to_string();
                    self.save_config();
                }
            }
            Ok(None) => {
                self.pending_pick = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pending_pick = None;
                let note = Some(("文件选择器异常退出".to_owned(), true));
                if launcher.is_some() {
                    self.game_note = note;
                } else {
                    self.settings_note = note;
                }
            }
        }
    }

    fn browse_path(&mut self, launcher: Option<GameLauncher>, ctx: &egui::Context) {
        if self.pending_pick.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("file-picker".to_owned())
            .spawn(move || {
                let _ = tx.send(pick_app_path());
                ctx.request_repaint();
            });
        self.pending_pick = Some((launcher, rx));
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

            ui.add_space(6.0);
            egui::CollapsingHeader::new(section_title("启动设置"))
                .default_open(false)
                .show(ui, |ui| self.game_launch_options(ui));
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
            match self._watcher.open_url(&url) {
                Ok(()) => self.push_log(format!("已重新打开 {url}")),
                Err(error) => self.push_log(format!("打开页面失败：{error}")),
            }
        }
    }

    fn game_launch_options(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.add_space(4.0);
            let mut changed = false;
            for (index, launcher) in [GameLauncher::Tcls, GameLauncher::WeGame]
                .into_iter()
                .enumerate()
            {
                ui.label(egui::RichText::new(launcher.label()).strong());
                let detected = match launcher {
                    GameLauncher::Tcls => &self.game_install.tcls,
                    GameLauncher::WeGame => &self.game_install.wegame,
                };
                let status = detected.as_ref().map_or_else(
                    || "未检测到，请填写启动程序或快捷方式的完整路径".to_owned(),
                    |path| {
                        let mut display = path.to_string_lossy().into_owned();
                        if matches!(display.as_bytes(), [b'a'..=b'z', b':', ..]) {
                            display[..1].make_ascii_uppercase();
                        }
                        format!("已检测到 {display}")
                    },
                );
                ui.label(egui::RichText::new(status).size(11.5).color(MUTED));
                let (edited, browse, launch) = path_actions(
                    ui,
                    &mut self.game_paths[index],
                    "手动路径（留空使用自动检测）",
                );
                changed |= edited;
                if browse {
                    self.browse_path(Some(launcher), ui.ctx());
                }
                if launch {
                    self.launch_game_now(launcher);
                }
                ui.add_space(4.0);
            }

            if ui.button("重新检测").clicked() {
                self.game_install = GameInstall::detect();
            }
            ui.separator();
            let previous = self.auto_launch_game;
            ui.label("本程序启动时");
            egui::ComboBox::from_id_salt("auto-launch-game")
                .selected_text(
                    self.auto_launch_game
                        .map_or("不启动游戏", GameLauncher::label),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.auto_launch_game, None, "不启动游戏");
                    for launcher in [GameLauncher::Tcls, GameLauncher::WeGame] {
                        ui.selectable_value(
                            &mut self.auto_launch_game,
                            Some(launcher),
                            launcher.label(),
                        );
                    }
                });
            changed |= self.auto_launch_game != previous;
            if changed {
                self.autosave_game_settings();
            }
            if let Some((message, is_error)) = &self.game_note {
                ui.label(egui::RichText::new(message).size(11.5).color(if *is_error {
                    BAD
                } else {
                    OK
                }));
            }
        });
    }

    fn manual_game_path(&self, launcher: GameLauncher) -> Option<PathBuf> {
        let index = game_path_index(launcher);
        let text = self.game_paths[index].trim().trim_matches('"');
        (!text.is_empty()).then(|| PathBuf::from(text))
    }

    fn autosave_game_settings(&mut self) {
        let result = self.save_game_settings();
        self.game_note = Some(match result {
            Ok(()) => ("已自动保存".to_owned(), false),
            Err(error) => (error, true),
        });
    }

    fn save_game_settings(&mut self) -> Result<(), String> {
        let config = Config {
            tcls_path: self.manual_game_path(GameLauncher::Tcls),
            wegame_path: self.manual_game_path(GameLauncher::WeGame),
            auto_launch_game: self.auto_launch_game,
            ..self.config.clone()
        };
        for path in [&config.tcls_path, &config.wegame_path]
            .into_iter()
            .flatten()
        {
            validate_app_path(path, false)?;
        }
        self.write_config(config, "游戏启动设置")
    }

    fn launch_game_now(&mut self, launcher: GameLauncher) {
        let detected = match launcher {
            GameLauncher::Tcls => &self.game_install.tcls,
            GameLauncher::WeGame => &self.game_install.wegame,
        };
        let path = self.manual_game_path(launcher).or_else(|| detected.clone());
        let result = path
            .ok_or_else(|| format!("未检测到 {}，请手动填写路径", launcher.label()))
            .and_then(|path| launch_game(launcher, &path));
        let (message, is_error) = match result {
            Ok(()) => (format!("已请求启动 {}", launcher.label()), false),
            Err(error) => (format!("启动 {} 失败：{error}", launcher.label()), true),
        };
        self.push_log(&message);
        self.game_note = Some((message, is_error));
    }

    fn settings_card(&mut self, ui: &mut egui::Ui) {
        let mut browse = false;
        let mut launch = false;
        let mut edited = false;
        let note = self.settings_note.clone();

        card(ui, |ui| {
            ui.label(section_title("ARAMKit 桌面端"));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("配置程序路径后，本程序每次启动都会自动启动它。")
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(10.0);

            (edited, browse, launch) = path_actions(
                ui,
                &mut self.path_input,
                r"例如 C:\Apps\ARAMKit\ARAMKit.exe",
            );

            if let Some((text, is_error)) = &note {
                ui.label(egui::RichText::new(text).size(11.5).color(if *is_error {
                    BAD
                } else {
                    OK
                }));
            }
        });

        if edited {
            self.save_config();
        }
        if browse {
            self.browse_path(None, ui.ctx());
        }
        if launch {
            self.launch_now();
        }
    }

    fn save_config(&mut self) {
        let path_text = self.path_input.trim().trim_matches('"').to_owned();
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
        let config = Config {
            aramkit_app_path: new_path,
            ..self.config.clone()
        };

        match self.write_config(config, "ARAMKit 桌面端路径") {
            Ok(()) => {
                let missing = self
                    .config
                    .aramkit_app_path
                    .as_ref()
                    .is_some_and(|path| !path.is_file());
                self.settings_note = Some(if missing {
                    ("已保存，但路径当前不存在".to_owned(), true)
                } else {
                    ("已自动保存".to_owned(), false)
                });
            }
            Err(error) => {
                self.settings_note = Some((error, true));
            }
        }
    }

    fn launch_now(&mut self) {
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
                    if ui.button("清空").clicked() {
                        clear = true;
                    }
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
        let tray_available = self.tray.is_some();
        let mut hide = false;
        let mut quit = false;
        let hint = if tray_available {
            "关闭窗口会最小化到系统托盘"
        } else {
            "系统托盘不可用，关闭窗口将退出程序"
        };

        ui.horizontal(|ui| {
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
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(hint).size(11.0).color(MUTED)).wrap(),
                    );
                });
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

    fn save_browser_bounds(&mut self, flush: bool) {
        if self.config_path.is_none() {
            return;
        }
        let Some(bounds) = self._watcher.browser_bounds() else {
            return;
        };
        if self.config.browser_window == Some(bounds) {
            self.pending_browser_bounds = None;
            return;
        }
        if !flush {
            match self.pending_browser_bounds {
                Some((pending, since)) if pending == bounds => {
                    if since.elapsed() < Duration::from_secs(1) {
                        return;
                    }
                }
                _ => {
                    self.pending_browser_bounds = Some((bounds, Instant::now()));
                    return;
                }
            }
        }
        let config = Config {
            browser_window: Some(bounds),
            ..self.config.clone()
        };
        self.pending_browser_bounds = None;
        let _ = self.write_config(config, "浏览器窗口位置和大小");
    }

    fn write_config(&mut self, config: Config, setting: &str) -> Result<(), String> {
        if config == self.config {
            return Ok(());
        }
        let result = self
            .config_path
            .as_deref()
            .ok_or_else(|| "无法确定配置目录".to_owned())
            .and_then(|path| config.save(path));
        match result {
            Ok(()) => {
                self.config = config;
                self.push_log(format!("配置已更新：{setting}"));
                Ok(())
            }
            Err(error) => {
                self.push_log(format!("配置更新失败（{setting}）：{error}"));
                Err(error)
            }
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        if self.logs.len() >= 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(message.into());
    }
}

impl Drop for InvokerApp {
    fn drop(&mut self) {
        self._watcher.stop();
        self.save_browser_bounds(true);
    }
}

impl eframe::App for InvokerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.poll_pending_pick();
        self.refresh_tray();
        self.handle_close_request(ctx);
        self.save_browser_bounds(false);
        #[cfg(windows)]
        if self.instance.take_show_request() && !self.quitting.load(Ordering::SeqCst) {
            crate::tray::show_window(ctx);
        }

        // Keeps tray events and the status tooltip live even while the
        // window is hidden.
        ctx.request_repaint_after(Duration::from_millis(350));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("app_footer")
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(24, 6)),
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
