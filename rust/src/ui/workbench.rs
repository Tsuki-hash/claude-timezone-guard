//! 桌面工作台。视图只维护选择状态；系统写入集中在明确的应用/恢复入口。
use super::*;

pub(super) fn blend(fg: egui::Color32, bg: egui::Color32, ratio: f32) -> egui::Color32 {
    let mix = |a: u8, b: u8| (a as f32 * (1.0 - ratio) + b as f32 * ratio).round() as u8;
    egui::Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

fn text(value: impl Into<String>, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(value).size(size).color(color)
}

fn card(p: Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(p.panel)
        .stroke(egui::Stroke::new(1.0_f32, p.line))
        .corner_radius(14)
        .inner_margin(20)
        .shadow(egui::epaint::Shadow {
            offset: [0, 3],
            blur: 12,
            spread: 0,
            color: egui::Color32::from_black_alpha(5),
        })
}

fn pill(ui: &mut egui::Ui, value: &str, ink: egui::Color32, p: Palette) {
    egui::Frame::NONE
        .fill(blend(ink, p.panel, 0.92))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(text(value, 11.0, ink));
        });
}

fn rule(ui: &mut egui::Ui, p: Palette) {
    let (r, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        r.x_range(),
        r.center().y,
        egui::Stroke::new(1.0_f32, p.line),
    );
}

#[derive(Clone, Copy)]
enum Icon {
    Globe,
    Scan,
    Log,
    Arrow,
    Backup,
}

fn icon(painter: &egui::Painter, center: egui::Pos2, kind: Icon, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.5_f32, color);
    let at = |x, y| center + egui::vec2(x, y);
    match kind {
        Icon::Globe => {
            painter.circle_stroke(center, 8.0, stroke);
            painter.add(egui::epaint::EllipseShape::stroke(
                center,
                egui::vec2(3.5, 8.0),
                stroke,
            ));
            painter.line_segment([at(-8.0, 0.0), at(8.0, 0.0)], stroke);
        }
        Icon::Scan => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                painter.line_segment([at(x * 3.0, y * 8.0), at(x * 8.0, y * 8.0)], stroke);
                painter.line_segment([at(x * 8.0, y * 8.0), at(x * 8.0, y * 3.0)], stroke);
            }
            painter.line_segment([at(-4.0, 0.0), at(4.0, 0.0)], stroke);
        }
        Icon::Log => {
            for y in [-6.0, 0.0, 6.0] {
                painter.circle_filled(at(-7.0, y), 1.2, color);
                painter.line_segment([at(-2.0, y), at(7.0, y)], stroke);
            }
        }
        Icon::Arrow => {
            painter.line_segment([at(-7.0, 0.0), at(7.0, 0.0)], stroke);
            painter.line_segment([at(2.0, -5.0), at(7.0, 0.0)], stroke);
            painter.line_segment([at(2.0, 5.0), at(7.0, 0.0)], stroke);
        }
        Icon::Backup => {
            painter.circle_stroke(center, 7.0, stroke);
            painter.line_segment([at(0.0, -4.0), at(0.0, 0.0)], stroke);
            painter.line_segment([at(0.0, 0.0), at(4.0, 2.0)], stroke);
        }
    }
}

