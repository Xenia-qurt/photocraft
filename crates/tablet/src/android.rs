//! Обработка ввода стилуса и отклонения ладони (Palm Rejection) для Android.
//!
//! Модуль спроектирован так, что вся математика преобразования осей Android (`AXIS_TILT`,
//! `AXIS_ORIENTATION`, `AXIS_PRESSURE`, кнопки стилуса) и конечный автомат Palm Rejection
//! компилируются и тестируются на всех платформах без зависимости от Android NDK.
//! При сборке под `target_os = "android"` дополнительно доступен тонкий адаптер [`ndk_bridge`],
//! принимающий сырые значения из `AMotionEvent` / `GameActivity`.

use std::f32::consts::FRAC_PI_2;

use crate::{Sample, Update};

/// Значения `AMOTION_EVENT_TOOL_TYPE_*` из Android NDK (`android/input.h`).
pub mod tool_type {
    /// Неизвестный тип инструмента.
    pub const UNKNOWN: i32 = 0;
    /// Палец (ёмкостный тач).
    pub const FINGER: i32 = 1;
    /// Перо / стилус.
    pub const STYLUS: i32 = 2;
    /// Мышь или трекпад.
    pub const MOUSE: i32 = 3;
    /// Обратный конец стилуса (ластик).
    pub const ERASER: i32 = 4;
}

/// Маска и коды действий `AMOTION_EVENT_ACTION_*` из Android NDK (`android/input.h`).
pub mod action {
    /// Битовая маска самого действия без индекса указателя.
    pub const MASK: i32 = 0xff;
    /// Первое касание экрана.
    pub const DOWN: i32 = 0;
    /// Отрыв последнего указателя.
    pub const UP: i32 = 1;
    /// Перемещение при касании.
    pub const MOVE: i32 = 2;
    /// Отмена жеста системой.
    pub const CANCEL: i32 = 3;
    /// Касание дополнительного указателя.
    pub const POINTER_DOWN: i32 = 5;
    /// Отрыв дополнительного указателя.
    pub const POINTER_UP: i32 = 6;
    /// Наведение (hover) стилуса или мыши без касания.
    pub const HOVER_MOVE: i32 = 7;
    /// Вход стилуса в зону наведения.
    pub const HOVER_ENTER: i32 = 9;
    /// Выход стилуса из зоны наведения.
    pub const HOVER_EXIT: i32 = 10;
}

/// Битовые флаги кнопок `AMOTION_EVENT_BUTTON_*` из Android NDK (`android/input.h`).
pub mod button {
    /// Основная кнопка мыши.
    pub const PRIMARY: i32 = 1 << 0;
    /// Вторичная (правая) кнопка мыши.
    pub const SECONDARY: i32 = 1 << 1;
    /// Средняя кнопка мыши.
    pub const TERTIARY: i32 = 1 << 2;
    /// Основная боковая кнопка стилуса (`BUTTON_STYLUS_PRIMARY`).
    pub const STYLUS_PRIMARY: i32 = 1 << 5;
    /// Вторая боковая кнопка стилуса (`BUTTON_STYLUS_SECONDARY`).
    pub const STYLUS_SECONDARY: i32 = 1 << 6;
}

/// Окно удержания защиты от ладони после отрыва пера (мс).
pub const DEFAULT_PALM_GRACE_MS: u64 = 250;

/// Порог диаметра пятна касания (`AXIS_TOUCH_MAJOR`, px), выше которого касание пальцем
/// классифицируется как случайное прикосновение ладонью.
pub const DEFAULT_PALM_MAJOR_PX: f32 = 48.0;

/// Тип указателя Android MotionEvent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AndroidToolType {
    /// Неизвестный тип.
    #[default]
    Unknown,
    /// Палец.
    Finger,
    /// Стилус (основной наконечник).
    Stylus,
    /// Мышь или тачпад.
    Mouse,
    /// Ластик на обратной стороне стилуса.
    Eraser,
}

impl AndroidToolType {
    /// Преобразует сырой код `AMOTION_EVENT_TOOL_TYPE_*` в [`AndroidToolType`].
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            tool_type::FINGER => Self::Finger,
            tool_type::STYLUS => Self::Stylus,
            tool_type::MOUSE => Self::Mouse,
            tool_type::ERASER => Self::Eraser,
            _ => Self::Unknown,
        }
    }

    /// Возвращает `true`, если событие пришло от пера (наконечника или ластика).
    pub fn is_pen(self) -> bool {
        matches!(self, Self::Stylus | Self::Eraser)
    }
}

