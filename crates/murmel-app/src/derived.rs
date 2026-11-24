//! The Derived fields window (the palette's "Derived fields…", the
//! colour-by menu and the field legend's menu; the maintainer's cards of
//! 2026-11-24: a catalogue with groups, formula, unit and inputs, the
//! quantities whose inputs are missing greyed): each quantity MurmelCraft
//! derives, the fields it reads (found by name, or chosen), whether it is
//! computed or in the file, and Compute, Show and Remove. The reference
//! values of the pressure quantities sit under them, each saying where it
//! came from, and a right-click keeps one as your default. Under them,
//! your formulas (the maintainer's card of 2026-11-24, "Full set plus a
//! formula library"): a formula to write, with the result's fields and
//! the calculator's functions to click into it, then your kept formulas
//! and the fields computed from formulas, each with Compute, Show,
//! Remove, Edit, Keep and Forget. Every change calls a tool
//! (`field.derive`, `field.calculate`, `settings.keep_formula`,
//! `settings.forget_formula`, `display.color_by`,
//! `settings.set_default`); the window reads `field.catalogue`'s engine
//! functions.

use std::collections::BTreeMap;

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::calc::{FUNCTIONS, Formula};
use murmel_engine::defaults::Kind;
use murmel_engine::derive::{self, Group, Input, Made, Quantity, Reference, Source};
use serde_json::{Value, json};

use crate::app::MurmelApp;
use crate::style::color;

/// The window's state.
#[derive(Clone, Debug, Default)]
pub struct DerivedWindow {
    pub open: bool,
    /// The field chosen for an input of a quantity; absent, the one the
    /// recipe has or the one found by name.
    pub inputs: BTreeMap<(Quantity, &'static str), String>,
    /// The reference values being edited, and where each came from
    /// (`None`: typed here); taken from the session when the window
    /// opens.
    pub reference: Option<(Reference, [Option<Source>; 3])>,
    /// The formula being written: the field it makes, its text, its
    /// unit.
    pub name: String,
    pub formula: String,
    pub unit: String,
    /// Why the last formula was refused (the tool's message).
    pub error: Option<String>,
    /// Put the keyboard in the formula field on the next frame.
    pub focus_formula: bool,
    /// The derived quantities, or your formulas.
    pub formulas_tab: bool,
}

const REFERENCE_KEYS: [&str; 3] = ["density", "pressure", "velocity"];

impl MurmelApp {
    /// Open the window, its reference values fresh from the session.
    pub(crate) fn open_derived(&mut self) {
        self.derived.open = true;
        self.derived.reference = None;
    }

    /// Open the window with the keyboard in the formula field (the
    /// palette's "Field calculator…").
    pub(crate) fn open_calculator(&mut self) {
        self.open_derived();
        self.derived.formulas_tab = true;
        self.derived.focus_formula = true;
    }

    /// The reference values the window starts from: those of the
    /// pressure coefficient when it is computed, else where new fields
    /// start from.
    fn start_reference(&self) -> (Reference, [Option<Source>; 3]) {
        let own = self.session.fields().and_then(|f| {
            f.recipe(Quantity::PressureCoefficient.field())
                .or_else(|| f.recipe(Quantity::TotalPressure.field()))
                .and_then(|r| r.reference)
        });
        match own {
            Some(r) => (r, [None; 3]),
            None => {
                let (r, from) = derive::start_reference(&self.session);
                (r, from.map(Some))
            }
        }
    }

