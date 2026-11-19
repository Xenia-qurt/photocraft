//! Мост ввода Android (`AMotionEvent` → `photocraft-tablet` → `photocraft-ui-egui`).
//!
//! Преобразует события пера, пальцев и мыши из [`AndroidMotionEvent`] в показания
//! [`StylusFeed`] (давление, наклон, вращение, ластик), фильтрует случайные касания
//! ладони (Palm Rejection) и буферизует нажатия боковых кнопок стилуса для обработки
//! в игровом кадре `egui`.

use std::sync::{Arc, Mutex, PoisonError};

use egui::Pos2;
use photocraft_tablet::Update;
use photocraft_tablet::android::{AndroidInputMapper, AndroidInputResult, AndroidMotionEvent};
use photocraft_ui_egui::stylus::{PenSample, StylusFeed};

/// Одно зафиксированное нажатие боковых кнопок стилуса (фронт нажатия).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingStylusButtonPress {
    /// Была ли нажата основная кнопка на корпусе стилуса (`BUTTON_STYLUS_PRIMARY`).
    pub primary: bool,
    /// Была ли нажата дополнительная кнопка на корпусе стилуса (`BUTTON_STYLUS_SECONDARY`).
    pub secondary: bool,
    /// Позиция указателя в логических точках экрана в момент нажатия.
    pub pos: Pos2,
}

/// Диагностические счётчики моста ввода Android.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputBridgeStats {
    /// Общее число обработанных событий.
    pub total_events: u64,
    /// Число событий стилуса и ластика.
    pub stylus_events: u64,
    /// Число пальцевых касаний, отклонённых системой Palm Rejection.
    pub rejected_palms: u64,
}

#[derive(Default)]
struct SharedInputState {
    mapper: AndroidInputMapper,
    button_queue: Vec<PendingStylusButtonPress>,
    stats: InputBridgeStats,
}

/// Потокобезопасный мост ввода Android, связывающий NDK-события с `StylusFeed` и очередью кнопок.
#[derive(Clone)]
pub struct AndroidInputBridge {
    feed: StylusFeed,
    state: Arc<Mutex<SharedInputState>>,
}

impl AndroidInputBridge {
    /// Создаёт новый мост ввода, публикующий показания стилуса в `feed`.
    pub fn new(feed: StylusFeed) -> Self {
        Self { feed, state: Arc::new(Mutex::new(SharedInputState::default())) }
    }

    /// Обрабатывает одно событие `AndroidMotionEvent` с учётом настройки `palm_rejection_enabled`
    /// и коэффициента масштабирования экрана `pixels_per_point`.
    ///
    /// Возвращает [`AndroidInputResult`]; если `reject_touch == true`, событие пальца следует
    /// отбросить до передачи в холст.
    pub fn handle_motion(&self, event: &AndroidMotionEvent, palm_rejection_enabled: bool, pixels_per_point: f32) -> AndroidInputResult {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let result = guard.mapper.process(event, palm_rejection_enabled);

        guard.stats.total_events = guard.stats.total_events.saturating_add(1);
        if result.reject_touch {
            guard.stats.rejected_palms = guard.stats.rejected_palms.saturating_add(1);
        }

        match result.sample_update {
            Update::Set(Some(sample)) => {
                guard.stats.stylus_events = guard.stats.stylus_events.saturating_add(1);
                self.feed.set(Some(PenSample {
                    pressure: sample.pressure,
                    tilt_x: sample.tilt_x,
                    tilt_y: sample.tilt_y,
                    rotation: sample.rotation,
                    eraser: sample.eraser,
                }));
            }
            Update::Set(None) => {
                self.feed.set(None);
            }
            Update::Keep => {}
        }

        if result.primary_just_pressed || result.secondary_just_pressed {
            let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
            guard.button_queue.push(PendingStylusButtonPress {
                primary: result.primary_just_pressed,
                secondary: result.secondary_just_pressed,
                pos: Pos2::new(event.x / scale, event.y / scale),
            });
        }

        result
    }

    /// Извлекает все накопленные фронты нажатий боковых кнопок стилуса для обработки в кадре UI.
    pub fn drain_button_presses(&self) -> Vec<PendingStylusButtonPress> {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut guard.button_queue)
    }

    /// Возвращает текущие диагностические счётчики ввода.
    pub fn stats(&self) -> InputBridgeStats {
        let guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.stats
    }

    /// Сбрасывает состояние маппера (например, при потере фокуса Activity в `onPause`).
    pub fn reset(&self) {
        let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        guard.mapper = AndroidInputMapper::new();
        guard.button_queue.clear();
        self.feed.set(None);
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    use photocraft_tablet::android::{AndroidAction, AndroidToolType, button};

    use super::*;

    #[test]
    fn stylus_motion_updates_feed_and_queues_button_press_in_logical_points() {
        let feed = StylusFeed::default();
        let bridge = AndroidInputBridge::new(feed.clone());

        let ev = AndroidMotionEvent {
            pointer_id: 1,
            tool_type: AndroidToolType::Stylus,
            action: AndroidAction::Down,
            x: 400.0,
            y: 600.0,
            pressure: 0.75,
            tilt_rad: FRAC_PI_4,
            orientation_rad: FRAC_PI_2,
            button_state: button::STYLUS_PRIMARY,
            timestamp_ms: 100,
            ..Default::default()
        };

        let res = bridge.handle_motion(&ev, true, 2.0);
        assert!(!res.reject_touch);

        let sample = feed.get().expect("сэмпл стилуса должен попасть в StylusFeed");
        assert!((sample.pressure - 0.75).abs() < 0.001);
        assert!((sample.tilt_x - 45.0).abs() < 0.1);
        assert!(!sample.eraser);

        let presses = bridge.drain_button_presses();
        assert_eq!(presses.len(), 1);
        assert!(presses[0].primary);
        assert!(!presses[0].secondary);
        assert_eq!(presses[0].pos, Pos2::new(200.0, 300.0));
        assert!(bridge.drain_button_presses().is_empty());
    }

    #[test]
    fn palm_rejection_increments_counter_and_reset_clears_feed() {
        let feed = StylusFeed::default();
        let bridge = AndroidInputBridge::new(feed.clone());

        let pen = AndroidMotionEvent {
            pointer_id: 1,
            tool_type: AndroidToolType::Stylus,
            action: AndroidAction::Down,
            pressure: 0.5,
            timestamp_ms: 200,
            ..Default::default()
        };
        let _ = bridge.handle_motion(&pen, true, 1.0);

        let palm = AndroidMotionEvent {
            pointer_id: 2,
            tool_type: AndroidToolType::Finger,
            action: AndroidAction::Down,
            touch_major: 56.0,
            timestamp_ms: 210,
            ..Default::default()
        };
        let res = bridge.handle_motion(&palm, true, 1.0);
        assert!(res.reject_touch);

        let stats = bridge.stats();
        assert_eq!(stats.total_events, 2);
        assert_eq!(stats.stylus_events, 1);
        assert_eq!(stats.rejected_palms, 1);

        bridge.reset();
        assert!(feed.get().is_none());
    }
}
