//! Распознаватель мультитач-жестов и обработка кнопок стилуса для планшетов и Android.
//!
//! Поддерживаемые жесты (согласованы с настройками [`TouchPrefs`]):
//! - **2 пальца (pinch / pan / rotate)**: одновременное масштабирование, сдвиг и поворот холста
//!   при включённом `prefs.touch.pinch_zoom_rotate`.
//! - **Тап 3 пальцами**: `prefs.touch.three_finger_tap` (по умолчанию `Undo` → команда `edit.undo`).
//! - **Тап 4 пальцами**: `prefs.touch.four_finger_tap` (по умолчанию `Redo` → команда `edit.redo`).
//! - **Свайп 3 пальцами вниз**: `prefs.touch.three_finger_swipe_down` (по умолчанию режим
//!   **«Только холст»** — `TouchGestureAction::CanvasOnly`). Направленный свайп вниз гарантированно
//!   не конфликтует с коротким тапом 3 пальцами (`Undo`).
//! - **Долгое нажатие 1 пальцем**: удержание без смещения дольше `prefs.touch.long_press_ms`
//!   открывает контекстное меню / пипетку.
//! - **Защита от ладони и рисование пальцем**: учитываются `prefs.touch.palm_rejection` и
//!   `prefs.touch.finger_draws`.
//! - **Боковые кнопки стилуса**: `prefs.touch.stylus_primary_button` и
//!   `prefs.touch.stylus_secondary_button` преобразуются в команды PhotoCraft.

use egui::{Event, Pos2, TouchId, TouchPhase, Vec2};
use photocraft_engine::prefs::{StylusButtonAction, TouchGestureAction, TouchPrefs};
use serde_json::json;

/// Максимальное число одновременно отслеживаемых касаний.
const MAX_CONTACTS: usize = 10;
/// Максимальная длительность мультитап-жеста (в секундах).
const MULTI_TAP_MAX_SEC: f64 = 0.35;
/// Максимальное допустимое смещение пальца при тапе (в логических точках egui).
const TAP_MAX_DRIFT_PT: f32 = 24.0;
/// Минимальное вертикальное смещение вниз для свайпа 3 пальцами (в логических точках egui).
const THREE_FINGER_SWIPE_MIN_Y_PT: f32 = 72.0;
/// Порог смещения для отмены долгого нажатия (в логических точках egui).
const LONG_PRESS_MAX_DRIFT_PT: f32 = 16.0;
/// Минимальный и максимальный допустимый масштаб холста при щипке.
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 64.0;
/// Минимальное расстояние между двумя пальцами для стабильного расчёта масштаба и поворота.
const MIN_PINCH_DIST_PT: f32 = 8.0;

/// Команда, которую сгенерировал распознаватель жестов или кнопка стилуса.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureCommand {
    /// Команда единого реестра движка PhotoCraft (500+ команд).
    Engine(&'static str),
    /// Переключить режим «Только холст» (скрытие/возврат панелей).
    ToggleCanvasOnly,
    /// Открыть быстрое меню в указанной точке экрана.
    OpenQuickMenu(Pos2),
    /// Эмуляция вторичного клика (контекстного меню) в точке экрана.
    SecondaryClick(Pos2),
    /// Временное панорамирование холста (рука).
    PanHold,
}

/// Инкрементальное преобразование вида холста от двухпальцевого жеста.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoFingerDelta {
    /// Сдвиг центра между двумя пальцами (в точках egui).
    pub pan: Vec2,
    /// Относительный множитель масштаба (`1.0` — без изменений).
    pub zoom_factor: f32,
    /// Приращение угла поворота в градусах (по часовой стрелке).
    pub rotation_deg: f32,
    /// Текущий центр жеста в координатах экрана.
    pub center: Pos2,
}

/// Результат обработки сенсорных событий одного кадра.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GestureFrameOutput {
    /// Сдвиг/масштаб/поворот от двух пальцев (если активны ровно 2 касания и опция включена).
    pub two_finger: Option<TwoFingerDelta>,
    /// Команды, сработавшие в этом кадре (тапы 3/4 пальцами, свайп 3 пальцами вниз).
    pub triggered: Vec<GestureCommand>,
    /// Позиция сработавшего долгого нажатия 1 пальцем.
    pub long_press: Option<Pos2>,
    /// Нужно ли заблокировать рисование пальцем в этом кадре (активен мультитач, Palm Rejection или `finger_draws == false`).
    pub block_finger_draw: bool,
}

