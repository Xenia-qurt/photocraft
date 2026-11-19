//! Модульные тесты преобразования стилуса и Palm Rejection для Android (`android.rs`).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use super::*;

#[test]
fn tilt_from_polar_maps_cardinal_directions_to_w3c_degrees() {
    // Перпендикулярное перо -> (0, 0).
    assert_eq!(tilt_from_polar(0.0, 0.0), (0.0, 0.0));

    // Наклон 45° вправо (orientation = +π/2) -> tilt_x ≈ +45°, tilt_y ≈ 0°.
    let (tx, ty) = tilt_from_polar(FRAC_PI_4, FRAC_PI_2);
    assert!((tx - 45.0).abs() < 0.01, "tx={tx}");
    assert!(ty.abs() < 0.01, "ty={ty}");

    // Наклон 45° влево (orientation = -π/2) -> tilt_x ≈ -45°, tilt_y ≈ 0°.
    let (tx, ty) = tilt_from_polar(FRAC_PI_4, -FRAC_PI_2);
    assert!((tx + 45.0).abs() < 0.01, "tx={tx}");
    assert!(ty.abs() < 0.01, "ty={ty}");

    // Наклон 45° от пользователя вверх (orientation = 0) -> tilt_x ≈ 0°, tilt_y ≈ -45°.
    let (tx, ty) = tilt_from_polar(FRAC_PI_4, 0.0);
    assert!(tx.abs() < 0.01, "tx={tx}");
    assert!((ty + 45.0).abs() < 0.01, "ty={ty}");

    // Наклон 45° к пользователю вниз (orientation = π) -> tilt_x ≈ 0°, tilt_y ≈ +45°.
    let (tx, ty) = tilt_from_polar(FRAC_PI_4, PI);
    assert!(tx.abs() < 0.01, "tx={tx}");
    assert!((ty - 45.0).abs() < 0.01, "ty={ty}");

    // Неконечные значения не приводят к NaN.
    assert_eq!(tilt_from_polar(f32::NAN, 0.0), (0.0, 0.0));
    assert_eq!(tilt_from_polar(0.5, f32::INFINITY), (0.0, 0.0));
}

#[test]
fn stylus_and_eraser_events_produce_sanitized_samples_and_button_edges() {
    let mut mapper = AndroidInputMapper::new();

    // Наведение стилуса с нажатой основной боковой кнопкой.
    let hover = AndroidMotionEvent {
        pointer_id: 1,
        tool_type: AndroidToolType::Stylus,
        action: AndroidAction::HoverMove,
        pressure: 0.8,
        tilt_rad: FRAC_PI_4,
        orientation_rad: FRAC_PI_2,
        button_state: button::STYLUS_PRIMARY,
        timestamp_ms: 100,
        ..Default::default()
    };
    let res = mapper.process(&hover, true);
    assert!(!res.reject_touch);
    assert!(res.primary_just_pressed);
    assert!(!res.secondary_just_pressed);
    assert!(res.stylus_buttons.primary);
    match res.sample_update {
        Update::Set(Some(s)) => {
            assert_eq!(s.pressure, 0.0, "hover должен давать нулевое давление");
            assert!((s.tilt_x - 45.0).abs() < 0.01);
            assert!(!s.eraser);
        }
        other => panic!("ожидался Update::Set(Some(_)), получено {other:?}"),
    }

    // Касание ластиком с удержанием той же кнопки (фронта уже нет) и нажатием второй.
    let eraser = AndroidMotionEvent {
        pointer_id: 1,
        tool_type: AndroidToolType::Eraser,
        action: AndroidAction::Down,
        pressure: 1.4,
        button_state: button::STYLUS_PRIMARY | button::STYLUS_SECONDARY,
        timestamp_ms: 120,
        ..Default::default()
    };
    let res = mapper.process(&eraser, true);
    assert!(!res.primary_just_pressed);
    assert!(res.secondary_just_pressed);
    match res.sample_update {
        Update::Set(Some(s)) => {
            assert_eq!(s.pressure, 1.0, "давление > 1.0 обрезается до 1.0");
            assert!(s.eraser);
        }
        other => panic!("ожидался Update::Set(Some(_)), получено {other:?}"),
    }

    // Отрыв пера сохраняет сэмпл (`Update::Keep`), выход из hover очищает (`Update::Set(None)`).
    let up = AndroidMotionEvent { action: AndroidAction::Up, ..eraser };
    assert_eq!(mapper.process(&up, true).sample_update, Update::Keep);
    let exit = AndroidMotionEvent { action: AndroidAction::HoverExit, ..eraser };
    assert_eq!(mapper.process(&exit, true).sample_update, Update::Set(None));
}

#[test]
fn palm_rejection_blocks_fingers_during_stylus_use_grace_window_and_wide_contact() {
    let mut mapper = AndroidInputMapper::new();

    // Обычный палец без стилуса проходит.
    let finger = AndroidMotionEvent {
        pointer_id: 2,
        tool_type: AndroidToolType::Finger,
        action: AndroidAction::Down,
        touch_major: 14.0,
        timestamp_ms: 1000,
        ..Default::default()
    };
    assert!(!mapper.process(&finger, true).reject_touch);

    // Стилус касается экрана -> палец отклоняется.
    let pen_down = AndroidMotionEvent {
        pointer_id: 1,
        tool_type: AndroidToolType::Stylus,
        action: AndroidAction::Down,
        pressure: 0.5,
        timestamp_ms: 1100,
        ..Default::default()
    };
    let _ = mapper.process(&pen_down, true);
    assert!(mapper.process(&AndroidMotionEvent { timestamp_ms: 1110, ..finger }, true).reject_touch);

    // Стилус оторван -> в пределах grace-окна (250 мс) палец всё ещё отклоняется.
    let pen_up = AndroidMotionEvent { action: AndroidAction::Up, timestamp_ms: 1200, ..pen_down };
    let _ = mapper.process(&pen_up, true);
    assert!(mapper.process(&AndroidMotionEvent { timestamp_ms: 1350, ..finger }, true).reject_touch);

    // После окончания grace-окна обычный палец снова принимается.
    assert!(!mapper.process(&AndroidMotionEvent { timestamp_ms: 1600, ..finger }, true).reject_touch);

    // Широкое пятно касания (ладонь) отклоняется даже без стилуса.
    let palm = AndroidMotionEvent { touch_major: 64.0, timestamp_ms: 2000, ..finger };
    assert!(mapper.process(&palm, true).reject_touch);
    // При выключенной опции palm_rejection палец/ладонь не отклоняются.
    assert!(!mapper.process(&palm, false).reject_touch);
}

#[test]
fn mouse_resets_stylus_sample() {
    let mut mapper = AndroidInputMapper::new();
    let mouse = AndroidMotionEvent { pointer_id: 0, tool_type: AndroidToolType::Mouse, action: AndroidAction::Move, timestamp_ms: 50, ..Default::default() };
    let res = mapper.process(&mouse, true);
    assert_eq!(res.sample_update, Update::Set(None));
    assert!(!res.reject_touch);
}
