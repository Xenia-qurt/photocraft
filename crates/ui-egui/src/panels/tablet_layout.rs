//! Адаптивная планшетная и мобильная компоновка интерфейса PhotoCraft для Android и сенсорных экранов.
//! - Сенсорные цели [`TOUCH_TARGET_DP`] = `48.0 dp` (Material Design / Android HIG).
//! - Верхняя панель, левая шторка с 69 инструментами в 10 категориях + flyout, правая шторка
//!   (Layers/Color/Brushes/Properties/History), плавающий HUD кисти (`tools.setBrush`) и режим «Только холст».

use egui::{Align2, Area, Color32, CornerRadius, Frame, Id, Margin, Order, Pos2, Rect, ScrollArea, Stroke, Vec2, vec2};
use photocraft_engine::prefs::TouchPrefs;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;

/// Минимальный размер сенсорной кнопки по стандартам Android (dp).
pub const TOUCH_TARGET_DP: f32 = 48.0;
/// Ширина компактной левой шторки категорий инструментов (dp).
pub const LEFT_RAIL_WIDTH_DP: f32 = 58.0;
/// Ширина выезжающего подменю инструментов категории (dp).
pub const FLYOUT_WIDTH_DP: f32 = 248.0;
/// Ширина правой выдвижной шторки панелей (dp).
pub const RIGHT_DRAWER_WIDTH_DP: f32 = 304.0;
/// Высота компактной верхней панели (dp).
pub const TOP_BAR_HEIGHT_DP: f32 = 52.0;

/// 10 категорий инструментов левой шторки (в совокупности покрывают все 69 инструментов `Tool::ALL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    /// Выделение (10 инструментов).
    Selection,
    /// Кадрирование и фрагменты (4 инструмента).
    Crop,
    /// Пипетка, линейка и заметки (5 инструментов).
    Eyedropper,
    /// Ретушь и коррекция (12 инструментов).
    Retouching,
    /// Рисование кистью, ластик и заливки (13 инструментов).
    Painting,
    /// Фигуры (7 инструментов).
    Drawing,
    /// Векторные контуры и перо (8 инструментов).
    Vector,
    /// Текст и текстовые маски (4 инструмента).
    Type,
    /// Навигация по холсту (3 инструмента).
    Navigation,
    /// Перемещение, монтажные области и рамка (3 инструмента).
    ArtboardFrame,
}

impl ToolCategory {
    /// Все 10 категорий в порядке отображения в левой шторке.
    pub const ALL: [ToolCategory; 10] = [
        ToolCategory::ArtboardFrame,
        ToolCategory::Selection,
        ToolCategory::Crop,
        ToolCategory::Eyedropper,
        ToolCategory::Retouching,
        ToolCategory::Painting,
        ToolCategory::Drawing,
        ToolCategory::Vector,
        ToolCategory::Type,
        ToolCategory::Navigation,
    ];

