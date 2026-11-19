//! Выбор графического бэкенда и лимитов `wgpu` для Android (Vulkan → GLES 3.2 → GLES 3.1 → GLES 3.0 → CPU).
//!
//! Модуль содержит чистую платформенно-независимую логику планирования бэкендов и расчёта
//! лимитов устройства, чтобы вся цепочка fallback проверялась юнит-тестами на любой ОС.

use eframe::egui_wgpu;
use eframe::wgpu;
use photocraft_engine::prefs::GpuBackend;

use super::setup::timing_features;

/// Максимальная сторона текстурного тайла на мобильных GPU (Vulkan / GLES 3.1+).
pub const ANDROID_DEFAULT_TILE: u32 = 4096;

/// Консервативный размер тайла для старых драйверов GLES 3.0 и CPU-режима.
pub const ANDROID_LEGACY_TILE: u32 = 2048;

/// Параметры графического профиля для конкретного бэкенда Android.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AndroidGpuProfile {
    /// Выбранный вариант настройки `performance.gpuBackend`.
    pub backend: GpuBackend,
    /// Флаги бэкендов `wgpu`, передаваемые в `InstanceDescriptor`.
    pub wgpu_backends: wgpu::Backends,
    /// Рекомендуемая максимальная сторона тайла холста в пикселях.
    pub max_tile_side: u32,
    /// Разрешён ли 16-битный float-холст (`Rgba16Float`) при поддержке драйвером.
    pub allow_f16_canvas: bool,
    /// Минорная версия OpenGL ES 3.x (для GLES-профилей).
    pub gles_minor: Option<u8>,
}

/// Возвращает упорядоченную цепочку резервных бэкендов Android для предпочтения `pref`.
///
/// При `Auto` или `Vulkan` используется полная цепочка `Vulkan → GLES 3.2 → GLES 3.1 → GLES 3.0 → CPU`.
/// Недоступные на Android десктопные значения (`Dx12`, `Metal`) нормализуются к `Auto`.
pub fn android_backend_chain(pref: GpuBackend) -> Vec<GpuBackend> {
    use GpuBackend::*;
    match pref {
        Auto | Vulkan | Dx12 | Metal => vec![Vulkan, Gles32, Gles31, Gles30, Cpu],
        Gl | Gles32 => vec![Gles32, Gles31, Gles30, Cpu],
        Gles31 => vec![Gles31, Gles30, Cpu],
        Gles30 => vec![Gles30, Cpu],
        Cpu => vec![Cpu],
    }
}

/// Возвращает следующий более безопасный бэкенд Android после сбоя инициализации `failed`.
pub fn next_android_fallback(failed: GpuBackend) -> Option<GpuBackend> {
    use GpuBackend::*;
    match failed {
        Auto | Vulkan | Dx12 | Metal => Some(Gles32),
        Gl | Gles32 => Some(Gles31),
        Gles31 => Some(Gles30),
        Gles30 => Some(Cpu),
        Cpu => None,
    }
}

/// Строит профиль графических возможностей Android для указанного бэкенда.
pub fn android_profile(backend: GpuBackend) -> AndroidGpuProfile {
    use GpuBackend::*;
    match backend {
        Auto | Dx12 | Metal => AndroidGpuProfile {
            backend: Auto,
            wgpu_backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            max_tile_side: ANDROID_DEFAULT_TILE,
            allow_f16_canvas: true,
            gles_minor: None,
        },
        Vulkan => AndroidGpuProfile {
            backend: Vulkan,
            wgpu_backends: wgpu::Backends::VULKAN,
            max_tile_side: ANDROID_DEFAULT_TILE,
            allow_f16_canvas: true,
            gles_minor: None,
        },
        Gl | Gles32 => AndroidGpuProfile {
            backend: Gles32,
            wgpu_backends: wgpu::Backends::GL,
            max_tile_side: ANDROID_DEFAULT_TILE,
            allow_f16_canvas: true,
            gles_minor: Some(2),
        },
        Gles31 => AndroidGpuProfile {
            backend: Gles31,
            wgpu_backends: wgpu::Backends::GL,
            max_tile_side: ANDROID_DEFAULT_TILE,
            allow_f16_canvas: false,
            gles_minor: Some(1),
        },
        Gles30 => AndroidGpuProfile {
            backend: Gles30,
            wgpu_backends: wgpu::Backends::GL,
            max_tile_side: ANDROID_LEGACY_TILE,
            allow_f16_canvas: false,
            gles_minor: Some(0),
        },
        Cpu => {
            AndroidGpuProfile { backend: Cpu, wgpu_backends: wgpu::Backends::GL, max_tile_side: ANDROID_LEGACY_TILE, allow_f16_canvas: false, gles_minor: None }
        }
    }
}

