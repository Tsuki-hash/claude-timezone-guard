//! 仅 ui-preview feature 下编译的视觉验收入口；不写系统设置或主题偏好。
use super::*;

pub(super) struct PreviewCapture {
    path: std::path::PathBuf,
    started: std::time::Instant,
    requested: bool,
}

impl App {
    pub(super) fn configure_preview(&mut self) {
        if let Ok(path) = std::env::var("FP_PREVIEW_OUTPUT") {
            self.preview_capture = Some(PreviewCapture {
                path: path.into(),
                started: std::time::Instant::now(),
                requested: false,
            });
        }
        match std::env::var("FP_PREVIEW_THEME").as_deref() {
            Ok("dark") => self.theme = Theme::Dark,
            Ok("light") => self.theme = Theme::Light,
            _ => {}
        }
        match std::env::var("FP_PREVIEW_PAGE").as_deref() {
            Ok("detail") => self.page = Page::Detail,
            Ok("log") => self.page = Page::Log,
            Ok("restore") => self.restore_pending = true,
            _ => {}
        }
        if let Ok(profile) = std::env::var("FP_PREVIEW_PROFILE") {
            self.selected_profile = match profile.as_str() {
                "singapore" => Profile::Singapore,
                "california" => Profile::California,
                "tokyo" => Profile::Tokyo,
                _ => self.selected_profile,
            };
        }
    }
    pub(super) fn capture_preview(&mut self, ctx: &egui::Context) {
        let Some(capture) = &mut self.preview_capture else {
            return;
        };
        // 等待窗口、字体和模态框完成布局；截图来自真实渲染器。
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if !capture.requested && capture.started.elapsed().as_millis() >= 600 {
            capture.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(screenshot) = screenshot {
            let bytes: Vec<u8> = screenshot
                .pixels
                .iter()
                .flat_map(|color| color.to_array())
                .collect();
            if let Err(error) = image::save_buffer(
                &capture.path,
                &bytes,
                screenshot.width() as u32,
                screenshot.height() as u32,
                image::ColorType::Rgba8,
            ) {
                eprintln!("保存界面截图失败：{error}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if capture.started.elapsed().as_secs() > 15 {
            eprintln!("界面截图超时");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