fn nav_button(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    kind: Icon,
    p: Palette,
) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 44.0],
        egui::Button::new("")
            .fill(if selected {
                blend(p.accent, p.panel, 0.90)
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::NONE)
            .corner_radius(8),
    );
    let ink = if selected { p.accent } else { p.fg_dim };
    let r = response.rect;
    if response.hovered() && !selected {
        ui.painter().rect_filled(r, 8, p.well);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            r.shrink(2.0),
            7,
            egui::Stroke::new(1.5_f32, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    icon(
        ui.painter(),
        egui::pos2(r.left() + 22.0, r.center().y),
        kind,
        ink,
    );
    ui.painter().text(
        egui::pos2(r.left() + 44.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        ink,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(super) fn draw_shell(ctx: &egui::Context, app: &mut App, p: Palette) {
    egui::SidePanel::left("navigation")
        .exact_width(174.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(p.panel).inner_margin(16))
        .show(ctx, |ui| {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                ui.painter().rect_filled(r, 10, p.accent);
                icon(ui.painter(), r.center(), Icon::Globe, p.accent_ink);
                ui.vertical(|ui| {
                    ui.label(text("Claude", 19.0, p.fg).strong());
                    ui.label(text("环境助手", 11.0, p.fg_dim));
                });
            });
            ui.add_space(35.0);
            for (page, name, glyph) in [
                (Page::Home, "地区设置", Icon::Globe),
                (Page::Detail, "环境检测", Icon::Scan),
                (Page::Log, "操作记录", Icon::Log),
            ] {
                if nav_button(ui, app.page == page, name, glyph, p).clicked() {
                    app.page = page;
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.label(text("仅管理本机设置", 11.0, p.fg_mute));
                ui.label(text("不会更改网络出口 IP", 11.0, p.fg_mute));
                ui.add_space(14.0);
                let restore_hint = match &app.backup {
                    BackupState::Missing => "首次应用地区设置时会尝试创建备份".to_string(),
                    BackupState::Broken(e) => {
                        format!("备份无法读取：{e}。请在操作记录中查看详情。")
                    }
                    BackupState::Ready => "当前任务完成后可恢复原设置".to_string(),
                };
                if ui
                    .add_enabled(
                        app.backup_ready() && !app.busy,
                        egui::Button::new(text("恢复原设置", 12.0, p.fg_dim))
                            .min_size(egui::vec2(ui.available_width(), 34.0)),
                    )
                    .on_disabled_hover_text(restore_hint)
                    .clicked()
                {
                    app.restore_pending = true;
                }
                let (label, color) = match app.backup {
                    BackupState::Ready => ("原设置已备份", p.ok),
                    BackupState::Missing => ("尚未创建备份", p.fg_mute),
                    BackupState::Broken(_) => ("备份不可用", p.danger),
                };
                ui.horizontal(|ui| {
                    let (r, _) =
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                    icon(ui.painter(), r.center(), Icon::Backup, color);
                    ui.label(text(label, 12.0, color));
                });
                rule(ui, p);
            });
        });
    egui::TopBottomPanel::bottom("activity_bar")
        .frame(
            egui::Frame::NONE
                .fill(p.panel)
                .inner_margin(egui::Margin::symmetric(24, 10)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if app.busy {
                    ui.spinner();
                } else {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    let color = app
                        .log
                        .last()
                        .map(|(_, k)| log_color(*k, p))
                        .unwrap_or(p.fg_mute);
                    ui.painter().circle_filled(r.center(), 3.0, color);
                }
                let msg = if app.busy {
                    app.operation_label.as_str()
                } else {
                    app.log.last().map(|(s, _)| s.as_str()).unwrap_or("就绪")
                };
                ui.add(egui::Label::new(text(msg, 12.0, p.fg_dim)).truncate())
                    .on_hover_text(msg);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link(text("查看记录", 12.0, p.accent)).clicked() {
                        app.page = Page::Log;
                    }
                });
            });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.bg).inner_margin(24))
        .show(ctx, |ui| {
            page_header(ui, app, p);
            ui.add_space(18.0);
            egui::ScrollArea::vertical()
                .id_salt(("workspace", app.page as u8))
                .auto_shrink([false, false])
                .show(ui, |ui| match app.page {
                    Page::Home => draw_home(ui, app, p),
                    Page::Detail => draw_detail(ui, app, p),
                    Page::Log => draw_log(ui, app, p),
                });
        });
    if app.restore_pending {
        restore_dialog(ctx, app, p);
    }
}

fn page_header(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let (title, subtitle) = match app.page {
        Page::Home => ("地区设置", "为你的工作环境，选择合适的地区。"),
        Page::Detail => ("环境检测", "理解每一项读数，明确下一步该做什么。"),
        Page::Log => ("操作记录", "查看每一步变更，以及需要继续处理的事项。"),
    };
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(text(title, 27.0, p.fg).strong());
            ui.label(text(subtitle, 13.0, p.fg_dim));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(
                    !app.busy,
                    egui::Button::new(text("刷新状态", 12.0, p.fg_dim))
                        .min_size(egui::vec2(88.0, 34.0)),
                )
                .on_hover_text("重新读取本机环境 · F5")
                .clicked()
            {
                app.refresh_fingerprint();
            }
        });
    });
}

