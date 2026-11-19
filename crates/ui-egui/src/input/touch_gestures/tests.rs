//! Модульные тесты движка сенсорных жестов (`touch_gestures.rs`).

use egui::{TouchDeviceId, pos2};

use super::*;

fn touch_ev(id: u64, phase: TouchPhase, pos: Pos2, force: Option<f32>) -> Event {
    Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force }
}

#[test]
fn three_finger_tap_triggers_undo_and_four_finger_tap_triggers_redo() {
    let prefs = TouchPrefs::default();
    let mut eng = TouchGestureEngine::new();

    // 3-пальцевый тап -> Undo.
    let down3 = vec![
        touch_ev(1, TouchPhase::Start, pos2(100.0, 100.0), None),
        touch_ev(2, TouchPhase::Start, pos2(150.0, 100.0), None),
        touch_ev(3, TouchPhase::Start, pos2(200.0, 100.0), None),
    ];
    let _ = eng.process_events(&down3, 1.0, &prefs, false);
    let up3 = vec![
        touch_ev(1, TouchPhase::End, pos2(102.0, 101.0), None),
        touch_ev(2, TouchPhase::End, pos2(151.0, 102.0), None),
        touch_ev(3, TouchPhase::End, pos2(199.0, 100.0), None),
    ];
    let out = eng.process_events(&up3, 1.12, &prefs, false);
    assert_eq!(out.triggered, vec![GestureCommand::Engine("edit.undo")]);

    // 4-пальцевый тап -> Redo.
    let down4 = vec![
        touch_ev(1, TouchPhase::Start, pos2(100.0, 100.0), None),
        touch_ev(2, TouchPhase::Start, pos2(140.0, 100.0), None),
        touch_ev(3, TouchPhase::Start, pos2(180.0, 100.0), None),
        touch_ev(4, TouchPhase::Start, pos2(220.0, 100.0), None),
    ];
    let _ = eng.process_events(&down4, 2.0, &prefs, false);
    let up4 = vec![
        touch_ev(1, TouchPhase::End, pos2(100.0, 101.0), None),
        touch_ev(2, TouchPhase::End, pos2(141.0, 100.0), None),
        touch_ev(3, TouchPhase::End, pos2(180.0, 102.0), None),
        touch_ev(4, TouchPhase::End, pos2(220.0, 100.0), None),
    ];
    let out = eng.process_events(&up4, 2.15, &prefs, false);
    assert_eq!(out.triggered, vec![GestureCommand::Engine("edit.redo")]);
}

#[test]
fn three_finger_swipe_down_toggles_canvas_only_without_firing_undo() {
    let prefs = TouchPrefs::default();
    let mut eng = TouchGestureEngine::new();

    let down = vec![
        touch_ev(1, TouchPhase::Start, pos2(100.0, 100.0), None),
        touch_ev(2, TouchPhase::Start, pos2(160.0, 100.0), None),
        touch_ev(3, TouchPhase::Start, pos2(220.0, 100.0), None),
    ];
    let _ = eng.process_events(&down, 1.0, &prefs, false);

    let swipe = vec![
        touch_ev(1, TouchPhase::Move, pos2(104.0, 195.0), None),
        touch_ev(2, TouchPhase::Move, pos2(162.0, 195.0), None),
        touch_ev(3, TouchPhase::Move, pos2(223.0, 195.0), None),
    ];
    let out_move = eng.process_events(&swipe, 1.15, &prefs, false);
    assert_eq!(out_move.triggered, vec![GestureCommand::ToggleCanvasOnly]);

    // Отпускание после свайпа не должно вызывать Undo.
    let up = vec![
        touch_ev(1, TouchPhase::End, pos2(104.0, 200.0), None),
        touch_ev(2, TouchPhase::End, pos2(162.0, 200.0), None),
        touch_ev(3, TouchPhase::End, pos2(223.0, 200.0), None),
    ];
    let out_up = eng.process_events(&up, 1.2, &prefs, false);
    assert!(out_up.triggered.is_empty(), "свайп вниз не должен конфликтовать с тапом 3 пальцами");
}

#[test]
fn two_finger_pinch_zoom_and_rotate_computes_delta() {
    let prefs = TouchPrefs::default();
    let mut eng = TouchGestureEngine::new();

    let start = vec![touch_ev(1, TouchPhase::Start, pos2(100.0, 200.0), None), touch_ev(2, TouchPhase::Start, pos2(200.0, 200.0), None)];
    let first = eng.process_events(&start, 1.0, &prefs, false);
    assert!(first.two_finger.is_none());
    assert!(first.block_finger_draw);

    // Расстояние увеличилось вдвое (100 -> 200), поворот на 90° вокруг (150, 200).
    let moved = vec![touch_ev(1, TouchPhase::Move, pos2(150.0, 100.0), None), touch_ev(2, TouchPhase::Move, pos2(150.0, 300.0), None)];
    let second = eng.process_events(&moved, 1.05, &prefs, false);
    let delta = second.two_finger.expect("должен быть вычислен двухпальцевый сдвиг");
    assert!((delta.zoom_factor - 2.0).abs() < 0.01, "zoom={}", delta.zoom_factor);
    assert!((delta.rotation_deg - 90.0).abs() < 0.1, "rot={}", delta.rotation_deg);
}

#[test]
fn long_press_respects_long_press_ms_and_palm_rejection_blocks_fingers() {
    let mut prefs = TouchPrefs { long_press_ms: 400, ..TouchPrefs::default() };
    let mut eng = TouchGestureEngine::new();

    let _ = eng.process_events(&[touch_ev(1, TouchPhase::Start, pos2(50.0, 60.0), None)], 1.0, &prefs, false);
    assert!(eng.process_events(&[], 1.3, &prefs, false).long_press.is_none());
    let lp = eng.process_events(&[], 1.45, &prefs, false);
    assert_eq!(lp.long_press, Some(pos2(50.0, 60.0)));
    // Повторно в том же касании не срабатывает.
    assert!(eng.process_events(&[], 1.6, &prefs, false).long_press.is_none());

    // Отключение рисования пальцем блокирует палец.
    prefs.finger_draws = false;
    let blocked = eng.process_events(&[], 1.7, &prefs, false);
    assert!(blocked.block_finger_draw);

    // Кнопки стилуса преобразуются в команды.
    let cmds = eng.stylus_button_commands(true, true, pos2(10.0, 20.0), &prefs);
    assert_eq!(cmds, vec![GestureCommand::Engine("tools.select.eraser"), GestureCommand::Engine("tools.select.eyedropper")]);
}
