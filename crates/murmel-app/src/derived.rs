//! The Derived fields window (the palette's "Derived fields…", the
//! colour-by menu and the field legend's menu; the maintainer's cards of
//! 2026-11-24: a catalogue with groups, formula, unit and inputs, the
//! quantities whose inputs are missing greyed): each quantity MurmelCraft
//! derives, the fields it reads (found by name, or chosen), whether it is
//! computed or in the file, and Compute, Show and Remove. The reference
//! values of the pressure quantities sit under them, each saying where it
//! came from, and a right-click keeps one as your default. Every change
//! calls a tool (`field.derive`, `display.color_by`,
//! `settings.set_default`); the window reads `field.catalogue`'s engine
//! functions.

use std::collections::BTreeMap;

use egui::{RichText, Role, Ui, WidgetInfo};
use murmel_engine::defaults::Kind;
use murmel_engine::derive::{self, Group, Input, Quantity, Reference, Source};
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
}

const REFERENCE_KEYS: [&str; 3] = ["density", "pressure", "velocity"];

impl MurmelApp {
    /// Open the window, its reference values fresh from the session.
    pub(crate) fn open_derived(&mut self) {
        self.derived.open = true;
        self.derived.reference = None;
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
                let Some(set) = self.session.fields().filter(|f| !f.is_empty()) else {
                    ui.label(self.i18n.tr("derived-no-fields"));
                    return;
                };
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
        let esc = !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        w.open = open && !esc;
        self.derived = w;
        for (tool, input) in calls {
            let shows = tool == "field.derive" && input.get("remove").is_none();
            let quantity = input["quantity"].as_str().and_then(Quantity::parse);
            if self.call(tool, input).is_some()
                && shows
                && let Some(q) = quantity
            {
                self.call(
                    "display.color_by",
                    json!({"by": "field", "field": q.field()}),
                );
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

fn source_key(s: Source) -> &'static str {
    match s {
        Source::Case => "case",
        Source::Defaults => "defaults",
        Source::BuiltIn => "built_in",
    }
}