fn draw_home(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    overview(ui, app, p);
    ui.add_space(20.0);
    let width = ui.available_width();
    if width >= 740.0 {
        let preview_w = 304.0;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.allocate_ui_with_layout(
                egui::vec2(width - preview_w - 20.0, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    profile_grid(ui, app, p);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(preview_w, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    preview(ui, app, p);
                },
            );
        });
    } else {
        let previous = app.selected_profile;
        profile_grid(ui, app, p);
        ui.add_space(16.0);
        let response = preview(ui, app, p);
        if app.selected_profile != previous {
            response.scroll_to_me(Some(egui::Align::Center));
        }
    }
}

fn overview(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    let (score, _) = risk_score(&app.fp);
    let findings = app.fp.risk_items().iter().filter(|i| i.score > 0.0).count();
    card(p).inner_margin(14).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(66.0, 66.0), egui::Sense::hover());
            let painter = ui.painter_at(r);
            painter.circle_stroke(r.center(), 29.0, egui::Stroke::new(4.0_f32, p.well));
            let points: Vec<_> = (0..=64)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * score as f32 / 100.0 * i as f32 / 64.0;
                    r.center() + egui::vec2(a.cos(), a.sin()) * 29.0
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(4.0_f32, p.risk(score)),
            ));
            painter.text(
                r.center() + egui::vec2(0.0, -3.0),
                egui::Align2::CENTER_CENTER,
                score.to_string(),
                egui::FontId::monospace(26.0),
                p.fg,
            );
            painter.text(
                r.center() + egui::vec2(0.0, 17.0),
                egui::Align2::CENTER_CENTER,
                "/ 100",
                egui::FontId::proportional(10.0),
                p.fg_mute,
            );
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(text("本机环境概览", 14.0, p.fg).strong());
                    pill(
                        ui,
                        &format!("{findings} 项待检查"),
                        if findings > 0 { p.warn } else { p.ok },
                        p,
                    );
                });
                ui.label(text(
                    format!(
                        "当前时区  {}   ·   {}",
                        short_tz(&app.fp.tz_id),
                        app.fp.culture
                    ),
                    12.0,
                    p.fg_dim,
                ));
                if app.page == Page::Detail {
                    ui.label(text("本机规则估算 · 不包含出口 IP", 12.0, p.fg_mute));
                } else if ui.link(text("查看检测详情 →", 12.0, p.accent)).clicked() {
                    app.page = Page::Detail;
                }
            });
        });
    });
}

fn profile_name(key: Profile) -> &'static str {
    match key {
        Profile::Shanghai => "上海",
        _ => info(key).label,
    }
}

fn profile_english(key: Profile) -> (&'static str, &'static str) {
    match key {
        Profile::Singapore => ("SG", "Singapore"),
        Profile::California => ("US", "California"),
        Profile::Taipei => ("TW", "Taipei"),
        Profile::Tokyo => ("JP", "Tokyo"),
        Profile::NewYork => ("US", "New York"),
        Profile::Shanghai => ("CN", "Shanghai"),
    }
}

fn profile_grid(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    ui.horizontal(|ui| {
        ui.label(text("选择地区", 15.0, p.fg).strong());
        ui.label(text("6 个预设", 12.0, p.fg_mute));
    });
    ui.label(text(
        "选择后可在预览中核对，应用前不会更改设置。",
        12.0,
        p.fg_dim,
    ));
    ui.add_space(8.0);
    let width = (ui.available_width() - 12.0) / 2.0;
    for row in PROFILES.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for profile in row {
                profile_card(ui, app, width, profile.key, p);
            }
        });
        ui.add_space(4.0);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        icon(ui.painter(), r.center(), Icon::Backup, p.fg_mute);
        ui.label(text(
            "首次应用时备份原设置，之后可随时恢复。",
            11.0,
            p.fg_mute,
        ));
    });
}