    /// Заголовок категории в подменю.
    pub fn title(self) -> &'static str {
        match self {
            Self::Selection => "Selection",
            Self::Crop => "Crop",
            Self::Eyedropper => "Eyedropper",
            Self::Retouching => "Retouching",
            Self::Painting => "Painting",
            Self::Drawing => "Drawing",
            Self::Vector => "Vector",
            Self::Type => "Type",
            Self::Navigation => "Navigation",
            Self::ArtboardFrame => "Artboard / Frame",
        }
    }

    /// Краткий символ-бейдж категории для компактной левой панели.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Selection => "M",
            Self::Crop => "C",
            Self::Eyedropper => "I",
            Self::Retouching => "J",
            Self::Painting => "B",
            Self::Drawing => "U",
            Self::Vector => "P",
            Self::Type => "T",
            Self::Navigation => "H",
            Self::ArtboardFrame => "V",
        }
    }

    /// Полный список инструментов категории (сумма по всем 10 категориям = ровно 69).
    pub fn tools(self) -> &'static [Tool] {
        match self {
            Self::Selection => &[
                Tool::RectMarquee,
                Tool::EllipseMarquee,
                Tool::SingleRowMarquee,
                Tool::SingleColumnMarquee,
                Tool::Lasso,
                Tool::PolygonLasso,
                Tool::MagneticLasso,
                Tool::ObjectSelection,
                Tool::QuickSelection,
                Tool::MagicWand,
            ],
            Self::Crop => &[Tool::Crop, Tool::PerspectiveCrop, Tool::Slice, Tool::SliceSelect],
            Self::Eyedropper => &[Tool::Eyedropper, Tool::ColorSampler, Tool::Ruler, Tool::Note, Tool::Count],
            Self::Retouching => &[
                Tool::SpotHealing,
                Tool::Remove,
                Tool::Healing,
                Tool::Patch,
                Tool::ContentAwareMove,
                Tool::RedEye,
                Tool::Blur,
                Tool::Sharpen,
                Tool::Smudge,
                Tool::Dodge,
                Tool::Burn,
                Tool::Sponge,
            ],
            Self::Painting => &[
                Tool::Brush,
                Tool::Pencil,
                Tool::ColorReplacement,
                Tool::MixerBrush,
                Tool::CloneStamp,
                Tool::PatternStamp,
                Tool::HistoryBrush,
                Tool::ArtHistoryBrush,
                Tool::Eraser,
                Tool::BackgroundEraser,
                Tool::MagicEraser,
                Tool::Gradient,
                Tool::PaintBucket,
            ],
            Self::Drawing => &[Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Star, Tool::Line, Tool::CustomShape],
            Self::Vector => &[
                Tool::Pen,
                Tool::FreeformPen,
                Tool::CurvaturePen,
                Tool::AddAnchorPoint,
                Tool::DeleteAnchorPoint,
                Tool::ConvertPoint,
                Tool::PathSelection,
                Tool::DirectSelection,
            ],
            Self::Type => &[Tool::Type, Tool::VerticalType, Tool::HorizontalTypeMask, Tool::VerticalTypeMask],
            Self::Navigation => &[Tool::Hand, Tool::RotateView, Tool::Zoom],
            Self::ArtboardFrame => &[Tool::Move, Tool::Artboard, Tool::Frame],
        }
    }

    /// Возвращает категорию, которой принадлежит `tool`.
    pub fn of(tool: Tool) -> Self {
        Self::ALL.into_iter().find(|cat| cat.tools().contains(&tool)).unwrap_or(Self::Painting)
    }
}

/// Вкладка правой сворачиваемой шторки панелей.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawerTab {
    /// Панель слоёв.
    #[default]
    Layers,
    /// Панель цвета.
    Color,
    /// Панель пресетов кистей.
    Brushes,
    /// Панель свойств.
    Properties,
    /// Панель истории.
    History,
}

impl DrawerTab {
    /// Все вкладки правой шторки.
    pub const ALL: [DrawerTab; 5] = [DrawerTab::Layers, DrawerTab::Color, DrawerTab::Brushes, DrawerTab::Properties, DrawerTab::History];

    /// Подпись вкладки.
    pub fn label(self) -> &'static str {
        match self {
            Self::Layers => "Layers",
            Self::Color => "Color",
            Self::Brushes => "Brushes",
            Self::Properties => "Props",
            Self::History => "History",
        }
    }
}

/// Сессионное состояние планшетного интерфейса.
#[derive(Clone, Debug, Default)]
pub struct TabletUiState {
    /// Режим «Только холст» (переключается свайпом 3 пальцами вниз или кнопкой).
    pub canvas_only: bool,
    /// Открытая категория инструментов в выезжающем подменю левой шторки.
    pub open_category: Option<ToolCategory>,
    /// Открыта ли правая шторка панелей.
    pub right_drawer_open: bool,
    /// Активная вкладка правой шторки.
    pub right_tab: DrawerTab,
    /// Принудительное переопределение режима планшета (используется в тестах и ручном превью).
    pub force_tablet_mode: Option<bool>,
}

/// Определяет, должна ли использоваться планшетная компоновка в текущем кадре.
///
/// На Android активируется автоматически при `prefs.auto_tablet_layout == true`.
/// На десктопе по умолчанию остаётся `false`, чтобы не менять привычную компоновку и UI-тесты,
/// но может быть включена явно через [`TabletUiState::force_tablet_mode`].
pub fn is_tablet_mode(touch: &TouchPrefs, state: &TabletUiState) -> bool {
    if let Some(forced) = state.force_tablet_mode {
        return forced;
    }
    touch.auto_tablet_layout && cfg!(target_os = "android")
}