    pub(crate) fn derived_window(&mut self, ctx: &egui::Context) {
        if !self.derived.open {
            return;
        }
        if self.derived.reference.is_none() {
            self.derived.reference = Some(self.start_reference());
        }
        let p = self.palette(ctx).clone();
        let mut w = self.derived.clone();
        let mut open = true;
        let mut calls: Vec<(&'static str, Value)> = Vec::new();
        egui::Window::new(self.i18n.tr("derived-title"))
            .id(egui::Id::unique("derived-fields"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                let secondary = color(p.text_secondary);
                let error = color(p.status_error);
                let Some(set) = self.session.fields().filter(|f| !f.is_empty()) else {
                    ui.label(self.i18n.tr("derived-no-fields"));
                    return;
                };
                ui.horizontal(|ui| {
                    for (formulas, key) in [
                        (false, "derived-tab-quantities"),
                        (true, "derived-group-formulas"),
                    ] {
                        if ui
                            .selectable_label(w.formulas_tab == formulas, self.i18n.tr(key))
                            .clicked()
                        {
                            w.formulas_tab = formulas;
                        }
                    }
                });
                ui.separator();
                ui.scope(|ui| {
                    if w.formulas_tab {
                        self.formulas_block(ui, set, &mut w, secondary, error, &mut calls);
                        return;
                    }
                    ui.label(
                        RichText::new(self.i18n.tr("derived-note"))
                            .small()
                            .color(secondary),
                    );
                    for group in Group::ALL {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(self.i18n.tr(&format!("derived-group-{}", group.key())))
                                .strong(),
                        );
                        for q in Quantity::ALL.into_iter().filter(|q| q.group() == group) {
                            self.derived_row(ui, set, q, &mut w, secondary, &mut calls);
                            ui.add_space(4.0);
                        }
                        if group == Group::Pressure {
                            self.reference_block(ui, &mut w, secondary, &mut calls);
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(self.i18n.tr("derived-foehncraft"))
                            .small()
                            .color(secondary),
                    );
                });
            });
        let esc = !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        w.open = open && !esc;
        self.derived = w;
        for (tool, input) in calls {
            let made = input.get("remove").is_none();
            let shown = match tool {
                "field.derive" => input["quantity"]
                    .as_str()
                    .and_then(Quantity::parse)
                    .map(|q| q.field().to_string()),
                "field.calculate" => input["name"].as_str().map(str::to_string),
                _ => None,
            };
            let e = self.call_envelope(tool, input);
            if matches!(tool, "field.calculate" | "settings.keep_formula") {
                self.derived.error = e
                    .diagnostics
                    .first()
                    .filter(|_| !e.ok())
                    .map(|d| d.message.clone());
            }
            if e.ok()
                && made
                && let Some(field) = shown
            {
                self.call("display.color_by", json!({"by": "field", "field": field}));
            }
        }
    }

    /// One quantity's row: its name, formula and unit; its inputs; its
    /// state; Compute, Show and Remove.
    fn derived_row(
        &self,
        ui: &mut Ui,
        set: &murmel_engine::field::FieldSet,
        q: Quantity,
        w: &mut DerivedWindow,
        secondary: egui::Color32,
        calls: &mut Vec<(&'static str, Value)>,
    ) {
        let title = self.i18n.tr(&format!("derived-{}", q.key()));
        let recipe = set.recipe(q.field());
        let in_file = recipe.is_none() && set.fields().iter().any(|f| f.name == q.field());
        // Its name, formula and unit on one line; its inputs, state and
        // buttons on the next.
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(&title).strong()).extend());
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{}   [{}]",
                        self.i18n.tr(&format!("derived-formula-{}", q.key())),
                        q.unit()
                    ))
                    .small()
                    .color(secondary),
                )
                .extend(),
            );
        });
        // The inputs, each a choice of the fields that fit.
        let mut chosen: Vec<(Input, Option<String>)> = Vec::new();
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            for &input in q.inputs() {
                let options = derive::candidates(set, input);
                let current = w
                    .inputs
                    .get(&(q, input.key()))
                    .cloned()
                    .filter(|n| options.contains(n))
                    .or_else(|| {
                        let named = match input {
                            Input::Velocity => recipe.and_then(|r| r.velocity.clone()),
                            Input::Pressure => recipe.and_then(|r| r.pressure.clone()),
                        };
                        named.or_else(|| derive::find_input(set, input))
                    });
                let what = self.i18n.tr(&format!("derived-input-{}", input.key()));
                if options.is_empty() {
                    ui.label(
                        RichText::new(self.i18n.tr(&format!("derived-no-{}", input.key())))
                            .color(secondary),
                    );
                } else {
                    let shown = current.clone().unwrap_or_default();
                    let name = format!("{title}, {what}: {shown}");
                    egui::ComboBox::from_id_salt(("derived-input", q.key(), input.key()))
                        .selected_text(format!("{what}: {shown}"))
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for o in &options {
                                if ui
                                    .selectable_label(current.as_deref() == Some(o), o)
                                    .clicked()
                                {
                                    w.inputs.insert((q, input.key()), o.clone());
                                }
                            }
                        })
                        .response
                        .widget_info(|| WidgetInfo::labeled(Role::ComboBox, true, name.clone()));
                }
                chosen.push((input, current));
            }
            let missing: Vec<String> = chosen
                .iter()
                .filter(|(_, f)| f.is_none())
                .map(|(i, _)| self.i18n.tr(&format!("derived-input-{}", i.key())))
                .collect();
            let state = if in_file {
                self.i18n.tr("derived-in-file")
            } else if !missing.is_empty() {
                self.i18n
                    .tr_args("derived-needs", &[("inputs", missing.join(", ").into())])
            } else if recipe.is_some() {
                self.i18n.tr("derived-computed")
            } else {
                String::new()
            };
            ui.label(RichText::new(state).small().color(secondary));
            {
                let named = |key: &str| {
                    self.i18n
                        .tr_args(key, &[("quantity", title.clone().into())])
                };
                if !in_file {
                    let text = self.i18n.tr(if recipe.is_some() {
                        "derived-compute-again"
                    } else {
                        "derived-compute"
                    });
                    let name = named("derived-compute-name");
                    let r = ui
                        .add_enabled(missing.is_empty(), egui::Button::new(text))
                        .on_disabled_hover_text(self.i18n.tr("derived-needs-tip"));
                    r.widget_info(|| WidgetInfo::labeled(Role::Button, missing.is_empty(), &name));
                    if r.clicked() {
                        let mut input = json!({"quantity": q.key()});
                        for (i, f) in &chosen {
                            if let Some(f) = f {
                                input[i.key()] = json!(f);
                            }
                        }
                        if !q.references().is_empty()
                            && let Some((r, _)) = &w.reference
                        {
                            input["reference"] = json!(r);
                        }
                        calls.push(("field.derive", input));
                    }
                }
                if in_file || recipe.is_some() {
                    let name = named("derived-show-name");
                    let r = ui.button(self.i18n.tr("derived-show"));
                    r.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
                    if r.clicked() {
                        calls.push((
                            "display.color_by",
                            json!({"by": "field", "field": q.field()}),
                        ));
                    }
                }
                if recipe.is_some() {
                    let name = named("derived-remove-name");
                    let r = ui.button(self.i18n.tr("derived-remove"));
                    r.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
                    if r.clicked() {
                        calls.push(("field.derive", json!({"quantity": q.key(), "remove": true})));
                    }
                }
            }
        });
    }

    /// The reference values of the pressure quantities: ρ_ref, p_ref and
    /// U_ref, each with where it came from; a right-click keeps one as
    /// your default.
    fn reference_block(
        &self,
        ui: &mut Ui,
        w: &mut DerivedWindow,
        secondary: egui::Color32,
        calls: &mut Vec<(&'static str, Value)>,
    ) {
        let Some((r, from)) = &mut w.reference else {
            return;
        };
        ui.add_space(4.0);
        ui.label(
            RichText::new(self.i18n.tr("derived-reference"))
                .small()
                .color(secondary),
        );
        egui::Grid::new("derived-reference")
            .num_columns(3)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                for (k, key) in REFERENCE_KEYS.into_iter().enumerate() {
                    let name = self.i18n.tr(&format!("derived-ref-{key}"));
                    let label = ui.label(&name);
                    let (value, unit) = match k {
                        0 => (&mut r.density, " kg/m³"),
                        1 => (&mut r.pressure, " Pa"),
                        _ => (&mut r.velocity, " m/s"),
                    };
                    let lo = if k == 1 { -derive::MAX_REFERENCE } else { 1e-9 };
                    let speed = (value.abs() * 0.01).max(1e-3);
                    let resp = ui
                        .add(
                            egui::DragValue::new(value)
                                .range(lo..=derive::MAX_REFERENCE)
                                .speed(speed)
                                .suffix(unit),
                        )
                        .labelled_by(label.id);
                    if resp.changed() {
                        from[k] = None;
                    }
                    let v = *value;
                    resp.context_menu(|ui| {
                        if let Some(a) = self.reference_default_entries(ui, key, v) {
                            calls.push(a);
                        }
                    });
                    let src = match from[k] {
                        Some(s) => format!("derived-from-{}", source_key(s)),
                        None => "derived-from-edited".into(),
                    };
                    ui.label(RichText::new(self.i18n.tr(&src)).small().color(secondary));
                    ui.end_row();
                }
            });
    }

    /// The menu entries that keep reference value `key` (now `value`) as
    /// the one new derived fields start from.
    fn reference_default_entries(
        &self,
        ui: &mut Ui,
        key: &'static str,
        value: f64,
    ) -> Option<(&'static str, Value)> {
        self.kind_default_entries(
            ui,
            Kind::Reference,
            key,
            json!({"kind": Kind::Reference.key(), "values": {key: value}}),
        )
    }
}