fn profile_card(
    ui: &mut egui::Ui,
    app: &mut App,
    width: f32,
    key: Profile,
    p: Palette,
) -> egui::Response {
    let selected = app.selected_profile == key;
    let response = ui.add_enabled(
        !app.busy,
        egui::Button::new("")
            .min_size(egui::vec2(width, 108.0))
            .corner_radius(12)
            .fill(if selected {
                blend(p.accent, p.panel, 0.95)
            } else {
                p.panel
            })
            .stroke(egui::Stroke::new(
                if selected { 1.6_f32 } else { 1.0_f32 },
                if selected { p.accent } else { p.line },
            )),
    );
    let r = response.rect;
    let painter = ui.painter_at(r);
    if response.hovered() && !app.busy && !selected {
        painter.rect_filled(r.shrink(1.0), 11, blend(p.accent, p.panel, 0.975));
    }
    if response.has_focus() {
        painter.rect_stroke(
            r.shrink(3.0),
            9,
            egui::Stroke::new(1.5_f32, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    let (code, english) = profile_english(key);
    let c = egui::pos2(r.left() + 28.0, r.top() + 29.0);
    painter.rect_filled(
        egui::Rect::from_center_size(c, egui::vec2(30.0, 28.0)),
        7,
        if selected {
            blend(p.accent, p.panel, 0.87)
        } else {
            p.well
        },
    );
    painter.text(
        c,
        egui::Align2::CENTER_CENTER,
        code,
        egui::FontId::monospace(12.0),
        if selected { p.accent } else { p.fg_dim },
    );
    let radio = egui::pos2(r.right() - 19.0, r.top() + 24.0);
    painter.circle_stroke(
        radio,
        6.0,
        egui::Stroke::new(1.3_f32, if selected { p.accent } else { p.line }),
    );
    if selected {
        painter.circle_filled(radio, 3.0, p.accent);
    }
    painter.text(
        egui::pos2(r.left() + 14.0, r.top() + 55.0),
        egui::Align2::LEFT_CENTER,
        profile_name(key),
        egui::FontId::proportional(17.0),
        p.fg,
    );
    painter.text(
        egui::pos2(r.left() + 14.0, r.top() + 74.0),
        egui::Align2::LEFT_CENTER,
        english,
        egui::FontId::proportional(12.0),
        p.fg_mute,
    );
    let offset = target_utc_offset(key).local_minus_utc() / 3600;
    let current = app.fp.tz_id == info(key).tz;
    let caption = if current {
        format!("UTC{offset:+}   ·   当前时区")
    } else {
        format!("UTC{offset:+}   ·   {}", info(key).culture)
    };
    painter.text(
        egui::pos2(r.left() + 14.0, r.bottom() - 16.0),
        egui::Align2::LEFT_CENTER,
        caption,
        egui::FontId::proportional(11.0),
        if selected { p.accent } else { p.fg_dim },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            !app.busy,
            selected,
            profile_name(key),
        )
    });
    if response.clicked() && !app.busy {
        app.selected_profile = key;
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("{} · 点击预览", info(key).sub))
}

fn preview(ui: &mut egui::Ui, app: &mut App, p: Palette) -> egui::Response {
    let key = app.selected_profile;
    let target = info(key);
    card(p)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(text("变更预览", 15.0, p.fg).strong());
                pill(ui, "已选择", p.accent, p);
            });
            ui.add_space(7.0);
            time_bridge(ui, key, p);
            ui.add_space(10.0);
            preview_row(
                ui,
                "系统时区",
                &short_tz(&app.fp.tz_id),
                &short_tz(target.tz),
                p,
            );
            preview_row(ui, "区域格式", &app.fp.culture, target.culture, p);
            preview_row(
                ui,
                "浏览器首选语言",
                &language_label(&app.fp.browser_lang),
                &language_label(target.browser_lang),
                p,
            );
            ui.add_space(4.0);
            rule(ui, p);
            ui.add_space(4.0);
            let browsers_open = app.fp.chrome_running || app.fp.edge_running;
            ui.label(text(
                if browsers_open {
                    "检测到浏览器正在运行"
                } else {
                    "应用前，请完全退出浏览器"
                },
                12.0,
                if browsers_open { p.warn } else { p.fg_dim },
            ));
            ui.label(text(
                "应用后重启 Claude Code，使新时区生效。",
                11.0,
                p.fg_mute,
            ));
            ui.add_space(8.0);
            let label = if app.busy {
                app.operation_label.clone()
            } else {
                format!("应用 {} 设置", profile_name(key))
            };
            let button = egui::Button::new(text(label, 14.0, p.accent_ink).strong())
                .fill(p.accent)
                .stroke(egui::Stroke::NONE)
                .corner_radius(9)
                .min_size(egui::vec2(ui.available_width(), 44.0));
            if ui.add_enabled(!app.busy, button).clicked() {
                app.switch_to(key);
            }
            ui.add_space(2.0);
            ui.vertical_centered(|ui| {
                ui.label(text("仅更改本机设置，不改变出口 IP", 11.0, p.fg_mute));
            });
        })
        .response
}