/// Преобразует настройку [`TouchGestureAction`] в конкретную [`GestureCommand`].
pub fn gesture_action_to_command(action: TouchGestureAction, pos: Pos2) -> Option<GestureCommand> {
    match action {
        TouchGestureAction::None => None,
        TouchGestureAction::Undo => Some(GestureCommand::Engine("edit.undo")),
        TouchGestureAction::Redo => Some(GestureCommand::Engine("edit.redo")),
        TouchGestureAction::CanvasOnly => Some(GestureCommand::ToggleCanvasOnly),
        TouchGestureAction::QuickMenu => Some(GestureCommand::OpenQuickMenu(pos)),
        TouchGestureAction::ResetView => Some(GestureCommand::Engine("view.fitOnScreen")),
    }
}

/// Преобразует настройку боковой кнопки стилуса [`StylusButtonAction`] в [`GestureCommand`].
pub fn stylus_button_to_command(action: StylusButtonAction, pos: Pos2) -> Option<GestureCommand> {
    match action {
        StylusButtonAction::None => None,
        StylusButtonAction::Eraser => Some(GestureCommand::Engine("tools.select.eraser")),
        StylusButtonAction::Eyedropper => Some(GestureCommand::Engine("tools.select.eyedropper")),
        StylusButtonAction::Undo => Some(GestureCommand::Engine("edit.undo")),
        StylusButtonAction::Redo => Some(GestureCommand::Engine("edit.redo")),
        StylusButtonAction::QuickMenu => Some(GestureCommand::OpenQuickMenu(pos)),
        StylusButtonAction::Pan => Some(GestureCommand::PanHold),
        StylusButtonAction::SecondaryClick => Some(GestureCommand::SecondaryClick(pos)),
    }
}

#[derive(Clone, Copy, Debug)]
struct TouchPoint {
    id: TouchId,
    start_pos: Pos2,
    pos: Pos2,
    start_time: f64,
    max_drift: f32,
    is_stylus: bool,
}

#[derive(Clone, Copy, Debug)]
struct TwoFingerSnapshot {
    center: Pos2,
    distance: f32,
    angle: f32,
}

/// Конечный автомат распознавания мультитач-жестов и долгого нажатия.
#[derive(Clone, Debug, Default)]
pub struct TouchGestureEngine {
    /// Текущие активные точки касания.
    contacts: Vec<TouchPoint>,
    /// Максимальное число пальцев в текущей «волне» касаний (от 0 до возврата к 0).
    peak_fingers: usize,
    /// Момент начала текущей волны касаний (секунды).
    wave_start_time: Option<f64>,
    /// Максимальное смещение любого пальца в текущей волне (для отличия тапа от свайпа).
    wave_max_drift: f32,
    /// Среднее смещение пальцев по осям в текущей волне (для детекции свайпа 3 пальцами вниз).
    wave_sum_delta: Vec2,
    /// Число отпущенных пальцев, чьё смещение учтено в `wave_sum_delta`.
    wave_ended_count: usize,
    /// Была ли уже сработана команда свайпа 3 пальцами вниз в текущей волне.
    three_finger_swipe_fired: bool,
    /// Последний центр волны касаний.
    wave_last_center: Pos2,
    /// Снимок геометрии 2 пальцев на предыдущем шаге.
    prev_two_finger: Option<TwoFingerSnapshot>,
    /// Сработало ли долгое нажатие для текущего одиночного касания.
    long_press_fired: bool,
    /// Позиция открытого быстрого сенсорного меню (если открыто жестом).
    pub quick_menu_pos: Option<Pos2>,
    /// Блокировать ли рисование пальцем в текущем кадре.
    pub block_finger_draw: bool,
}

impl TouchGestureEngine {
    /// Создаёт новый экземпляр распознавателя жестов.
    pub fn new() -> Self {
        Self::default()
    }

    /// Возвращает число активных пальцевых касаний (без стилуса).
    pub fn active_finger_count(&self) -> usize {
        self.contacts.iter().filter(|c| !c.is_stylus).count()
    }