/// Тип действия Android MotionEvent (после применения [`action::MASK`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AndroidAction {
    /// Начало касания первым указателем.
    Down,
    /// Завершение касания последним указателем.
    Up,
    /// Перемещение во время касания.
    Move,
    /// Системная отмена касания.
    Cancel,
    /// Касание дополнительного указателя.
    PointerDown,
    /// Отрыв дополнительного указателя.
    PointerUp,
    /// Перемещение в воздухе (hover).
    HoverMove,
    /// Вход в зону наведения (hover).
    HoverEnter,
    /// Выход из зоны наведения (hover).
    HoverExit,
    /// Прочий необрабатываемый код действия.
    Other(i32),
}

impl AndroidAction {
    /// Извлекает [`AndroidAction`] из сырого поля `action` (`AMotionEvent_getAction`).
    pub fn from_raw(raw: i32) -> Self {
        match raw & action::MASK {
            action::DOWN => Self::Down,
            action::UP => Self::Up,
            action::MOVE => Self::Move,
            action::CANCEL => Self::Cancel,
            action::POINTER_DOWN => Self::PointerDown,
            action::POINTER_UP => Self::PointerUp,
            action::HOVER_MOVE => Self::HoverMove,
            action::HOVER_ENTER => Self::HoverEnter,
            action::HOVER_EXIT => Self::HoverExit,
            other => Self::Other(other),
        }
    }

    /// Возвращает `true`, если действие относится к наведению (без касания экрана).
    pub fn is_hover(self) -> bool {
        matches!(self, Self::HoverMove | Self::HoverEnter | Self::HoverExit)
    }
}

/// Состояние боковых кнопок стилуса.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AndroidStylusButtons {
    /// Нажата ли основная кнопка стилуса (`BUTTON_STYLUS_PRIMARY`).
    pub primary: bool,
    /// Нажата ли вторая кнопка стилуса (`BUTTON_STYLUS_SECONDARY`).
    pub secondary: bool,
}

impl AndroidStylusButtons {
    /// Разбирает битовую маску `AMotionEvent_getButtonState`.
    pub fn from_button_state(state: i32) -> Self {
        Self { primary: (state & button::STYLUS_PRIMARY) != 0, secondary: (state & button::STYLUS_SECONDARY) != 0 }
    }
}

/// Очищенные поля одного указателя из Android `MotionEvent`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AndroidMotionEvent {
    /// Стабильный идентификатор указателя в рамках жеста.
    pub pointer_id: i32,
    /// Тип инструмента (`FINGER`, `STYLUS`, `ERASER`, `MOUSE`).
    pub tool_type: AndroidToolType,
    /// Действие события.
    pub action: AndroidAction,
    /// Координата X в пикселях окна.
    pub x: f32,
    /// Координата Y в пикселях окна.
    pub y: f32,
    /// Сила нажатия `AXIS_PRESSURE` (обычно `0.0..=1.0`, может превышать `1.0` на ряде планшетов).
    pub pressure: f32,
    /// Угол наклона `AXIS_TILT` в радианах от нормали экрана (`0..=π/2`).
    pub tilt_rad: f32,
    /// Азимут наклона `AXIS_ORIENTATION` в радианах (`-π..=π`, `0` = вверх, `+π/2` = вправо).
    pub orientation_rad: f32,
    /// Расстояние наведения `AXIS_DISTANCE` (для hover стилуса).
    pub distance: f32,
    /// Большая ось эллипса касания `AXIS_TOUCH_MAJOR` в пикселях (используется для Palm Rejection).
    pub touch_major: f32,
    /// Сырая битовая маска кнопок `AMotionEvent_getButtonState`.
    pub button_state: i32,
    /// Временная метка события в миллисекундах (монотонные часы).
    pub timestamp_ms: u64,
}

impl Default for AndroidMotionEvent {
    fn default() -> Self {
        Self {
            pointer_id: 0,
            tool_type: AndroidToolType::Unknown,
            action: AndroidAction::Move,
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            tilt_rad: 0.0,
            orientation_rad: 0.0,
            distance: 0.0,
            touch_major: 0.0,
            button_state: 0,
            timestamp_ms: 0,
        }
    }
}

/// Результат обработки одного Android-события ввода через [`AndroidInputMapper`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AndroidInputResult {
    /// Обновление сэмпла пера для передачи в `StylusFeed`.
    pub sample_update: Update,
    /// Нужно ли отбросить это событие касания пальцем (сработал Palm Rejection).
    pub reject_touch: bool,
    /// Текущее состояние боковых кнопок стилуса.
    pub stylus_buttons: AndroidStylusButtons,
    /// Фронт нажатия основной кнопки стилуса (`false -> true` на этом событии).
    pub primary_just_pressed: bool,
    /// Фронт нажатия второй кнопки стилуса (`false -> true` на этом событии).
    pub secondary_just_pressed: bool,
}