fn time_bridge(ui: &mut egui::Ui, key: Profile, p: Palette) {
    let (r, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 100.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(r);
    painter.rect_filled(r, 10, p.well);
    let left = egui::pos2(r.left() + 18.0, r.top() + 20.0);
    painter.text(
        left,
        egui::Align2::LEFT_CENTER,
        "北京时间",
        egui::FontId::proportional(11.0),
        p.fg_dim,
    );
    painter.text(
        egui::pos2(r.right() - 18.0, left.y),
        egui::Align2::RIGHT_CENTER,
        profile_name(key),
        egui::FontId::proportional(11.0),
        p.accent,
    );
    let now = chrono::Utc::now();
    painter.text(
        left + egui::vec2(0.0, 29.0),
        egui::Align2::LEFT_CENTER,
        (now + chrono::Duration::hours(8))
            .format("%H:%M")
            .to_string(),
        egui::FontId::monospace(25.0),
        p.fg,
    );
    painter.text(
        egui::pos2(r.right() - 18.0, left.y + 29.0),
        egui::Align2::RIGHT_CENTER,
        now.with_timezone(&target_utc_offset(key))
            .format("%H:%M")
            .to_string(),
        egui::FontId::monospace(25.0),
        p.accent,
    );
    icon(
        &painter,
        egui::pos2(r.center().x, left.y + 29.0),
        Icon::Arrow,
        p.fg_mute,
    );
    // 二十四小时刻度：两个时区的小时位置与实时读数对应。
    for tick in 0..=24 {
        let x = r.left() + 18.0 + (r.width() - 36.0) * tick as f32 / 24.0;
        let h = if tick % 6 == 0 { 9.0 } else { 4.0 };
        painter.line_segment(
            [
                egui::pos2(x, r.top() + 71.0),
                egui::pos2(x, r.top() + 71.0 + h),
            ],
            egui::Stroke::new(1.0_f32, p.line),
        );
    }
    use chrono::Timelike;
    for (hour, minute, color) in [
        (
            (now + chrono::Duration::hours(8)).hour(),
            now.minute(),
            p.fg_dim,
        ),
        (
            now.with_timezone(&target_utc_offset(key)).hour(),
            now.minute(),
            p.accent,
        ),
    ] {
        let x = r.left() + 18.0 + (r.width() - 36.0) * (hour as f32 + minute as f32 / 60.0) / 24.0;
        painter.circle_filled(egui::pos2(x, r.top() + 72.0), 3.0, color);
    }
    let diff = target_utc_offset(key).local_minus_utc() / 3600 - 8;
    let description = if diff == 0 {
        "与北京时间一致 · 无时差".into()
    } else {
        format!("相对北京时间 {diff:+} 小时")
    };
    painter.text(
        egui::pos2(r.center().x, r.bottom() - 16.0),
        egui::Align2::CENTER_CENTER,
        description,
        egui::FontId::proportional(11.0),
        p.fg_dim,
    );
}

fn preview_row(ui: &mut egui::Ui, label: &str, before: &str, after: &str, p: Palette) {
    ui.label(text(label, 11.0, p.fg_mute));
    let description = if before == after {
        format!("{after} · 保持一致")
    } else {
        format!("{before} → {after}")
    };
    ui.add(egui::Label::new(text(description, 13.0, p.fg)).truncate())
        .on_hover_text(format!("当前：{before}\n目标：{after}"));
    ui.add_space(8.0);
}

fn language_label(value: &str) -> String {
    value
        .split(',')
        .take(1)
        .map(
            |part| match part.trim().split(';').next().unwrap_or_default() {
                "en-SG" => "英语（新加坡）",
                "en-US" => "英语（美国）",
                "en" => "英语",
                "zh-CN" => "简体中文",
                "zh-TW" => "繁体中文",
                "zh" => "中文",
                "ja-JP" => "日语",
                other => other,
            },
        )
        .collect::<Vec<_>>()
        .join("、")
}

fn draw_detail(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    overview(ui, app, p);
    ui.add_space(14.0);
    card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(text("检测项", 16.0, p.fg).strong());
        ui.label(text(
            "分数来自本机规则估算，不代表服务可用性或账号安全。",
            12.0,
            p.fg_dim,
        ));
        ui.add_space(6.0);
        let f = &app.fp;
        let rows = [
            (
                "中转地址",
                f.base_url.as_deref().unwrap_or("未设置"),
                "需自行检查服务地址与配置来源",
            ),
            ("系统时区", f.tz_id.as_str(), "可在地区设置中调整"),
            (
                "字体环境",
                &f.fonts_vendor
                    .iter()
                    .chain(f.fonts_extra.iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("、"),
                "仅检测，不自动删除字体",
            ),
            (
                "区域格式",
                f.culture.as_str(),
                "区域与浏览器语言可在地区设置中调整",
            ),
            (
                "NTP 校时",
                f.ntp_server.as_deref().unwrap_or("未配置"),
                "仅检测，请按需检查系统校时设置",
            ),
            (
                "国产浏览器",
                &f.cn_browsers.join("、"),
                "仅检测，不自动卸载应用",
            ),
        ];
        for ((name, value, advice), risk) in rows.into_iter().zip(f.risk_items()) {
            rule(ui, p);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(text(name, 14.0, p.fg).strong());
                pill(
                    ui,
                    if risk.score > 0.0 {
                        "待检查"
                    } else {
                        "未命中"
                    },
                    if risk.score > 0.0 { p.warn } else { p.ok },
                    p,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        text(
                            format!("{:.0} / {}", risk.score * risk.weight as f32, risk.weight),
                            12.0,
                            p.fg_mute,
                        )
                        .monospace(),
                    );
                });
            });
            ui.add(
                egui::Label::new(text(
                    if value.is_empty() {
                        "未检测到"
                    } else {
                        value
                    },
                    13.0,
                    p.fg_dim,
                ))
                .truncate(),
            )
            .on_hover_text(value);
            ui.label(text(advice, 11.0, p.fg_mute));
            ui.add_space(8.0);
        }
        egui::CollapsingHeader::new("更多系统读数").show(ui, |ui| {
            for (label, value) in [
                ("浏览器语言", &f.browser_lang),
                ("界面语言（只读）", &f.ui_langs),
                ("系统区域", &f.sys_locale),
                ("Geo ID", &f.geo_id),
                ("中转配置来源", &f.base_url_hint),
            ] {
                ui.label(text(label, 12.0, p.fg_mute));
                ui.add(egui::Label::new(text(value, 13.0, p.fg)).wrap());
            }
            ui.label(text(
                format!(
                    "分享配置文件前请隐藏密钥，例如 {}",
                    redact_secret("sk-example-0000000000000000")
                ),
                11.0,
                p.fg_mute,
            ));
        });
    });
    ui.add_space(14.0);
    remote_card(ui, app, p);
}

