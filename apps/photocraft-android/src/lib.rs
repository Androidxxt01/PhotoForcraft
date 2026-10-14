use eframe as _;
use photocraft_engine as _;
use photocraft_ui_egui as _;

#[no_mangle]
fn android_main(_app: android_activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    log::info!("PhotoCraft Android: compilou");
}
