use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{PhotocraftApp, Services};

fn services() -> Services {
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], depth: usize| {
            photocraft_io::import_with_svg_group_depth(name, bytes, depth)
                .map(|r| (r.document, r.warnings))
                .map_err(|e| e.to_string())
        })),
        export: Some(Box::new(
            |doc: &Document, path: &str, s: &photocraft_ui_egui::ExportSettings| {
                photocraft_io::export(doc, path, &s.io_options())
                    .map(|r| (r.bytes, r.warnings))
                    .map_err(|e| e.to_string())
            },
        )),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec())
                .map_err(|e| e.to_string())?;
            photocraft_codecs::encode(
                &img,
                photocraft_codecs::Format::Png,
                &EncodeOptions::default(),
            )
            .map_err(|e| e.to_string())
        })),
        ..Default::default()
    }
}

#[no_mangle]
fn android_main(android_app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    let mut options = eframe::NativeOptions::default();
    options.android_app = Some(android_app);
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);

    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let mut app = PhotocraftApp::new(Session::new(), services());
            app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.set_wgpu(rs);
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        log::error!("PhotoCraft saiu com erro: {e}");
    }
}