fn remote_card(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(text("出口 IP 估算", 16.0, p.fg).strong());
        ui.label(text(
            "由第三方公开接口提供，查询会向该服务暴露你的出口 IP。",
            12.0,
            p.fg_dim,
        ));
        if app.remote_busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(text("正在查询出口信息…", 13.0, p.fg_dim));
            });
        } else {
            match &app.remote {
                None => {
                    ui.label(text(
                        "尚未查询 · 本机设置不会改变网络出口。",
                        13.0,
                        p.fg_mute,
                    ));
                }
                Some(Ok(est)) => {
                    ui.label(text(est.headline(), 18.0, p.fg));
                    ui.label(text(est.geo_summary(), 12.0, p.fg_dim));
                }
                Some(Err(e)) => {
                    ui.label(text(format!("查询失败：{e}"), 13.0, p.danger));
                }
            }
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !app.remote_busy,
                    egui::Button::new(text(
                        if matches!(app.remote, Some(Err(_))) {
                            "重新查询"
                        } else {
                            "查询出口 IP"
                        },
                        13.0,
                        p.accent,
                    )),
                )
                .clicked()
            {
                app.request_remote();
            }
            if ui.button("打开独立检测页").clicked() {
                match webbrowser::open(DETECT_PAGE_URL) {
                    Ok(()) => app.push_log("已打开独立检测页".into(), LogKind::Info),
                    Err(e) => app.push_log(format!("打开检测页失败：{e}"), LogKind::Warn),
                }
            }
        });
    });
}