    /// Обрабатывает поток событий `egui::Event` одного кадра с учётом настроек [`TouchPrefs`].
    pub fn process_events(&mut self, events: &[Event], now_sec: f64, prefs: &TouchPrefs, stylus_active: bool) -> GestureFrameOutput {
        let mut out = GestureFrameOutput::default();

        for event in events {
            let Event::Touch { id, phase, pos, force, .. } = *event else { continue };
            // На Android/Windows касания с давлением или при активном стилусе классифицируем как перо.
            let is_stylus = stylus_active || force.is_some_and(|f| f > 0.0);
            if prefs.palm_rejection && stylus_active && !is_stylus {
                out.block_finger_draw = true;
                continue;
            }
            match phase {
                TouchPhase::Start => self.on_touch_start(id, pos, now_sec, is_stylus),
                TouchPhase::Move => self.on_touch_move(id, pos, prefs, &mut out),
                TouchPhase::End => self.on_touch_end(id, pos, now_sec, prefs, &mut out),
                TouchPhase::Cancel => self.on_touch_cancel(id),
            }
        }

        // Проверяем двухпальцевый жест (pinch / zoom / rotate), если ровно 2 пальца на экране.
        let finger_positions = self.finger_positions();
        if prefs.pinch_zoom_rotate && finger_positions.len() == 2 {
            let snap = snapshot_two(finger_positions[0], finger_positions[1]);
            if let Some(prev) = self.prev_two_finger
                && prev.distance >= MIN_PINCH_DIST_PT
                && snap.distance >= MIN_PINCH_DIST_PT
            {
                let pan = snap.center - prev.center;
                let zoom_factor = (snap.distance / prev.distance).clamp(0.5, 2.0);
                let mut d_angle = snap.angle - prev.angle;
                if d_angle > std::f32::consts::PI {
                    d_angle -= std::f32::consts::TAU;
                } else if d_angle < -std::f32::consts::PI {
                    d_angle += std::f32::consts::TAU;
                }
                let rotation_deg = d_angle.to_degrees();
                if pan.length_sq() > 1e-4 || (zoom_factor - 1.0).abs() > 1e-4 || rotation_deg.abs() > 0.05 {
                    out.two_finger = Some(TwoFingerDelta { pan, zoom_factor, rotation_deg, center: snap.center });
                }
            }
            self.prev_two_finger = Some(snap);
        } else {
            self.prev_two_finger = None;
        }

        // Проверяем долгое нажатие 1 пальцем (когда стилус не активен и палец не двигается).
        if !stylus_active
            && self.contacts.len() == 1
            && let Some(c) = self.contacts.first()
            && !c.is_stylus
            && !self.long_press_fired
            && c.max_drift <= LONG_PRESS_MAX_DRIFT_PT
        {
            let hold_sec = f64::from(prefs.long_press_ms.clamp(200, 1500)) / 1000.0;
            if now_sec - c.start_time >= hold_sec {
                self.long_press_fired = true;
                out.long_press = Some(c.pos);
            }
        }

        // Блокируем рисование пальцем при мультитаче, при включённом Palm Rejection со стилусом,
        // либо если пользователь отключил `finger_draws`.
        let multi_finger = self.active_finger_count() >= 2;
        let finger_blocked = !stylus_active && !prefs.finger_draws && self.active_finger_count() > 0;
        let palm_blocked = prefs.palm_rejection && stylus_active;
        out.block_finger_draw |= multi_finger || finger_blocked || palm_blocked;
        self.block_finger_draw = out.block_finger_draw;
        out
    }

    /// Преобразует события нажатия боковых кнопок стилуса в команды согласно [`TouchPrefs`].
    pub fn stylus_button_commands(&self, primary_pressed: bool, secondary_pressed: bool, pos: Pos2, prefs: &TouchPrefs) -> Vec<GestureCommand> {
        let mut cmds = Vec::new();
        if primary_pressed && let Some(cmd) = stylus_button_to_command(prefs.stylus_primary_button, pos) {
            cmds.push(cmd);
        }
        if secondary_pressed && let Some(cmd) = stylus_button_to_command(prefs.stylus_secondary_button, pos) {
            cmds.push(cmd);
        }
        cmds
    }

