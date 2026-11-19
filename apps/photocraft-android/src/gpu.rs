//! Выбор графического бэкенда `wgpu` и защита от сбоев GPU-драйвера при запуске на Android.
//!
//! Опирается на платформенно-независимый модуль
//! `photocraft_ui_egui::gpu_canvas::android_gpu` и добавляет маркер незавершённого старта
//! в приватной директории приложения: если предыдущий запуск упал внутри драйвера
//! (например, на нестабильной реализации Vulkan), следующий запуск автоматически
//! переходит к более безопасному бэкенду из цепочки `Vulkan → GLES 3.2 → GLES 3.1 → GLES 3.0 → CPU`.

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui_wgpu;
use photocraft_engine::prefs::GpuBackend;
use photocraft_ui_egui::gpu_canvas::android_gpu::{self, AndroidGpuProfile};

/// Имя файла-маркера незавершённой инициализации GPU.
const GPU_MARKER_FILE: &str = "android_gpu_startup.marker";

/// Результат планирования запуска графического бэкенда на Android.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AndroidGpuPlan {
    /// Бэкенд, который будет использован в текущем запуске.
    pub selected: GpuBackend,
    /// Был ли выполнен автоматический откат после предыдущего падения драйвера.
    pub fell_back_after_crash: bool,
    /// Профиль возможностей для выбранного бэкенда.
    pub profile: AndroidGpuProfile,
    /// Полная оставшаяся цепочка резервных бэкендов.
    pub fallback_chain: Vec<GpuBackend>,
}

/// Управляет выбором GPU-бэкенда Android и файлом-маркером защиты от цикличных падений драйвера.
#[derive(Clone, Debug)]
pub struct AndroidGpuLauncher {
    marker_path: Option<PathBuf>,
    plan: AndroidGpuPlan,
    startup_confirmed: bool,
}

impl AndroidGpuLauncher {
    /// Создаёт планировщик GPU-запуска для предпочтения `preferred` и директории данных `data_dir`.
    ///
    /// Если в `data_dir` остался маркер от предыдущего незавершённого старта, выбирается
    /// следующий более безопасный бэкенд по цепочке `android_gpu::next_android_fallback`.
    pub fn prepare(preferred: GpuBackend, data_dir: Option<&Path>) -> Self {
        let marker_path = data_dir.map(|dir| dir.join(GPU_MARKER_FILE));
        let previous_crash = marker_path.as_deref().and_then(read_marker);

        let (selected, fell_back_after_crash) = match previous_crash {
            Some(crashed) => {
                let next = android_gpu::next_android_fallback(crashed).unwrap_or(GpuBackend::Cpu);
                (next, true)
            }
            None => {
                let chain = android_gpu::android_backend_chain(preferred);
                let first = chain.first().copied().unwrap_or(GpuBackend::Cpu);
                (first, false)
            }
        };

        let profile = android_gpu::android_profile(selected);
        let fallback_chain = android_gpu::android_backend_chain(selected);

        Self { marker_path, plan: AndroidGpuPlan { selected, fell_back_after_crash, profile, fallback_chain }, startup_confirmed: false }
    }

    /// Возвращает текущий план графического запуска.
    pub fn plan(&self) -> &AndroidGpuPlan {
        &self.plan
    }

    /// Возвращает `true`, если графический конвейер должен работать строго на CPU (без GPU-холста).
    pub fn force_cpu_canvas(&self) -> bool {
        self.plan.selected == GpuBackend::Cpu
    }

    /// Записывает маркер перед инициализацией `wgpu` и возвращает конфигурацию `WgpuSetup`.
    pub fn begin_wgpu_setup(&self) -> egui_wgpu::WgpuSetup {
        if let Some(path) = self.marker_path.as_deref()
            && let Err(err) = write_marker(path, self.plan.selected)
        {
            log::warn!("photocraft-android: не удалось записать GPU-маркер {}: {err}", path.display());
        }
        android_gpu::android_wgpu_setup(self.plan.selected)
    }

    /// Фиксирует успешную отрисовку первого кадра и удаляет маркер незавершённого старта.
    pub fn confirm_first_frame(&mut self) {
        if self.startup_confirmed {
            return;
        }
        self.startup_confirmed = true;
        if let Some(path) = self.marker_path.as_deref()
            && path.exists()
            && let Err(err) = fs::remove_file(path)
        {
            log::warn!("photocraft-android: не удалось удалить GPU-маркер {}: {err}", path.display());
        }
    }

    /// Возвращает `true`, если первый кадр уже был успешно подтверждён.
    pub fn is_confirmed(&self) -> bool {
        self.startup_confirmed
    }
}

/// Сериализует бэкенд в короткий текстовый токен для файла-маркера.
fn backend_token(backend: GpuBackend) -> &'static str {
    match backend {
        GpuBackend::Auto | GpuBackend::Vulkan | GpuBackend::Dx12 | GpuBackend::Metal => "vulkan",
        GpuBackend::Gl | GpuBackend::Gles32 => "gles32",
        GpuBackend::Gles31 => "gles31",
        GpuBackend::Gles30 => "gles30",
        GpuBackend::Cpu => "cpu",
    }
}

/// Разбирает текстовый токен файла-маркера обратно в `GpuBackend`.
fn parse_backend_token(raw: &str) -> Option<GpuBackend> {
    match raw.trim() {
        "vulkan" => Some(GpuBackend::Vulkan),
        "gles32" => Some(GpuBackend::Gles32),
        "gles31" => Some(GpuBackend::Gles31),
        "gles30" => Some(GpuBackend::Gles30),
        "cpu" => Some(GpuBackend::Cpu),
        _ => None,
    }
}

fn read_marker(path: &Path) -> Option<GpuBackend> {
    let text = fs::read_to_string(path).ok()?;
    parse_backend_token(&text)
}

fn write_marker(path: &Path, backend: GpuBackend) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, backend_token(backend))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("photocraft_android_gpu_{label}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("создание временной директории");
        dir
    }

    #[test]
    fn clean_start_uses_preferred_backend_and_clears_marker_on_confirm() {
        let dir = temp_dir("clean");
        let mut launcher = AndroidGpuLauncher::prepare(GpuBackend::Auto, Some(&dir));
        assert_eq!(launcher.plan().selected, GpuBackend::Vulkan);
        assert!(!launcher.plan().fell_back_after_crash);
        assert!(!launcher.force_cpu_canvas());

        let _setup = launcher.begin_wgpu_setup();
        assert!(dir.join(GPU_MARKER_FILE).exists(), "маркер должен создаваться до старта wgpu");

        launcher.confirm_first_frame();
        assert!(launcher.is_confirmed());
        assert!(!dir.join(GPU_MARKER_FILE).exists(), "маркер должен удаляться после первого кадра");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_crashes_step_down_through_entire_fallback_chain_to_cpu() {
        let dir = temp_dir("fallback");

        // Симулируем падения драйвера подряд: Vulkan -> GLES 3.2 -> GLES 3.1 -> GLES 3.0 -> CPU.
        let expected = [GpuBackend::Vulkan, GpuBackend::Gles32, GpuBackend::Gles31, GpuBackend::Gles30, GpuBackend::Cpu, GpuBackend::Cpu];

        for (step, &want) in expected.iter().enumerate() {
            let launcher = AndroidGpuLauncher::prepare(GpuBackend::Auto, Some(&dir));
            assert_eq!(launcher.plan().selected, want, "неверный бэкенд на шаге {step}");
            assert_eq!(launcher.plan().fell_back_after_crash, step > 0);
            let _ = launcher.begin_wgpu_setup();
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