/// Вычисляет безопасные лимиты `wgpu::Limits` для Android-адаптера.
///
/// Для профилей OpenGL ES используется база `downlevel_webgl2_defaults`, для Vulkan —
/// `downlevel_defaults`, затем все значения зажимаются реальными возможностями адаптера `supported`.
pub fn android_device_limits(backend: GpuBackend, supported: wgpu::Limits) -> wgpu::Limits {
    let profile = android_profile(backend);
    let is_gles = matches!(backend, GpuBackend::Gl | GpuBackend::Gles32 | GpuBackend::Gles31 | GpuBackend::Gles30 | GpuBackend::Cpu);
    let base = if is_gles { wgpu::Limits::downlevel_webgl2_defaults() } else { wgpu::Limits::downlevel_defaults() };
    let clamped = base.or_worse_values_from(&supported);
    let max_tex = supported.max_texture_dimension_2d.min(profile.max_tile_side.max(ANDROID_LEGACY_TILE));
    wgpu::Limits { max_texture_dimension_2d: max_tex.max(clamped.max_texture_dimension_2d), max_buffer_size: supported.max_buffer_size, ..clamped }
}

/// Формирует конфигурацию `egui_wgpu::WgpuSetup` для запуска на Android с учётом предпочтения `pref`.
pub fn android_wgpu_setup(pref: GpuBackend) -> egui_wgpu::WgpuSetup {
    let profile = android_profile(pref);
    let mut create = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    create.instance_descriptor.backends = profile.wgpu_backends;
    create.power_preference = if pref == GpuBackend::Cpu { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance };
    create.device_descriptor = std::sync::Arc::new(move |adapter| {
        let resolved_backend = match adapter.get_info().backend {
            wgpu::Backend::Vulkan => GpuBackend::Vulkan,
            wgpu::Backend::Gl => match pref {
                GpuBackend::Gles31 | GpuBackend::Gles30 | GpuBackend::Cpu => pref,
                _ => GpuBackend::Gles32,
            },
            _ => pref,
        };
        wgpu::DeviceDescriptor {
            label: Some("photocraft android wgpu device"),
            required_limits: android_device_limits(resolved_backend, adapter.limits()),
            required_features: timing_features(adapter),
            ..Default::default()
        }
    });
    egui_wgpu::WgpuSetup::CreateNew(create)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_chain_follows_vulkan_to_gles_to_cpu_order() {
        assert_eq!(
            android_backend_chain(GpuBackend::Auto),
            vec![GpuBackend::Vulkan, GpuBackend::Gles32, GpuBackend::Gles31, GpuBackend::Gles30, GpuBackend::Cpu]
        );
        assert_eq!(android_backend_chain(GpuBackend::Gles32), vec![GpuBackend::Gles32, GpuBackend::Gles31, GpuBackend::Gles30, GpuBackend::Cpu]);
        assert_eq!(android_backend_chain(GpuBackend::Gles31), vec![GpuBackend::Gles31, GpuBackend::Gles30, GpuBackend::Cpu]);
        assert_eq!(android_backend_chain(GpuBackend::Gles30), vec![GpuBackend::Gles30, GpuBackend::Cpu]);
        assert_eq!(android_backend_chain(GpuBackend::Cpu), vec![GpuBackend::Cpu]);
        // Десктопные бэкенды нормализуются к полной мобильной цепочке.
        assert_eq!(android_backend_chain(GpuBackend::Dx12), android_backend_chain(GpuBackend::Auto));
        assert_eq!(android_backend_chain(GpuBackend::Metal), android_backend_chain(GpuBackend::Auto));
    }

    #[test]
    fn next_fallback_terminates_at_cpu() {
        assert_eq!(next_android_fallback(GpuBackend::Vulkan), Some(GpuBackend::Gles32));
        assert_eq!(next_android_fallback(GpuBackend::Gles32), Some(GpuBackend::Gles31));
        assert_eq!(next_android_fallback(GpuBackend::Gles31), Some(GpuBackend::Gles30));
        assert_eq!(next_android_fallback(GpuBackend::Gles30), Some(GpuBackend::Cpu));
        assert_eq!(next_android_fallback(GpuBackend::Cpu), None);
    }

    #[test]
    fn android_profiles_and_limits_respect_gles_tiers() {
        let p_vk = android_profile(GpuBackend::Vulkan);
        assert_eq!(p_vk.wgpu_backends, wgpu::Backends::VULKAN);
        assert!(p_vk.allow_f16_canvas);
        assert_eq!(p_vk.max_tile_side, ANDROID_DEFAULT_TILE);

        let p_30 = android_profile(GpuBackend::Gles30);
        assert_eq!(p_30.wgpu_backends, wgpu::Backends::GL);
        assert!(!p_30.allow_f16_canvas);
        assert_eq!(p_30.max_tile_side, ANDROID_LEGACY_TILE);
        assert_eq!(p_30.gles_minor, Some(0));

        let supported = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::downlevel_defaults() };
        let limits_30 = android_device_limits(GpuBackend::Gles30, supported.clone());
        assert_eq!(limits_30.max_texture_dimension_2d, ANDROID_LEGACY_TILE);

        let limits_vk = android_device_limits(GpuBackend::Vulkan, supported);
        assert_eq!(limits_vk.max_texture_dimension_2d, ANDROID_DEFAULT_TILE);
    }
}