    fn on_touch_start(&mut self, id: TouchId, pos: Pos2, now_sec: f64, is_stylus: bool) {
        self.contacts.retain(|c| c.id != id);
        if self.contacts.len() >= MAX_CONTACTS {
            self.contacts.remove(0);
        }
        self.contacts.push(TouchPoint { id, start_pos: pos, pos, start_time: now_sec, max_drift: 0.0, is_stylus });
        if !is_stylus {
            if self.wave_start_time.is_none() {
                self.wave_start_time = Some(now_sec);
                self.peak_fingers = 0;
                self.wave_max_drift = 0.0;
                self.wave_sum_delta = Vec2::ZERO;
                self.wave_ended_count = 0;
                self.three_finger_swipe_fired = false;
            }
            let fingers = self.active_finger_count();
            if fingers > self.peak_fingers {
                self.peak_fingers = fingers;
            }
            if fingers > 1 {
                // Многопальцевый жест отменяет ожидание долгого нажатия 1 пальцем.
                self.long_press_fired = true;
            } else {
                self.long_press_fired = false;
            }
            self.wave_last_center = self.finger_center().unwrap_or(pos);
        }
    }

    fn on_touch_move(&mut self, id: TouchId, pos: Pos2, prefs: &TouchPrefs, out: &mut GestureFrameOutput) {
        if let Some(c) = self.contacts.iter_mut().find(|c| c.id == id) {
            c.pos = pos;
            let drift = c.pos.distance(c.start_pos);
            if drift > c.max_drift {
                c.max_drift = drift;
            }
            if !c.is_stylus && drift > self.wave_max_drift {
                self.wave_max_drift = drift;
            }
        }
        // Свайп 3 пальцами вниз: проверяем в реальном времени, пока 3 пальца на экране.
        if !self.three_finger_swipe_fired && self.active_finger_count() == 3 {
            let mut sum = Vec2::ZERO;
            let mut count = 0usize;
            for c in &self.contacts {
                if !c.is_stylus {
                    sum += c.pos - c.start_pos;
                    count += 1;
                }
            }
            if count == 3 {
                let avg = sum / 3.0;
                // Движение строго преимущественно вниз (+Y) на расстояние >= THREE_FINGER_SWIPE_MIN_Y_PT.
                if avg.y >= THREE_FINGER_SWIPE_MIN_Y_PT && avg.y > avg.x.abs() * 1.5 {
                    self.three_finger_swipe_fired = true;
                    let center = self.finger_center().unwrap_or(pos);
                    if let Some(cmd) = gesture_action_to_command(prefs.three_finger_swipe_down, center) {
                        out.triggered.push(cmd);
                    }
                }
            }
        }
    }

    fn on_touch_end(&mut self, id: TouchId, pos: Pos2, now_sec: f64, prefs: &TouchPrefs, out: &mut GestureFrameOutput) {
        if let Some(idx) = self.contacts.iter().position(|c| c.id == id) {
            let c = self.contacts.remove(idx);
            if !c.is_stylus {
                let delta = pos - c.start_pos;
                let drift = delta.length();
                if drift > self.wave_max_drift {
                    self.wave_max_drift = drift;
                }
                self.wave_sum_delta += delta;
                self.wave_ended_count += 1;
                self.wave_last_center = pos;
            }
        }

        // Когда все пальцы отпущены — завершаем волну и проверяем тапы 3/4 пальцами.
        if self.active_finger_count() == 0 {
            if let Some(start_t) = self.wave_start_time.take() {
                let duration = now_sec - start_t;
                let is_quick_tap = duration <= MULTI_TAP_MAX_SEC && self.wave_max_drift <= TAP_MAX_DRIFT_PT;
                if !self.three_finger_swipe_fired && is_quick_tap {
                    let action = match self.peak_fingers {
                        3 => Some(prefs.three_finger_tap),
                        4 => Some(prefs.four_finger_tap),
                        _ => None,
                    };
                    if let Some(act) = action
                        && let Some(cmd) = gesture_action_to_command(act, self.wave_last_center)
                    {
                        out.triggered.push(cmd);
                    }
                }
            }
            self.reset_wave();
        }
    }

    fn on_touch_cancel(&mut self, id: TouchId) {
        self.contacts.retain(|c| c.id != id);
        if self.active_finger_count() == 0 {
            self.reset_wave();
        }
    }

