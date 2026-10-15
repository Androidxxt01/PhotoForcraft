use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services};

const DOWNLOAD: &str = "/storage/emulated/0/Download";
const START_ZOOM: f32 = 0.5;

static CTX: OnceLock<egui::Context> = OnceLock::new();

struct Pending {
    multiple: bool,
    reply: Box<dyn FnOnce(Option<FileDialogAnswer>)>,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

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
        file_dialog: Some(Box::new(|request, _parent, reply| match request {
            FileDialogRequest::Open { multiple, .. } => {
                PENDING.with(|p| {
                    *p.borrow_mut() = Some(Pending {
                        multiple,
                        reply: Box::new(move |a| {
                            reply.send(a);
                        }),
                    })
                });
                if let Some(c) = CTX.get() {
                    c.request_repaint();
                }
            }
            FileDialogRequest::Folder { .. } => {
                reply.send(None);
            }
            FileDialogRequest::Save { suggested } => {
                let name = Path::new(&suggested)
                    .file_name()
                    .map_or_else(|| suggested.clone(), |n| n.to_string_lossy().to_string());
                reply.send(Some(FileDialogAnswer::SaveTo(format!("{DOWNLOAD}/{name}"))));
            }
        })),
        write: Some(Box::new(|path: &str, bytes: &[u8]| {
            std::fs::write(path, bytes).map_err(|e| e.to_string())
        })),
        ..Default::default()
    }
}

/// Abre um link no navegador do Android (Intent ACTION_VIEW via JNI).
fn open_url(url: &str) -> jni::errors::Result<()> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast())? };
    let activity = unsafe { jni::objects::JObject::from_raw(ctx.context().cast()) };
    let mut env = vm.attach_current_thread()?;
    let jurl = env.new_string(url)?;
    let uri = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[(&jurl).into()],
        )?
        .l()?;
    let action = env.new_string("android.intent.action.VIEW")?;
    let intent = env.new_object(
        "android/content/Intent",
        "(Ljava/lang/String;Landroid/net/Uri;)V",
        &[(&action).into(), (&uri).into()],
    )?;
    env.call_method(
        &activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[(&intent).into()],
    )?;
    Ok(())
}

fn forward_links(ctx: &egui::Context) {
    let urls: Vec<String> = ctx.output_mut(|o| {
        let mut urls = Vec::new();
        o.commands.retain(|c| {
            if let egui::OutputCommand::OpenUrl(u) = c {
                urls.push(u.url.clone());
                false
            } else {
                true
            }
        });
        urls
    });
    for url in urls {
        if let Err(e) = open_url(&url) {
            log::error!("nao abriu o link {url}: {e}");
        }
    }
}

fn big(text: impl Into<String>, s: f32) -> egui::RichText {
    egui::RichText::new(text).size(22.0 * s)
}

fn read_entries(dir: &Path) -> std::io::Result<Vec<(PathBuf, bool, String)>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        v.push((e.path(), is_dir, name));
    }
    v.sort_by(|a, b| (!a.1, a.2.to_lowercase()).cmp(&(!b.1, b.2.to_lowercase())));
    Ok(v)
}

struct Shell {
    app: PhotocraftApp,
    zoom: f32,
    dir: PathBuf,
    error: Option<String>,
}

impl Shell {
    fn zoom_buttons(&mut self, ctx: &egui::Context) {
        let s = 1.0 / self.zoom;
        egui::Area::new(egui::Id::new("zoom_ctl"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0 * s, -8.0 * s))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let size = egui::vec2(34.0 * s, 34.0 * s);
                    if ui.add(egui::Button::new(big("A-", s * 0.7)).min_size(size)).clicked() {
                        self.zoom = (self.zoom - 0.1).max(0.3);
                    }
                    if ui.add(egui::Button::new(big("A+", s * 0.7)).min_size(size)).clicked() {
                        self.zoom = (self.zoom + 0.1).min(1.5);
                    }
                });
            });
    }

    fn file_browser(&mut self, ctx: &egui::Context) {
        let multiple = match PENDING.with(|p| p.borrow().as_ref().map(|x| x.multiple)) {
            Some(m) => m,
            None => return,
        };
        let s = 1.0 / self.zoom;
        let rect = ctx.content_rect();
        let mut go: Option<PathBuf> = None;
        let mut chosen: Option<(PathBuf, String)> = None;
        let mut cancel = false;
        let dir = self.dir.clone();
        let error = self.error.clone();

        egui::Window::new(big("Abrir arquivo", s))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .fixed_size(egui::vec2(rect.width() * 0.92, rect.height() * 0.8))
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(big(dir.display().to_string(), s * 0.8));
                if let Some(e) = &error {
                    ui.colored_label(egui::Color32::LIGHT_RED, big(e.clone(), s * 0.8));
                }
                let w = ui.available_width();
                let up = ui.add(
                    egui::Button::new(big(".. (pasta acima)", s))
                        .min_size(egui::vec2(w, 46.0 * s)),
                );
                if up.clicked() {
                    if let Some(p) = dir.parent() {
                        go = Some(p.to_path_buf());
                    }
                }
                match read_entries(&dir) {
                    Err(e) => {
                        ui.label(big(
                            format!(
                                "Sem acesso: {e}\n\nAtive 'Acesso a todos os arquivos' para o PhotoCraft nas configuracoes do Android e abra de novo."
                            ),
                            s * 0.8,
                        ));
                    }
                    Ok(entries) => {
                        egui::ScrollArea::vertical()
                            .max_height(rect.height() * 0.5)
                            .show(ui, |ui| {
                                for (path, is_dir, name) in entries {
                                    let label =
                                        if is_dir { format!("{name}/") } else { name.clone() };
                                    let b = ui.add(
                                        egui::Button::new(big(label, s))
                                            .min_size(egui::vec2(w, 46.0 * s)),
                                    );
                                    if b.clicked() {
                                        if is_dir {
                                            go = Some(path);
                                        } else {
                                            chosen = Some((path, name));
                                        }
                                    }
                                }
                            });
                    }
                }
                let c = ui.add(
                    egui::Button::new(big("Cancelar", s)).min_size(egui::vec2(w, 46.0 * s)),
                );
                if c.clicked() {
                    cancel = true;
                }
            });

        if let Some(p) = go {
            self.dir = p;
            self.error = None;
        }
        if cancel {
            if let Some(p) = PENDING.with(|p| p.borrow_mut().take()) {
                (p.reply)(None);
            }
        }
        if let Some((path, name)) = chosen {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if let Some(p) = PENDING.with(|p| p.borrow_mut().take()) {
                        let answer = if multiple {
                            FileDialogAnswer::ContentsMany(vec![(name, bytes)])
                        } else {
                            FileDialogAnswer::Contents(name, bytes)
                        };
                        (p.reply)(Some(answer));
                    }
                }
                Err(e) => self.error = Some(format!("Nao consegui ler: {e}")),
            }
        }
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.set_zoom_factor(self.zoom);
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let ctx = ui.ctx().clone();
        self.zoom_buttons(&ctx);
        self.file_browser(&ctx);
        forward_links(&ctx);
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
            let _ = CTX.set(cc.egui_ctx.clone());
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let mut app = PhotocraftApp::new(Session::new(), services());
            app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.set_wgpu(rs);
            }
            Ok(Box::new(Shell {
                app,
                zoom: START_ZOOM,
                dir: PathBuf::from(DOWNLOAD),
                error: None,
            }))
        }),
    );
    if let Err(e) = result {
        log::error!("PhotoCraft saiu com erro: {e}");
    }
}
