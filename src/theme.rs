//! Shared visual theme and small presentation helpers for the app window.

use aramkit_invoker::champion::MAYHEM_QUEUE_ID;

pub(crate) const BG: egui::Color32 = egui::Color32::from_rgb(13, 15, 21);
pub(crate) const CARD_BG: egui::Color32 = egui::Color32::from_rgb(23, 26, 35);
pub(crate) const BORDER: egui::Color32 = egui::Color32::from_rgb(35, 39, 51);
pub(crate) const INPUT_BG: egui::Color32 = egui::Color32::from_rgb(18, 21, 29);
pub(crate) const TEXT: egui::Color32 = egui::Color32::from_rgb(235, 237, 245);
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(148, 155, 172);
pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(124, 92, 255);
pub(crate) const OK: egui::Color32 = egui::Color32::from_rgb(61, 220, 151);
pub(crate) const WARN: egui::Color32 = egui::Color32::from_rgb(255, 200, 87);
pub(crate) const BAD: egui::Color32 = egui::Color32::from_rgb(255, 107, 107);

pub(crate) fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = INPUT_BG;
    visuals.faint_bg_color = CARD_BG;
    visuals.selection.bg_fill = egui::Color32::from_rgb(70, 55, 150);
    visuals.override_text_color = Some(TEXT);

    visuals.widgets.noninteractive.bg_fill = CARD_BG;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);

    visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(34, 38, 50);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(34, 38, 50);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);

    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(45, 50, 66);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(45, 50, 66);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);

    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));

    ctx.set_style_of(egui::Theme::Dark, style);
}

pub(crate) fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(CARD_BG)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

pub(crate) fn draw_logo(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
    let painter = ui.painter();
    let center = rect.center();

    painter.circle_filled(center, 22.0, ACCENT);

    let points = vec![
        center + egui::vec2(0.0, -11.0),
        center + egui::vec2(9.0, 0.0),
        center + egui::vec2(0.0, 11.0),
        center + egui::vec2(-9.0, 0.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        egui::Color32::WHITE,
        egui::Stroke::new(0.0, egui::Color32::TRANSPARENT),
    ));
}

pub(crate) fn status_pill(ui: &mut egui::Ui, connected: bool) {
    let (label, color, background) = if connected {
        ("已连接", OK, egui::Color32::from_rgb(24, 44, 37))
    } else {
        ("搜索中", WARN, egui::Color32::from_rgb(44, 38, 24))
    };

    egui::Frame::NONE
        .fill(background)
        .corner_radius(20)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(egui::RichText::new(label).size(11.5).strong().color(color));
            });
        });
}

pub(crate) fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub(crate) fn section_title(text: &str) -> egui::RichText {
    egui::RichText::new(text).size(12.0).strong().color(MUTED)
}

pub(crate) fn champion_display(champion_id: i64, champion_name: &str) -> String {
    if champion_name.is_empty() {
        format!("英雄 #{champion_id}")
    } else {
        champion_name.to_owned()
    }
}

pub(crate) fn queue_label(queue_id: i64) -> String {
    match queue_id {
        MAYHEM_QUEUE_ID => "海克斯大乱斗".to_owned(),
        450 => "极地大乱斗".to_owned(),
        420 => "单双排位".to_owned(),
        440 => "灵活排位".to_owned(),
        430 => "匹配模式".to_owned(),
        480 => "快速模式".to_owned(),
        other => format!("队列 {other}"),
    }
}

pub(crate) fn phase_label(phase: &str) -> &str {
    match phase {
        "" => "读取中",
        "None" => "未进入对局",
        "Lobby" => "大厅",
        "Matchmaking" => "匹配中",
        "ReadyCheck" => "确认对局",
        "ChampSelect" => "英雄选择中",
        "GameStart" => "正在进入游戏",
        "InProgress" => "游戏中",
        "Reconnect" => "重新连接中",
        "WaitingForStats" => "结算中",
        "PreEndOfGame" => "对局结束",
        "EndOfGame" => "结算完成",
        "TerminatedInError" => "客户端异常",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_labels_are_localized() {
        assert_eq!(phase_label("ChampSelect"), "英雄选择中");
        assert_eq!(phase_label("GameStart"), "正在进入游戏");
        assert_eq!(phase_label("InProgress"), "游戏中");
        assert_eq!(phase_label("SomethingNew"), "SomethingNew");
    }

    #[test]
    fn mayhem_queue_has_a_friendly_name() {
        assert_eq!(queue_label(MAYHEM_QUEUE_ID), "海克斯大乱斗");
        assert_eq!(queue_label(450), "极地大乱斗");
        assert_eq!(queue_label(9999), "队列 9999");
    }

    #[test]
    fn champion_display_falls_back_to_the_numeric_id() {
        assert_eq!(champion_display(103, "阿狸"), "阿狸");
        assert_eq!(champion_display(103, ""), "英雄 #103");
    }
}
