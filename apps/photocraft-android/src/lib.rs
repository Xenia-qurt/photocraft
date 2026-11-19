//! Точка входа Android-приложения PhotoCraft (`GameActivity` + `winit` + `eframe` + `wgpu`).
//!
//! Архитектура сохраняет ядро (`photocraft-engine`, 69 инструментов, 500+ команд, форматы
//! `photocraft-io`) без изменений и подключает мобильные адаптеры:
//! - [`gpu`]: выбор бэкенда `Vulkan → GLES 3.2 → GLES 3.1 → GLES 3.0 → CPU` с откатом при сбое драйвера;
//! - [`input`]: стилус (`AMotionEvent` → `StylusFeed`), боковые кнопки и Palm Rejection;
//! - [`storage`]: работа с файлами строго через Android Storage Access Framework (SAF).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod gpu;
pub mod input;
pub mod storage;

use std::path::PathBuf;
use std::sync::Arc;

use photocraft_engine::Session;
use photocraft_engine::prefs::GpuBackend;
use photocraft_ui_egui::theme::AppTheme;
use photocraft_ui_egui::{PhotocraftApp, Services};

use crate::gpu::AndroidGpuLauncher;
use crate::input::AndroidInputBridge;
use crate::storage::SafBridge;

/// Конфигурация запуска Android-оболочки PhotoCraft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AndroidLaunchConfig {
    /// Приватная директория данных приложения (`AndroidApp::internal_data_path()`).
    pub internal_data_dir: Option<PathBuf>,
    /// Предпочитаемый графический бэкенд из настроек пользователя.
    pub preferred_backend: GpuBackend,
    /// Принудительно включить планшетный интерфейс (по умолчанию `true` на Android).
    pub force_tablet_ui: bool,
}

impl Default for AndroidLaunchConfig {
    fn default() -> Self {
        Self { internal_data_dir: None, preferred_backend: GpuBackend::Auto, force_tablet_ui: true }
    }
}

/// Состояние жизненного цикла Android Activity (pause / resume / low-memory).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AndroidLifecycleState {
    /// Приложение в данный момент свёрнуто (`onPause` / `Suspended`).
    pub paused: bool,
    /// Количество полученных системных сигналов нехватки памяти (`onTrimMemory` / `LowMemory`).
    pub low_memory_events: u32,
    /// Требуется ли принудительное автосохранение документов в ближайшем кадре.
    pub pending_emergency_autosave: bool,
}

impl AndroidLifecycleState {
    /// Фиксирует переход Activity в фон: запрашивает немедленное автосохранение.
    pub fn on_pause(&mut self) {
        self.paused = true;
        self.pending_emergency_autosave = true;
    }

    /// Фиксирует возвращение Activity на передний план.
    pub fn on_resume(&mut self) {
        self.paused = false;
    }

    /// Фиксирует сигнал нехватки памяти от Android (`onLowMemory` / `onTrimMemory`).
    pub fn on_low_memory(&mut self) {
        self.low_memory_events = self.low_memory_events.saturating_add(1);
        self.pending_emergency_autosave = true;
    }

    /// Снимает флаг отложенного автосохранения после его выполнения.
    pub fn take_emergency_autosave(&mut self) -> bool {
        std::mem::replace(&mut self.pending_emergency_autosave, false)
    }
}

/// Собирает [`Services`] для Android-приложения: все кодеки `photocraft-io` и SAF-мост.
pub fn build_services(saf: Arc<SafBridge>) -> Services {
    #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
    let mut services = Services {
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc, path, settings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        inbox: Some(saf.inbox()),
        place_inbox: Some(Arc::default()),
        ..Default::default()
    };

    #[cfg(target_os = "android")]
    {
        attach_android_saf_services(&mut services, saf);
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = saf;
    }

    services
}

/// Подключает SAF-диалоги открытия/сохранения и запись в `content://` URI на Android.
#[cfg(target_os = "android")]
fn attach_android_saf_services(services: &mut Services, saf: Arc<SafBridge>) {
    use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest};

    let saf_dialog = saf.clone();
    services.file_dialog = Some(Box::new(move |request, _parent, reply| match request {
        FileDialogRequest::Open { extensions, .. } => {
            let mimes = storage::supported_open_mime_types(extensions.as_deref());
            let req_id = saf_dialog.register_open_request(move |res| {
                let answer = res.map(|opened| FileDialogAnswer::Contents(opened.display_name, opened.bytes));
                reply.send(answer);
            });
            if let Err(err) = storage::jni_bridge::launch_open_document(req_id, &mimes) {
                saf_dialog.complete_open(req_id, None);
                log::error!("photocraft-android: ошибка запуска SAF OpenDocument: {err}");
            }
        }
        FileDialogRequest::Save { suggested } => {
            let mime = storage::mime_for_extension(&suggested);
            let req_id = saf_dialog.register_create_request(move |res| {
                let answer = res.map(|created| FileDialogAnswer::SaveTo(created.uri));
                reply.send(answer);
            });
            if let Err(err) = storage::jni_bridge::launch_create_document(req_id, &suggested, mime) {
                saf_dialog.complete_create(req_id, None);
                log::error!("photocraft-android: ошибка запуска SAF CreateDocument: {err}");
            }
        }
        FileDialogRequest::Folder { .. } => {
            reply.send(Some(FileDialogAnswer::Unavailable("Выбор произвольной папки недоступен в sandbox Android; используйте SAF".into())));
        }
    }));

    services.write = Some(Box::new(storage::jni_bridge::write_bytes_to_uri));
}