fn log_color(kind: LogKind, p: Palette) -> egui::Color32 {
    match kind {
        LogKind::Info => p.fg_mute,
        LogKind::Ok => p.ok,
        LogKind::Warn => p.warn,
        LogKind::Bad => p.danger,
    }
}

fn draw_log(ui: &mut egui::Ui, app: &mut App, p: Palette) {
    ui.horizontal(|ui| {
        for (value, label) in [(0, "全部记录"), (1, "待处理")] {
            let selected = app.log_filter == value;
            if ui
                .add(
                    egui::Button::new(text(
                        label,
                        13.0,
                        if selected { p.accent } else { p.fg_dim },
                    ))
                    .fill(if selected {
                        blend(p.accent, p.panel, 0.90)
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .stroke(egui::Stroke::NONE),
                )
                .clicked()
            {
                app.log_filter = value;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let copy_id = egui::Id::new("log_copy_feedback");
            let copied = ui
                .ctx()
                .data(|data| data.get_temp::<std::time::Instant>(copy_id))
                .is_some_and(|time| time.elapsed().as_secs() < 2);
            if ui
                .button(if copied {
                    "已复制"
                } else {
                    "复制全部记录"
                })
                .clicked()
            {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(copy_id, std::time::Instant::now()));
                ui.ctx().copy_text(
                    app.log
                        .iter()
                        .map(|(s, _)| s.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        });
    });
    ui.add_space(12.0);
    card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let mut shown = 0;
        for (message, kind) in app
            .log
            .iter()
            .rev()
            .filter(|(_, k)| app.log_filter == 0 || matches!(k, LogKind::Warn | LogKind::Bad))
        {
            if shown > 0 {
                rule(ui, p);
            }
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                let label = match kind {
                    LogKind::Info => "信息",
                    LogKind::Ok => "完成",
                    LogKind::Warn => "提示",
                    LogKind::Bad => "失败",
                };
                pill(ui, label, log_color(*kind, p), p);
                ui.add(egui::Label::new(text(message, 13.0, p.fg)).wrap());
            });
            ui.add_space(6.0);
            shown += 1;
        }
        if shown == 0 {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(text("暂无待处理记录", 18.0, p.fg));
                ui.label(text("操作中的提示与失败信息会显示在这里。", 13.0, p.fg_dim));
            });
            ui.add_space(40.0);
        }
    });
}