    fn reset_wave(&mut self) {
        self.peak_fingers = 0;
        self.wave_start_time = None;
        self.wave_max_drift = 0.0;
        self.wave_sum_delta = Vec2::ZERO;
        self.wave_ended_count = 0;
        self.three_finger_swipe_fired = false;
        self.long_press_fired = false;
        self.prev_two_finger = None;
    }

    fn finger_positions(&self) -> Vec<Pos2> {
        self.contacts.iter().filter(|c| !c.is_stylus).map(|c| c.pos).collect()
    }

    fn finger_center(&self) -> Option<Pos2> {
        let mut sum = Vec2::ZERO;
        let mut count = 0usize;
        for c in &self.contacts {
            if !c.is_stylus {
                sum += c.pos.to_vec2();
                count += 1;
            }
        }
        (count > 0).then(|| (sum / count as f32).to_pos2())
    }
}

fn snapshot_two(a: Pos2, b: Pos2) -> TwoFingerSnapshot {
    let diff = b - a;
    TwoFingerSnapshot { center: Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5), distance: diff.length(), angle: diff.y.atan2(diff.x) }
}

/// Применяет инкрементальный двухпальцевый жест (сдвиг, зум, поворот) к активному виду документа.
pub fn apply_two_finger_delta(app: &mut crate::PhotocraftApp, delta: TwoFingerDelta) {
    let Some(idx) = app.session.active_index() else { return };
    let ppp = app.canvas_ppp();
    let Some(view) = app.ui.views.get_mut(idx) else { return };

    let old_zoom = view.zoom.max(MIN_ZOOM);
    let new_zoom = (old_zoom * delta.zoom_factor).clamp(MIN_ZOOM, MAX_ZOOM);
    view.zoom = new_zoom;

    let doc_scale = (new_zoom / ppp).max(0.01);
    let rot_rad = view.rotation.to_radians();
    let (sin_r, cos_r) = rot_rad.sin_cos();
    let dx = delta.pan.x / doc_scale;
    let dy = delta.pan.y / doc_scale;
    // Переводим экранный сдвиг в координаты документа с учётом текущего угла поворота вида.
    view.center[0] -= dx * cos_r + dy * sin_r;
    view.center[1] -= -dx * sin_r + dy * cos_r;

    let next_rot = (view.rotation + delta.rotation_deg).rem_euclid(360.0);
    view.rotation = if next_rot >= 360.0 { 0.0 } else { next_rot };
}

/// Выполняет команду [`GestureCommand`] через существующую командную архитектуру PhotoCraft.
pub fn execute_gesture_command(app: &mut crate::PhotocraftApp, cmd: GestureCommand) {
    match cmd {
        GestureCommand::Engine(id) => {
            if let Err(err) = app.run(id, json!({})) {
                app.ui.status = err;
            }
        }
        GestureCommand::ToggleCanvasOnly => {
            app.tablet_ui.canvas_only = !app.tablet_ui.canvas_only;
        }
        GestureCommand::OpenQuickMenu(pos) => {
            app.touch_gestures.quick_menu_pos = Some(pos);
        }
        GestureCommand::SecondaryClick(pos) => {
            app.touch_gestures.quick_menu_pos = Some(pos);
        }
        GestureCommand::PanHold => {
            if let Err(err) = app.run("tools.select.hand", json!({})) {
                app.ui.status = err;
            }
        }
    }
}

/// Кадровый обработчик сенсорных жестов: читает события из `ctx`, применяет настройки `prefs.touch`
/// и диспатчит команды в движок. Вызывается один раз за кадр из `logic_frame`.
pub fn tick(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    let touch = app.session.prefs().touch.clone();
    let (events, now_sec) = ctx.input(|i| (i.events.clone(), i.time));
    let stylus_active = app.stylus.feed.get().is_some();

    let out = app.touch_gestures.process_events(&events, now_sec, &touch, stylus_active);
    if let Some(delta) = out.two_finger {
        apply_two_finger_delta(app, delta);
    }
    if let Some(pos) = out.long_press {
        app.touch_gestures.quick_menu_pos = Some(pos);
    }
    for cmd in out.triggered {
        execute_gesture_command(app, cmd);
    }
}

#[cfg(test)]
#[path = "touch_gestures/tests.rs"]
mod tests;
