//! Модульные тесты планшетной компоновки (`tablet_layout.rs`).

use std::collections::HashSet;

use super::*;

#[test]
fn all_69_tools_are_covered_across_ten_categories_without_duplicates() {
    assert_eq!(ToolCategory::ALL.len(), 10, "должно быть ровно 10 категорий");
    let mut seen = HashSet::new();
    for cat in ToolCategory::ALL {
        assert!(!cat.tools().is_empty(), "категория {:?} не должна быть пустой", cat);
        for &tool in cat.tools() {
            assert!(seen.insert(tool), "дубликат инструмента {tool:?} в категории {cat:?}");
            assert_eq!(ToolCategory::of(tool), cat);
        }
    }
    assert_eq!(seen.len(), Tool::ALL.len(), "должны быть покрыты все 69 инструментов");
    for tool in Tool::ALL {
        assert!(seen.contains(&tool), "инструмент {tool:?} отсутствует в планшетной шторке");
    }
}

#[test]
fn touch_target_meets_android_48dp_minimum() {
    const {
        assert!(TOUCH_TARGET_DP >= 48.0);
        assert!(TOP_BAR_HEIGHT_DP >= 48.0);
        assert!(LEFT_RAIL_WIDTH_DP >= 48.0);
    }
}

#[test]
fn tablet_mode_respects_override_and_defaults_off_on_desktop() {
    let prefs = TouchPrefs::default();
    let mut state = TabletUiState::default();
    assert_eq!(is_tablet_mode(&prefs, &state), cfg!(target_os = "android"));
    state.force_tablet_mode = Some(true);
    assert!(is_tablet_mode(&prefs, &state));
    state.force_tablet_mode = Some(false);
    assert!(!is_tablet_mode(&prefs, &state));
}