/// Оболочка `eframe::App` для Android: оборачивает [`PhotocraftApp`], мост стилуса и GPU-страж.
pub struct AndroidShell {
    app: Box<PhotocraftApp>,
    input_bridge: AndroidInputBridge,
    gpu_launcher: AndroidGpuLauncher,
    lifecycle: AndroidLifecycleState,
}

impl AndroidShell {
    /// Создаёт оболочку Android поверх уже настроенного `egui::Context`.
    pub fn new(ctx: egui::Context, config: &AndroidLaunchConfig, saf: Arc<SafBridge>, gpu_launcher: AndroidGpuLauncher) -> Self {
        PhotocraftApp::setup_context(&ctx, AppTheme::tablet_dark());
        let session = Session::new();
        let services = build_services(saf);
        let mut app = PhotocraftApp::with_context(ctx.clone(), session, services);
        app.set_theme(&ctx, AppTheme::tablet_dark());
        if config.force_tablet_ui {
            app.tablet_ui.force_tablet_mode = Some(true);
        }
        let input_bridge = AndroidInputBridge::new(app.stylus.feed.clone());
        Self { app, input_bridge, gpu_launcher, lifecycle: AndroidLifecycleState::default() }
    }

    /// Возвращает ссылку на мост ввода Android для передачи нативных `AMotionEvent`.
    pub fn input_bridge(&self) -> &AndroidInputBridge {
        &self.input_bridge
    }

    /// Возвращает изменяемую ссылку на состояние жизненного цикла Activity.
    pub fn lifecycle_mut(&mut self) -> &mut AndroidLifecycleState {
        &mut self.lifecycle
    }
}

impl eframe::App for AndroidShell {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Отсекаем дублирующие Pointer+Touch события на мобильных экранах.
        photocraft_ui_egui::touch_twins::drop_twins(ctx, raw_input);
        eframe::App::raw_input_hook(self.app.as_mut(), ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Применяем накопленные фронты боковых кнопок стилуса через единую командную шину.
        let touch_prefs = self.app.session.prefs().touch.clone();
        for press in self.input_bridge.drain_button_presses() {
            let cmds = self.app.touch_gestures.stylus_button_commands(press.primary, press.secondary, press.pos, &touch_prefs);
            for cmd in cmds {
                photocraft_ui_egui::touch_gestures::execute_gesture_command(&mut self.app, cmd);
            }
        }
        if self.lifecycle.take_emergency_autosave() {
            photocraft_ui_egui::prefs_ui::autosave_now(&mut self.app);
        }
        self.app.logic_frame(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui_frame(ui, frame);
        self.gpu_launcher.confirm_first_frame();
    }
}

/// Точка входа нативной библиотеки Android (`libphotocraft_android.so`), вызываемая `GameActivity`.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    init_minimal_logger();

    let data_dir = app.internal_data_path();
    let config = AndroidLaunchConfig { internal_data_dir: data_dir.clone(), preferred_backend: GpuBackend::Auto, force_tablet_ui: true };

    let saf = SafBridge::install_global(Arc::new(SafBridge::default()));
    let gpu_launcher = AndroidGpuLauncher::prepare(config.preferred_backend, data_dir.as_deref());
    let force_cpu = gpu_launcher.force_cpu_canvas();

    let mut native_options = eframe::NativeOptions {
        android_app: Some(app),
        system_font_fallback: false,
        persistence_path: data_dir.map(|d| d.join("ui.ron")),
        ..Default::default()
    };
    native_options.wgpu_options.wgpu_setup = gpu_launcher.begin_wgpu_setup();

    let run_result = eframe::run_native(
        "PhotoCraft",
        native_options,
        Box::new(move |cc| {
            let mut shell = AndroidShell::new(cc.egui_ctx.clone(), &config, saf, gpu_launcher);
            if let Some(rs) = cc.wgpu_render_state.clone()
                && !force_cpu
            {
                log::info!("photocraft-android: активен wgpu-бэкенд {:?}", rs.adapter.get_info().backend);
                shell.app.set_wgpu(rs);
            }
            Ok(Box::new(shell))
        }),
    );

    if let Err(err) = run_result {
        log::error!("photocraft-android: eframe завершился с ошибкой: {err}");
    }
}

/// Минимальный логгер без внешних зависимостей для Android-сборки.
#[cfg(target_os = "android")]
fn init_minimal_logger() {
    struct StderrLogger;
    impl log::Log for StderrLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("[PhotoCraft][{}] {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_launch_config_enables_tablet_ui_and_auto_backend() {
        let cfg = AndroidLaunchConfig::default();
        assert_eq!(cfg.preferred_backend, GpuBackend::Auto);
        assert!(cfg.force_tablet_ui);
        assert!(cfg.internal_data_dir.is_none());
    }

    #[test]
    fn lifecycle_triggers_emergency_autosave_on_pause_and_low_memory() {
        let mut lc = AndroidLifecycleState::default();
        assert!(!lc.paused);
        assert!(!lc.take_emergency_autosave());

        lc.on_pause();
        assert!(lc.paused);
        assert!(lc.take_emergency_autosave());
        assert!(!lc.take_emergency_autosave());

        lc.on_resume();
        assert!(!lc.paused);

        lc.on_low_memory();
        assert_eq!(lc.low_memory_events, 1);
        assert!(lc.take_emergency_autosave());
    }

    #[test]
    fn build_services_wires_codecs_and_saf_inbox() {
        let saf = Arc::new(SafBridge::default());
        let services = build_services(saf);
        assert!(services.import.is_some());
        assert!(services.export.is_some());
        assert!(services.inbox.is_some());
        assert!(services.place_inbox.is_some());
    }
}