/// Возвращает `true`, если для активного инструмента уместно показать плавающий HUD кисти.
pub fn shows_brush_hud(tool: Tool) -> bool {
    matches!(ToolCategory::of(tool), ToolCategory::Painting | ToolCategory::Retouching)
}

/// Отрисовывает полный планшетный интерфейс поверх `ui` (или режим «Только холст»).
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let tokens = Tokens::get(&ctx);
    let home = app.home_screen_up();

    if app.tablet_ui.canvas_only && !home {
        // Режим «Только холст»: рисуем только документ и кнопку возврата в углу.
        crate::canvas::document_area(app, ui);
        draw_canvas_only_restore_pill(app, &ctx, &tokens);
        draw_quick_menu(app, &ctx, &tokens);
        crate::notices::show(app, ui);
        return;
    }

    // Компактная верхняя панель (≥48 dp).
    egui::Panel::top("tablet_top_bar")
        .exact_size(TOP_BAR_HEIGHT_DP)
        .frame(Frame::new().fill(tokens.chrome).inner_margin(Margin::symmetric(8, 2)).stroke(Stroke::new(1.0, tokens.separator)))
        .show(ui, |ui| draw_top_bar(app, ui, &tokens, home));

    // Левая шторка категорий инструментов (показывается при открытом документе).
    if !home {
        egui::Panel::left("tablet_left_tools")
            .exact_size(LEFT_RAIL_WIDTH_DP)
            .resizable(false)
            .frame(Frame::new().fill(tokens.dock).inner_margin(Margin::symmetric(4, 6)).stroke(Stroke::new(1.0, tokens.separator)))
            .show(ui, |ui| draw_left_tool_rail(app, ui, &tokens));
    }

    // Правая выдвижная шторка панелей (Layers, Color, Brushes, Properties, History).
    if !home && app.tablet_ui.right_drawer_open {
        egui::Panel::right("tablet_right_drawer")
            .exact_size(RIGHT_DRAWER_WIDTH_DP)
            .resizable(false)
            .frame(Frame::new().fill(tokens.dock).inner_margin(Margin::same(8)).stroke(Stroke::new(1.0, tokens.separator)))
            .show(ui, |ui| draw_right_drawer(app, ui, &tokens));
    }

    // Центральная область — холст или экран Home.
    egui::CentralPanel::default().frame(Frame::new().fill(tokens.canvas)).show(ui, |ui| {
        crate::canvas::document_area(app, ui);
        crate::notices::show(app, ui);
    });

    if !home {
        draw_category_flyout(app, &ctx, &tokens);
        if shows_brush_hud(app.ui.tool) {
            draw_brush_hud(app, &ctx, &tokens);
        }
    }
    draw_quick_menu(app, &ctx, &tokens);
}

fn touch_button(ui: &mut egui::Ui, label: &str, selected: bool, size: Vec2, tokens: &Tokens) -> egui::Response {
    let fill = if selected { tokens.accent_soft } else { tokens.card };
    let stroke = if selected { Stroke::new(1.5, tokens.accent) } else { Stroke::new(1.0, tokens.card_border) };
    let text_color = if selected { tokens.accent_text } else { tokens.text };
    let btn = egui::Button::new(egui::RichText::new(label).color(text_color).size(14.0))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .min_size(size);
    ui.add(btn)
}