impl MurmelApp {
    /// Your formulas: a formula to write, the fields and functions to
    /// click into it, then each kept formula and each field computed
    /// from a formula.
    fn formulas_block(
        &self,
        ui: &mut Ui,
        set: &murmel_engine::field::FieldSet,
        w: &mut DerivedWindow,
        secondary: egui::Color32,
        error: egui::Color32,
        calls: &mut Vec<(&'static str, Value)>,
    ) {
        let tr = |k: &str| self.i18n.tr(k);
        ui.label(RichText::new(tr("calc-note")).small().color(secondary));
        egui::Grid::new("calc-editor")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                for (key, text, width, mono) in [
                    ("calc-name", &mut w.name, 180.0, false),
                    ("calc-formula", &mut w.formula, 400.0, true),
                    ("calc-unit", &mut w.unit, 100.0, false),
                ] {
                    let name = tr(key);
                    ui.label(&name);
                    let mut edit = egui::TextEdit::singleline(text).desired_width(width);
                    if mono {
                        edit = edit.font(egui::TextStyle::Monospace);
                    }
                    let r = ui.add(edit);
                    crate::focus::set_name(ui, r.id, &name);
                    if mono && w.focus_formula {
                        // The keyboard in the field, the cursor at the
                        // end of what was written into it.
                        r.request_focus();
                        let mut st = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
                        let end = egui::text::CCursor::new(text.chars().count());
                        st.cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(end)));
                        st.store(ui.ctx(), r.id);
                        w.focus_formula = false;
                    }
                    ui.end_row();
                }
            });
        let filled = !w.name.trim().is_empty() && !w.formula.trim().is_empty();
        let input = || {
            let mut v = json!({"name": w.name.trim(), "formula": w.formula});
            if !w.unit.trim().is_empty() {
                v["unit"] = json!(w.unit.trim());
            }
            v
        };
        ui.horizontal(|ui| {
            let r = ui
                .add_enabled(filled, egui::Button::new(tr("derived-compute")))
                .on_disabled_hover_text(tr("calc-needs-tip"));
            let name = tr("calc-compute-name");
            r.widget_info(|| WidgetInfo::labeled(Role::Button, filled, &name));
            if r.clicked() {
                calls.push(("field.calculate", input()));
            }
            let r = ui
                .add_enabled(filled, egui::Button::new(tr("calc-keep")))
                .on_hover_text(tr("calc-keep-tip"));
            let name = tr("calc-keep-name");
            r.widget_info(|| WidgetInfo::labeled(Role::Button, filled, &name));
            if r.clicked() {
                calls.push(("settings.keep_formula", input()));
            }
        });
        if let Some(e) = &w.error {
            ui.label(RichText::new(e).small().color(error));
        }
        // The result's fields and the functions, each a click into the
        // formula.
        let mut insert: Option<String> = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("calc-fields")).small().color(secondary));
            for n in set.names() {
                let plain = n
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                    && n.chars().all(|c| c.is_alphanumeric() || c == '_');
                if ui.small_button(n).clicked() {
                    insert = Some(if plain {
                        n.to_string()
                    } else {
                        format!("\"{n}\"")
                    });
                }
            }
            ui.label(RichText::new(tr("calc-built-in")).small().color(secondary));
            for n in ["x", "y", "z", "pos", "pi"] {
                let r = ui
                    .small_button(n)
                    .on_hover_text(tr(&format!("calc-name-{n}")));
                if r.clicked() {
                    insert = Some(n.to_string());
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("calc-functions")).small().color(secondary));
            for f in FUNCTIONS {
                let tip = self.i18n.tr_args(
                    "calc-fn-tip",
                    &[
                        ("call", format!("{}({})", f.name, f.args).into()),
                        ("gives", f.gives.into()),
                        ("summary", tr(&format!("calc-fn-{}", f.name)).into()),
                    ],
                );
                if ui.small_button(f.name).on_hover_text(tip).clicked() {
                    insert = Some(format!("{}(", f.name));
                }
            }
        });
        ui.label(RichText::new(tr("calc-args")).small().color(secondary));
        if let Some(t) = insert {
            let gap = w
                .formula
                .chars()
                .last()
                .is_some_and(|c| !c.is_whitespace() && c != '(' && c != '[');
            if gap {
                w.formula.push(' ');
            }
            w.formula.push_str(&t);
            w.focus_formula = true;
        }
        // Your formulas, then the fields of formulas not kept.
        let mut rows: Vec<(String, Option<&Formula>, Option<&Formula>)> = self
            .session
            .formulas
            .iter()
            .map(|k| (k.name.clone(), Some(k), set.formula(&k.name)))
            .collect();
        for m in set.made() {
            if let Made::Formula(f) = m
                && !rows.iter().any(|(n, _, _)| *n == f.name)
            {
                rows.push((f.name.clone(), None, Some(f)));
            }
        }
        if rows.is_empty() {
            ui.label(RichText::new(tr("calc-none")).small().color(secondary));
            return;
        }
        ui.separator();
        // A long list scrolls; the editor above stays.
        egui::ScrollArea::vertical()
            .id_salt("calc-rows")
            .min_scrolled_height(300.0)
            .max_height(300.0)
            .show(ui, |ui| {
                self.formula_rows(ui, set, rows, w, secondary, calls)
            });
    }

    /// Each kept formula and each field of a formula: its name, formula
    /// and unit, whether it is computed or kept, and its buttons.
    fn formula_rows(
        &self,
        ui: &mut Ui,
        set: &murmel_engine::field::FieldSet,
        rows: Vec<(String, Option<&Formula>, Option<&Formula>)>,
        w: &mut DerivedWindow,
        secondary: egui::Color32,
        calls: &mut Vec<(&'static str, Value)>,
    ) {
        let tr = |k: &str| self.i18n.tr(k);
        for (name, kept, made) in rows {
            let Some(shown) = kept.or(made) else {
                continue;
            };
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&name).strong()).extend());
                let unit = shown
                    .unit
                    .as_deref()
                    .map(|u| format!("   [{u}]"))
                    .unwrap_or_default();
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("= {}{unit}", shown.formula))
                            .small()
                            .color(secondary),
                    )
                    .extend(),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                // A field of the file by this name: nothing to compute.
                let in_file = made.is_none() && set.fields().iter().any(|f| f.name == name);
                let state = match (kept, made) {
                    _ if in_file => tr("calc-in-file"),
                    (Some(k), Some(m)) if k != m => tr("calc-computed-other"),
                    (Some(_), Some(_)) => tr("calc-computed-kept"),
                    (None, Some(_)) => tr("derived-computed"),
                    _ => tr("calc-kept"),
                };
                ui.label(RichText::new(state).small().color(secondary));
                let named =
                    |key: &str| self.i18n.tr_args(key, &[("quantity", name.clone().into())]);
                let mut button_if = |on: bool, text: String, key: &str| {
                    let label = named(key);
                    let r = ui
                        .add_enabled(on, egui::Button::new(text))
                        .on_disabled_hover_text(tr("calc-in-file-tip"));
                    r.widget_info(|| WidgetInfo::labeled(Role::Button, on, &label));
                    r.clicked()
                };
                let again = made.is_some_and(|m| Some(m) == kept.or(made));
                if button_if(
                    !in_file,
                    tr(if again {
                        "derived-compute-again"
                    } else {
                        "derived-compute"
                    }),
                    "derived-compute-name",
                ) {
                    calls.push(("field.calculate", json!(shown)));
                }
                if made.is_some() {
                    if button_if(true, tr("derived-show"), "derived-show-name") {
                        calls.push(("display.color_by", json!({"by": "field", "field": name})));
                    }
                    if button_if(true, tr("derived-remove"), "derived-remove-name") {
                        calls.push(("field.calculate", json!({"name": name, "remove": true})));
                    }
                }
                if button_if(true, tr("calc-edit"), "calc-edit-name") {
                    w.name = shown.name.clone();
                    w.formula = shown.formula.clone();
                    w.unit = shown.unit.clone().unwrap_or_default();
                    w.error = None;
                    w.focus_formula = true;
                }
                if let Some(m) = made
                    && kept != Some(m)
                    && button_if(true, tr("calc-keep"), "calc-keep-row-name")
                {
                    calls.push(("settings.keep_formula", json!(m)));
                }
                if kept.is_some() && button_if(true, tr("calc-forget"), "calc-forget-name") {
                    calls.push(("settings.forget_formula", json!({"name": name})));
                }
            });
        }
    }
}

fn source_key(s: Source) -> &'static str {
    match s {
        Source::Case => "case",
        Source::Defaults => "defaults",
        Source::BuiltIn => "built_in",
    }
}