fn restore_dialog(ctx: &egui::Context, app: &mut App, p: Palette) {
    egui::Modal::new(egui::Id::new("restore_confirmation"))
        .frame(card(p))
        .show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(text("恢复原设置", 21.0, p.fg).strong());
            ui.add_space(8.0);
            ui.label(text(
                "将时区、区域格式与浏览器语言恢复到最初备份中的状态。",
                14.0,
                p.fg_dim,
            ));
            ui.label(text(
                "请先退出浏览器。恢复后仍会保留备份。",
                12.0,
                p.fg_mute,
            ));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    app.restore_pending = false;
                }
                if ui
                    .add_enabled(
                        !app.busy && app.backup_ready(),
                        egui::Button::new(text("恢复备份", 13.0, p.accent_ink)).fill(p.accent),
                    )
                    .clicked()
                {
                    app.restore_pending = false;
                    app.restore();
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 地区卡片只选择且忙碌时禁用() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        let initial_tz = app.fp.tz_id.clone();
        let log_count = app.log.len();
        let mut rect = egui::Rect::NOTHING;
        for busy in [false, true] {
            app.busy = busy;
            app.selected_profile = Profile::Singapore;
            for frame in 0..3 {
                let events = if frame == 0 {
                    vec![]
                } else {
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed: frame == 1,
                            modifiers: egui::Modifiers::default(),
                        },
                    ]
                };
                let _ = ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            rect = profile_card(
                                ui,
                                &mut app,
                                220.0,
                                Profile::California,
                                Palette::for_theme(Theme::Light),
                            )
                            .rect;
                        });
                    },
                );
            }
            assert_eq!(
                app.selected_profile,
                if busy {
                    Profile::Singapore
                } else {
                    Profile::California
                }
            );
            assert!(app.task_rx.is_none());
            assert_eq!(app.fp.tz_id, initial_tz);
            assert_eq!(app.log.len(), log_count);
        }
    }

    #[test]
    fn 各地区与深浅主题在窄屏和宽屏下不溢出() {
        let mut app = App::new();
        app.fp.base_url = Some("https://example.test/".to_owned() + &"long-path/".repeat(30));
        for theme in [Theme::Light, Theme::Dark] {
            let ctx = egui::Context::default();
            crate::setup_fonts(&ctx);
            apply_theme(&ctx, theme);
            app.theme = theme;
            for width in [MIN_WINDOW_SIZE[0] - 230.0, WINDOW_SIZE[0] - 230.0] {
                for profile in PROFILES {
                    app.selected_profile = profile.key;
                    let mut right = 0.0_f32;
                    let mut bottom = 0.0_f32;
                    for _ in 0..2 {
                        let _ = ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 2000.0),
                                )),
                                ..Default::default()
                            },
                            |ctx| {
                                egui::CentralPanel::default().show(ctx, |ui| {
                                    let content = ui.scope(|ui| {
                                        draw_home(ui, &mut app, Palette::for_theme(theme))
                                    });
                                    right = content.response.rect.right();
                                    bottom = content.response.rect.bottom();
                                });
                            },
                        );
                    }
                    if width > 740.0 {
                        assert!(bottom <= 630.0, "默认窗口内容过高：{bottom}");
                    }
                    assert!(
                        right <= width,
                        "{theme:?} {:?}: {right} > {width}",
                        profile.key
                    );
                }
            }
        }
    }
    #[test]
    fn 刷新完成和异常断开都释放操作锁() {
        let mut app = App::new();
        let mut fp = read_fingerprint();
        fp.culture = "test-refresh".into();
        let (tx, rx) = channel();
        app.busy = true;
        app.task_rx = Some(rx);
        tx.send(TaskResult::Refreshed {
            fp: Box::new(fp),
            backup: BackupState::Missing,
        })
        .unwrap();
        app.poll_tasks();
        assert!(!app.busy);
        assert!(app.task_rx.is_none());
        assert_eq!(app.fp.culture, "test-refresh");
        let (tx, rx) = channel::<TaskResult>();
        app.busy = true;
        app.task_rx = Some(rx);
        drop(tx);
        app.poll_tasks();
        assert!(!app.busy);
        assert!(app.task_rx.is_none());
        assert!(matches!(app.log.last(), Some((_, LogKind::Bad))));
    }
}