fn draw_top_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tokens: &Tokens, home: bool) {
    let btn_sz = vec2(TOUCH_TARGET_DP, TOUCH_TARGET_DP - 4.0);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);

        if touch_button(ui, "↶", false, btn_sz, tokens).on_hover_text("Undo").clicked() {
            let _ = app.run("edit.undo", json!({}));
        }
        if touch_button(ui, "↷", false, btn_sz, tokens).on_hover_text("Redo").clicked() {
            let _ = app.run("edit.redo", json!({}));
        }
        if touch_button(ui, "Open", false, vec2(64.0, TOUCH_TARGET_DP - 4.0), tokens).clicked() {
            let _ = app.run("file.open", json!({}));
        }
        if !home && touch_button(ui, "Save", false, vec2(64.0, TOUCH_TARGET_DP - 4.0), tokens).clicked() {
            let _ = app.run("file.save", json!({}));
        }

        // Название активного документа и текущий масштаб.
        let zoom_pct = (app.current_zoom() * 100.0).round() as i32;
        if !home && touch_button(ui, &format!("{zoom_pct}%"), false, vec2(66.0, TOUCH_TARGET_DP - 4.0), tokens).clicked() {
            let _ = app.run("view.fitOnScreen", json!({}));
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            if touch_button(ui, "⚙", false, btn_sz, tokens).on_hover_text("Preferences").clicked() {
                let _ = app.run("edit.preferences.general", json!({}));
            }
            if !home && touch_button(ui, "⛶", app.tablet_ui.canvas_only, btn_sz, tokens).on_hover_text("Canvas Only (3-finger swipe down)").clicked() {
                app.tablet_ui.canvas_only = !app.tablet_ui.canvas_only;
                app.tablet_ui.open_category = None;
            }
            if !home && touch_button(ui, "☰", app.tablet_ui.right_drawer_open, btn_sz, tokens).on_hover_text("Panels").clicked() {
                app.tablet_ui.right_drawer_open = !app.tablet_ui.right_drawer_open;
            }
        });
    });
}

fn draw_left_tool_rail(app: &mut PhotocraftApp, ui: &mut egui::Ui, tokens: &Tokens) {
    let active_cat = ToolCategory::of(app.ui.tool);
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
        for cat in ToolCategory::ALL {
            let is_active = cat == active_cat;
            let is_open = app.tablet_ui.open_category == Some(cat);
            let resp = touch_button(ui, cat.glyph(), is_active || is_open, vec2(TOUCH_TARGET_DP, TOUCH_TARGET_DP), tokens).on_hover_text(cat.title());
            if resp.clicked() {
                if app.tablet_ui.open_category == Some(cat) {
                    app.tablet_ui.open_category = None;
                } else {
                    app.tablet_ui.open_category = Some(cat);
                }
            }
        }
    });
}

fn draw_category_flyout(app: &mut PhotocraftApp, ctx: &egui::Context, tokens: &Tokens) {
    let Some(cat) = app.tablet_ui.open_category else { return };
    let area_pos = Pos2::new(LEFT_RAIL_WIDTH_DP + 8.0, TOP_BAR_HEIGHT_DP + 8.0);
    let resp = Area::new(Id::unique("tablet_tool_flyout")).order(Order::Foreground).fixed_pos(area_pos).show(ctx, |ui| {
        Frame::new().fill(tokens.chrome).stroke(Stroke::new(1.0, tokens.card_border)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(8)).show(
            ui,
            |ui| {
                ui.set_width(FLYOUT_WIDTH_DP);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(cat.title()).strong().color(tokens.text).size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if touch_button(ui, "✕", false, vec2(36.0, 36.0), tokens).clicked() {
                            app.tablet_ui.open_category = None;
                        }
                    });
                });
                ui.separator();
                ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                    for &tool in cat.tools() {
                        let selected = app.ui.tool == tool;
                        if touch_button(ui, tool.label(), selected, vec2(FLYOUT_WIDTH_DP - 8.0, TOUCH_TARGET_DP), tokens).clicked() {
                            crate::tool_memory::select(app, tool);
                            app.tablet_ui.open_category = None;
                        }
                    }
                });
            },
        );
    });

    // Закрываем подменю при тапе вне его области.
    let clicked_outside =
        ctx.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !resp.response.rect.contains(p) && p.x > LEFT_RAIL_WIDTH_DP));
    if clicked_outside {
        app.tablet_ui.open_category = None;
    }
}

fn draw_right_drawer(app: &mut PhotocraftApp, ui: &mut egui::Ui, tokens: &Tokens) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for tab in DrawerTab::ALL {
            let selected = app.tablet_ui.right_tab == tab;
            if touch_button(ui, tab.label(), selected, vec2(54.0, TOUCH_TARGET_DP - 4.0), tokens).clicked() {
                app.tablet_ui.right_tab = tab;
            }
        }
    });
    ui.separator();
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.tablet_ui.right_tab {
        DrawerTab::Layers => super::layers::layers(app, ui),
        DrawerTab::Color => super::color::panel(app, ui),
        DrawerTab::Brushes => super::brush_panel::brushes::panel(app, ui),
        DrawerTab::Properties => super::properties::panel(app, ui),
        DrawerTab::History => super::history::history(app, ui),
    });
}

