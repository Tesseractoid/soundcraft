//! The AK 4K EQ panel: a console-style knob layout with a live EQ curve.

use crate::{dsp, EqParams};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

const W: u32 = 800;
const H: u32 = 480;

const BG: Color32 = Color32::from_rgb(0x2a, 0x2b, 0x2d);
const PANEL: Color32 = Color32::from_rgb(0x34, 0x35, 0x38);
const INK: Color32 = Color32::from_rgb(0xe8, 0xe6, 0xe0);
const INK_SOFT: Color32 = Color32::from_rgb(0x9a, 0x99, 0x93);
const GRID: Color32 = Color32::from_rgb(0x45, 0x46, 0x4a);
const CURVE: Color32 = Color32::from_rgb(0xf2, 0xb0, 0x4a);

const RED: Color32 = Color32::from_rgb(0xc8, 0x3a, 0x32);
const GREEN: Color32 = Color32::from_rgb(0x3f, 0x8f, 0x4f);
const BLUE: Color32 = Color32::from_rgb(0x36, 0x63, 0xb0);
const BLACK: Color32 = Color32::from_rgb(0x18, 0x18, 0x1a);
const GREY: Color32 = Color32::from_rgb(0x8a, 0x8c, 0x90);
const BROWN: Color32 = Color32::from_rgb(0x6b, 0x4a, 0x32);

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(W, H)
}

pub fn create(params: Arc<EqParams>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        (),
        |_, _| {},
        move |ctx, setter, _| {
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(BG).inner_margin(egui::Margin::same(14)))
                .show(ctx, |ui| {
                    header(ui);
                    ui.add_space(6.0);
                    curve(ui, &params.settings());
                    ui.add_space(10.0);
                    controls(ui, &params, setter);
                });
        },
    )
}

fn header(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    let p = ui.painter();
    p.text(rect.left_center(), Align2::LEFT_CENTER, "AK 4K EQ", FontId::proportional(18.0), INK);
    p.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        "ATMOS KINGS  ·  E-SERIES BLACK",
        FontId::proportional(11.0),
        INK_SOFT,
    );
}

// ---------------------------------------------------------------------------
// Curve
// ---------------------------------------------------------------------------

fn curve(ui: &mut egui::Ui, s: &dsp::Settings) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0, PANEL);

    const F_MIN: f64 = 20.0;
    const F_MAX: f64 = 20_000.0;
    const DB: f64 = 18.0;
    let x_of = |f: f64| {
        let t = (f / F_MIN).ln() / (F_MAX / F_MIN).ln();
        rect.left() + 8.0 + t as f32 * (rect.width() - 16.0)
    };
    let y_of = |db: f64| {
        let t = (db.clamp(-DB * 1.2, DB * 1.2) / DB) as f32;
        rect.center().y - t * (rect.height() / 2.0 - 10.0)
    };

    // grid
    for f in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0] {
        let x = x_of(f);
        p.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, GRID));
        let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
        p.text(Pos2::new(x + 3.0, rect.bottom() - 4.0), Align2::LEFT_BOTTOM, label, FontId::proportional(9.0), INK_SOFT);
    }
    for db in [-12.0, -6.0, 6.0, 12.0] {
        let y = y_of(db);
        p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, GRID));
        p.text(Pos2::new(rect.left() + 4.0, y - 1.0), Align2::LEFT_BOTTOM, format!("{db:+}"), FontId::proportional(9.0), INK_SOFT);
    }
    let y0 = y_of(0.0);
    p.line_segment([Pos2::new(rect.left(), y0), Pos2::new(rect.right(), y0)], Stroke::new(1.0, INK_SOFT));

    // response
    let n = 240;
    let points: Vec<Pos2> = (0..=n)
        .map(|i| {
            let f = F_MIN * (F_MAX / F_MIN).powf(i as f64 / n as f64);
            Pos2::new(x_of(f), y_of(dsp::response_db(s, f)))
        })
        .collect();
    let col = if s.eq_in { CURVE } else { CURVE.gamma_multiply(0.45) };
    p.add(Shape::line(points, Stroke::new(2.2, col)));
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

const KNOB: f32 = 42.0;