/// Преобразует полярные координаты наклона Android (`AXIS_TILT`, `AXIS_ORIENTATION` в радианах)
/// в декартовы углы W3C Pointer Events `(tilt_x, tilt_y)` в градусах `[-90, 90]`.
///
/// Соглашение Android:
/// - `tilt_rad = 0` — перо перпендикулярно экрану, `π/2` — параллельно экрану;
/// - `orientation_rad = 0` — перо наклонено к верхнему краю экрана (`-Y`),
///   `+π/2` — вправо (`+X`), `±π` — к пользователю (`+Y`), `-π/2` — влево (`-X`).
///
/// Соглашение W3C / [`Sample`]:
/// - `+tilt_x` — наклон вправо;
/// - `+tilt_y` — наклон к пользователю (вниз по экрану).
pub fn tilt_from_polar(tilt_rad: f32, orientation_rad: f32) -> (f32, f32) {
    if !tilt_rad.is_finite() || !orientation_rad.is_finite() {
        return (0.0, 0.0);
    }
    let tilt = tilt_rad.clamp(0.0, FRAC_PI_2);
    if tilt <= f32::EPSILON {
        return (0.0, 0.0);
    }
    let sin_t = tilt.sin();
    let cos_t = tilt.cos();
    // Проекции единичного вектора корпуса пера на оси X (вправо) и Y (вниз, к пользователю).
    let vx = sin_t * orientation_rad.sin();
    let vy = -sin_t * orientation_rad.cos();
    let tilt_x = vx.atan2(cos_t).to_degrees().clamp(-90.0, 90.0);
    let tilt_y = vy.atan2(cos_t).to_degrees().clamp(-90.0, 90.0);
    (tilt_x, tilt_y)
}

/// Преобразует азимут `AXIS_ORIENTATION` в радианах в угол поворота `[0, 360)` по часовой стрелке.
pub fn rotation_from_orientation(orientation_rad: f32) -> f32 {
    if !orientation_rad.is_finite() {
        return 0.0;
    }
    let deg = orientation_rad.to_degrees().rem_euclid(360.0);
    if deg >= 360.0 { 0.0 } else { deg }
}

/// Состояние фильтра отклонения ладони (Palm Rejection).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PalmRejectionState {
    /// Стилус сейчас касается экрана.
    pub stylus_down: bool,
    /// Стилус находится в зоне наведения (hover).
    pub stylus_hovering: bool,
    /// Временная метка последней активности стилуса (мс).
    pub last_stylus_ms: Option<u64>,
    /// Длительность защитного окна после отрыва/выхода стилуса (мс).
    pub grace_ms: u64,
    /// Порог диаметра пятна `touch_major` (px), выше которого касание считается ладонью.
    pub palm_major_px: f32,
}

impl Default for PalmRejectionState {
    fn default() -> Self {
        Self { stylus_down: false, stylus_hovering: false, last_stylus_ms: None, grace_ms: DEFAULT_PALM_GRACE_MS, palm_major_px: DEFAULT_PALM_MAJOR_PX }
    }
}

impl PalmRejectionState {
    /// Создаёт состояние Palm Rejection с пользовательскими порогами.
    pub fn with_thresholds(grace_ms: u64, palm_major_px: f32) -> Self {
        Self {
            grace_ms,
            palm_major_px: if palm_major_px.is_finite() && palm_major_px > 0.0 { palm_major_px } else { DEFAULT_PALM_MAJOR_PX },
            ..Self::default()
        }
    }

    /// Обновляет состояние на основе события стилуса или ластика.
    pub fn observe_stylus(&mut self, action: AndroidAction, timestamp_ms: u64) {
        self.last_stylus_ms = Some(timestamp_ms);
        match action {
            AndroidAction::Down | AndroidAction::PointerDown | AndroidAction::Move => {
                self.stylus_down = true;
                self.stylus_hovering = false;
            }
            AndroidAction::HoverEnter | AndroidAction::HoverMove => {
                self.stylus_hovering = true;
            }
            AndroidAction::Up | AndroidAction::PointerUp | AndroidAction::Cancel => {
                self.stylus_down = false;
            }
            AndroidAction::HoverExit => {
                self.stylus_down = false;
                self.stylus_hovering = false;
            }
            AndroidAction::Other(_) => {}
        }
    }

    /// Определяет, нужно ли отбросить событие пальца как случайное касание ладонью.
    pub fn should_reject_finger(&self, ev: &AndroidMotionEvent, enabled: bool) -> bool {
        if !enabled || ev.tool_type != AndroidToolType::Finger {
            return false;
        }
        if self.stylus_down || self.stylus_hovering {
            return true;
        }
        if let Some(last) = self.last_stylus_ms
            && ev.timestamp_ms >= last
            && ev.timestamp_ms.saturating_sub(last) <= self.grace_ms
        {
            return true;
        }
        ev.touch_major.is_finite() && ev.touch_major >= self.palm_major_px
    }