fn draw_brush_hud(app: &mut PhotocraftApp, ctx: &egui::Context, tokens: &Tokens) {
    let screen = ctx.content_rect();
    let hud_pos = Pos2::new(LEFT_RAIL_WIDTH_DP + 10.0, (screen.center().y - 110.0).max(TOP_BAR_HEIGHT_DP + 12.0));
    Area::new(Id::unique("tablet_brush_hud")).order(Order::Middle).fixed_pos(hud_pos).show(ctx, |ui| {
        Frame::new()
            .fill(Color32::from_rgba_unmultiplied(tokens.chrome.r(), tokens.chrome.g(), tokens.chrome.b(), 225))
            .stroke(Stroke::new(1.0, tokens.card_border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(56.0);
                let mut size = app.session.tools.brush.size;
                let prev_size = size;
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(format!("{}px", size.round() as u32)).color(tokens.text_dim).size(11.0));
                    ui.add(egui::Slider::new(&mut size, 1.0..=500.0).vertical().logarithmic(true).show_value(false));
                });
                if (size - prev_size).abs() > 0.05 {
                    let _ = app.run("tools.setBrush", json!({ "size": size }));
                }
                ui.separator();
                let mut opacity_pct = (app.session.tools.brush.opacity * 100.0).round();
                let prev_op = opacity_pct;
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(format!("{}%", opacity_pct as u32)).color(tokens.text_dim).size(11.0));
                    ui.add(egui::Slider::new(&mut opacity_pct, 1.0..=100.0).vertical().show_value(false));
                });
                if (opacity_pct - prev_op).abs() >= 1.0 {
                    let _ = app.run("tools.setBrush", json!({ "opacity": opacity_pct / 100.0 }));
                }
            });
    });
}

fn draw_canvas_only_restore_pill(app: &mut PhotocraftApp, ctx: &egui::Context, tokens: &Tokens) {
    Area::new(Id::unique("tablet_canvas_only_restore")).order(Order::Foreground).anchor(Align2::RIGHT_TOP, vec2(-12.0, 12.0)).show(ctx, |ui| {
        if touch_button(ui, "⛶", true, vec2(TOUCH_TARGET_DP, TOUCH_TARGET_DP), tokens).on_hover_text("Exit Canvas Only").clicked() {
            app.tablet_ui.canvas_only = false;
        }
    });
}

fn draw_quick_menu(app: &mut PhotocraftApp, ctx: &egui::Context, tokens: &Tokens) {
    let Some(pos) = app.touch_gestures.quick_menu_pos else { return };
    let bounds = ctx.content_rect();
    let clamped = Pos2::new(
        pos.x.clamp(bounds.left() + 8.0, (bounds.right() - 200.0).max(bounds.left() + 8.0)),
        pos.y.clamp(bounds.top() + 8.0, (bounds.bottom() - 280.0).max(bounds.top() + 8.0)),
    );
    let resp = Area::new(Id::unique("tablet_quick_menu")).order(Order::Foreground).fixed_pos(clamped).show(ctx, |ui| {
        Frame::new()
            .fill(tokens.chrome)
            .stroke(Stroke::new(1.0, tokens.accent_border))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(184.0);
                ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
                let items: &[(&str, &str)] =
                    &[("Undo", "edit.undo"), ("Redo", "edit.redo"), ("Deselect", "select.deselect"), ("Fit on Screen", "view.fitOnScreen")];
                for &(label, cmd) in items {
                    if touch_button(ui, label, false, vec2(176.0, TOUCH_TARGET_DP), tokens).clicked() {
                        let _ = app.run(cmd, json!({}));
                        app.touch_gestures.quick_menu_pos = None;
                    }
                }
                if touch_button(ui, "Canvas Only", app.tablet_ui.canvas_only, vec2(176.0, TOUCH_TARGET_DP), tokens).clicked() {
                    app.tablet_ui.canvas_only = !app.tablet_ui.canvas_only;
                    app.touch_gestures.quick_menu_pos = None;
                }
            });
    });
    let close_rect: Rect = resp.response.rect;
    let outside = ctx.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !close_rect.contains(p)));
    if outside {
        app.touch_gestures.quick_menu_pos = None;
    }
}

#[cfg(test)]
#[path = "tablet_layout/tests.rs"]
mod tests;