fn controls(ui: &mut egui::Ui, params: &EqParams, setter: &ParamSetter) {
    let total = ui.available_width();
    let gap = 8.0;
    // FILTERS, LF, LMF, HMF, HF, OUTPUT
    let widths = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let unit = (total - gap * 5.0) / widths.iter().sum::<f32>();
    let height = ui.available_height();

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        section(ui, unit, height, "FILTERS", |ui| {
            knob(ui, &params.hpf, setter, "HPF", GREY);
            knob(ui, &params.lpf, setter, "LPF", GREY);
        });
        section(ui, unit, height, "LF", |ui| {
            knob(ui, &params.lf_gain, setter, "GAIN", BLACK);
            knob(ui, &params.lf_freq, setter, "FREQ", BLACK);
            toggle(ui, &params.lf_bell, setter, "BELL");
        });
        section(ui, unit, height, "LMF", |ui| {
            knob(ui, &params.lmf_gain, setter, "GAIN", BLUE);
            knob(ui, &params.lmf_freq, setter, "FREQ", BLUE);
            knob(ui, &params.lmf_q, setter, "Q", BLUE);
        });
        section(ui, unit, height, "HMF", |ui| {
            knob(ui, &params.hmf_gain, setter, "GAIN", GREEN);
            knob(ui, &params.hmf_freq, setter, "FREQ", GREEN);
            knob(ui, &params.hmf_q, setter, "Q", GREEN);
        });
        section(ui, unit, height, "HF", |ui| {
            knob(ui, &params.hf_gain, setter, "GAIN", RED);
            knob(ui, &params.hf_freq, setter, "FREQ", RED);
            toggle(ui, &params.hf_bell, setter, "BELL");
        });
        section(ui, unit, height, "OUTPUT", |ui| {
            knob(ui, &params.output, setter, "LEVEL", BROWN);
            toggle(ui, &params.eq_in, setter, "EQ IN");
            toggle(ui, &params.analog, setter, "ANALOG");
        });
    });
}

fn section(ui: &mut egui::Ui, width: f32, height: f32, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, PANEL);
    ui.painter().text(
        Pos2::new(rect.center().x, rect.top() + 12.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(11.0),
        INK_SOFT,
    );
    let inner = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 24.0), rect.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.y = 4.0;
    add(&mut child);
}

/// A rotary knob: drag up/down (hold Shift for fine), double-click to reset.
fn knob<P: Param>(ui: &mut egui::Ui, param: &P, setter: &ParamSetter, label: &str, cap: Color32) {
    let size = Vec2::new(ui.available_width(), KNOB + 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());

    if resp.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, param.default_normalized_value());
        setter.end_set_parameter(param);
    } else {
        if resp.drag_started() {
            setter.begin_set_parameter(param);
        }
        if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let speed = if fine { 0.0015 } else { 0.006 };
            let delta = -resp.drag_delta().y * speed;
            if delta != 0.0 {
                let v = (param.unmodulated_normalized_value() + delta).clamp(0.0, 1.0);
                setter.set_parameter_normalized(param, v);
            }
        }
        if resp.drag_stopped() {
            setter.end_set_parameter(param);
        }
    }

    let p = ui.painter();
    let center = Pos2::new(rect.center().x, rect.top() + KNOB / 2.0 + 2.0);
    let r = KNOB / 2.0 - 4.0;
    let v = param.unmodulated_normalized_value();

    // scale arc (7 o'clock to 5 o'clock)
    let start = 0.75 * std::f32::consts::PI;
    let sweep = 1.5 * std::f32::consts::PI;
    let arc = |from: f32, to: f32, radius: f32| -> Vec<Pos2> {
        let n = 32;
        (0..=n)
            .map(|i| {
                let a = start + sweep * (from + (to - from) * i as f32 / n as f32);
                center + Vec2::angled(a) * radius
            })
            .collect()
    };
    p.add(Shape::line(arc(0.0, 1.0, r + 3.0), Stroke::new(2.0, GRID)));
    p.add(Shape::line(arc(0.0, v, r + 3.0), Stroke::new(2.0, CURVE)));

    // body and cap
    let hovered = resp.hovered() || resp.dragged();
    p.circle_filled(center, r, Color32::from_rgb(0x22, 0x22, 0x24));
    p.circle_filled(center, r * 0.62, cap);
    p.circle_stroke(center, r, Stroke::new(1.0, if hovered { INK_SOFT } else { GRID }));
    let a = start + sweep * v;
    p.line_segment(
        [center + Vec2::angled(a) * (r * 0.25), center + Vec2::angled(a) * (r - 2.0)],
        Stroke::new(2.0, INK),
    );

    // label + value
    let text = if hovered {
        param.normalized_value_to_string(v, true)
    } else {
        label.to_string()
    };
    p.text(
        Pos2::new(rect.center().x, rect.bottom() - 9.0),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.5),
        if hovered { INK } else { INK_SOFT },
    );
}

fn toggle(ui: &mut egui::Ui, param: &BoolParam, setter: &ParamSetter, label: &str) {
    let size = Vec2::new((ui.available_width() - 16.0).max(40.0), 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !param.value());
        setter.end_set_parameter(param);
    }
    let on = param.value();
    let p = ui.painter();
    p.rect_filled(rect, 4.0, if on { CURVE } else { Color32::from_rgb(0x22, 0x22, 0x24) });
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.5),
        if on { BLACK } else { INK_SOFT },
    );
}