    /// Возвращает `true`, если стилус активен прямо сейчас (касание или наведение).
    pub fn is_stylus_active(&self) -> bool {
        self.stylus_down || self.stylus_hovering
    }
}

/// Основной преобразователь событий Android `MotionEvent` в [`Sample`] и решение Palm Rejection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AndroidInputMapper {
    /// Фильтр защиты от случайных касаний ладонью.
    pub palm: PalmRejectionState,
    /// Предыдущее состояние боковых кнопок стилуса (для выделения фронта нажатия).
    last_buttons: AndroidStylusButtons,
}

impl AndroidInputMapper {
    /// Создаёт новый преобразователь с настройками Palm Rejection по умолчанию.
    pub fn new() -> Self {
        Self::default()
    }

    /// Обрабатывает одно событие [`AndroidMotionEvent`].
    ///
    /// - Для стилуса и ластика формирует нормализованный [`Sample`] с давлением, наклоном и поворотом.
    /// - Для мыши сбрасывает сэмпл пера (`Update::Set(None)`).
    /// - Для пальца проверяет Palm Rejection (если `palm_rejection == true`).
    pub fn process(&mut self, ev: &AndroidMotionEvent, palm_rejection: bool) -> AndroidInputResult {
        let buttons = AndroidStylusButtons::from_button_state(ev.button_state);
        let primary_just_pressed = buttons.primary && !self.last_buttons.primary;
        let secondary_just_pressed = buttons.secondary && !self.last_buttons.secondary;

        if ev.tool_type.is_pen() {
            self.palm.observe_stylus(ev.action, ev.timestamp_ms);
            self.last_buttons = buttons;

            let sample_update = match ev.action {
                AndroidAction::HoverExit => Update::Set(None),
                // При отрыве пера сохраняем последнее давление мазка, аналогично AppKit.
                AndroidAction::Up | AndroidAction::PointerUp | AndroidAction::Cancel => Update::Keep,
                _ => {
                    let (tilt_x, tilt_y) = tilt_from_polar(ev.tilt_rad, ev.orientation_rad);
                    let rotation = rotation_from_orientation(ev.orientation_rad);
                    let pressure = if ev.action.is_hover() { 0.0 } else { ev.pressure };
                    let sample = Sample { pressure, tilt_x, tilt_y, rotation, eraser: ev.tool_type == AndroidToolType::Eraser }.sanitized();
                    Update::Set(Some(sample))
                }
            };
            return AndroidInputResult { sample_update, reject_touch: false, stylus_buttons: buttons, primary_just_pressed, secondary_just_pressed };
        }

        if ev.tool_type == AndroidToolType::Mouse {
            self.last_buttons = AndroidStylusButtons::default();
            return AndroidInputResult {
                sample_update: Update::Set(None),
                reject_touch: false,
                stylus_buttons: AndroidStylusButtons::default(),
                primary_just_pressed: false,
                secondary_just_pressed: false,
            };
        }

        let reject_touch = self.palm.should_reject_finger(ev, palm_rejection);
        AndroidInputResult {
            sample_update: Update::Keep,
            reject_touch,
            stylus_buttons: self.last_buttons,
            primary_just_pressed: false,
            secondary_just_pressed: false,
        }
    }
}

/// Тонкий адаптер Android NDK / `GameActivity`, компилируемый только под `target_os = "android"`.
#[cfg(target_os = "android")]
pub mod ndk_bridge {
    use super::{AndroidAction, AndroidMotionEvent, AndroidToolType};

    /// Собирает [`AndroidMotionEvent`] из сырых полей указателя Android `MotionEvent`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_raw_pointer(
        pointer_id: i32,
        raw_tool_type: i32,
        raw_action: i32,
        x: f32,
        y: f32,
        pressure: f32,
        tilt_rad: f32,
        orientation_rad: f32,
        distance: f32,
        touch_major: f32,
        button_state: i32,
        timestamp_ms: u64,
    ) -> AndroidMotionEvent {
        AndroidMotionEvent {
            pointer_id,
            tool_type: AndroidToolType::from_raw(raw_tool_type),
            action: AndroidAction::from_raw(raw_action),
            x,
            y,
            pressure,
            tilt_rad,
            orientation_rad,
            distance,
            touch_major,
            button_state,
            timestamp_ms,
        }
    }
}

#[cfg(test)]
#[path = "android_tests.rs"]
mod tests;
