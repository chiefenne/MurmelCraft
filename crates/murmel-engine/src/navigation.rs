//! Mouse bindings of the viewport (UI spec, "Viewport, navigation and
//! view cube"). Every binding is data: the four presets are tables, an edit
//! makes a custom table, and the table is saved as `navigation.toml` and
//! changed through the tool `settings.set_bindings`.
//!
//! A button is the side of the mouse that is pressed: `primary` (left, or
//! the left side of an Apple mouse), `secondary` (right side) and `middle`.
//! `cmd` is the Command key on macOS.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Orbit,
    Pan,
    Zoom,
    Roll,
    Pick,
    ContextMenu,
    Fit,
    /// A drag draws a box and zooms into it (the rubber band, as the
    /// toolbar's Zoom to box and W do for one drag). No preset binds it.
    ZoomBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Primary,
    Secondary,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    Drag,
    Click,
    DoubleClick,
    /// Wheel, or a two-finger swipe on a trackpad.
    Scroll,
    Pinch,
}

/// Modifier keys that must be held (all others must be up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Mods {
    pub cmd: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        cmd: false,
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const SHIFT: Mods = Mods {
        shift: true,
        ..Mods::NONE
    };
    pub const CTRL: Mods = Mods {
        ctrl: true,
        ..Mods::NONE
    };
    pub const CMD: Mods = Mods {
        cmd: true,
        ..Mods::NONE
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub action: Action,
    pub gesture: Gesture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<Button>,
    #[serde(default)]
    pub mods: Mods,
}

impl Action {
    pub const ALL: [Action; 8] = [
        Action::Orbit,
        Action::Pan,
        Action::Zoom,
        Action::Roll,
        Action::ZoomBox,
        Action::Pick,
        Action::ContextMenu,
        Action::Fit,
    ];

    /// The name in navigation.toml and of the translation `action-<key>`.
    pub fn key(self) -> &'static str {
        match self {
            Action::Orbit => "orbit",
            Action::Pan => "pan",
            Action::Zoom => "zoom",
            Action::Roll => "roll",
            Action::Pick => "pick",
            Action::ContextMenu => "context_menu",
            Action::Fit => "fit",
            Action::ZoomBox => "zoom_box",
        }
    }
}

impl Button {
    pub const ALL: [Button; 3] = [Button::Primary, Button::Secondary, Button::Middle];

    pub fn key(self) -> &'static str {
        match self {
            Button::Primary => "primary",
            Button::Secondary => "secondary",
            Button::Middle => "middle",
        }
    }
}

impl Gesture {
    pub const ALL: [Gesture; 5] = [
        Gesture::Drag,
        Gesture::Click,
        Gesture::DoubleClick,
        Gesture::Scroll,
        Gesture::Pinch,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Gesture::Drag => "drag",
            Gesture::Click => "click",
            Gesture::DoubleClick => "double_click",
            Gesture::Scroll => "scroll",
            Gesture::Pinch => "pinch",
        }
    }

    /// Whether the gesture is made with a mouse button (the wheel and a
    /// pinch are not).
    pub fn has_button(self) -> bool {
        !matches!(self, Gesture::Scroll | Gesture::Pinch)
    }
}

const fn b(action: Action, gesture: Gesture, button: Option<Button>, mods: Mods) -> Binding {
    Binding {
        action,
        gesture,
        button,
        mods,
    }
}

use Action::*;
use Button::*;
use Gesture::*;

/// The presets of the spec's table.
pub const PRESETS: &[&str] = &["three_button", "apple", "fusion", "paraview"];

fn preset_table(name: &str) -> Option<Vec<Binding>> {
    let alt_cmd = Mods {
        cmd: true,
        alt: true,
        ..Mods::NONE
    };
    Some(match name {
        "three_button" => vec![
            b(Orbit, Drag, Some(Primary), Mods::NONE),
            b(Pan, Drag, Some(Middle), Mods::NONE),
            b(Pan, Drag, Some(Primary), Mods::SHIFT),
            b(Zoom, Scroll, None, Mods::NONE),
            b(Zoom, Pinch, None, Mods::NONE),
            b(Roll, Drag, Some(Primary), Mods::CTRL),
            b(Pick, Click, Some(Primary), Mods::NONE),
            b(ContextMenu, Click, Some(Secondary), Mods::NONE),
            b(Fit, DoubleClick, Some(Middle), Mods::NONE),
        ],
        "apple" => vec![
            b(Orbit, Drag, Some(Primary), Mods::CMD),
            b(Pan, Drag, Some(Primary), Mods::NONE),
            b(Zoom, Drag, Some(Secondary), Mods::CMD),
            b(Zoom, Scroll, None, Mods::NONE),
            b(Zoom, Pinch, None, Mods::NONE),
            b(Roll, Drag, Some(Primary), alt_cmd),
            b(Pick, Click, Some(Primary), Mods::NONE),
            b(ContextMenu, Click, Some(Secondary), Mods::NONE),
            b(ContextMenu, Click, Some(Primary), Mods::CTRL),
        ],
        "fusion" => vec![
            b(Orbit, Drag, Some(Middle), Mods::SHIFT),
            b(Pan, Drag, Some(Middle), Mods::NONE),
            b(Zoom, Scroll, None, Mods::NONE),
            b(Zoom, Pinch, None, Mods::NONE),
            b(Pick, Click, Some(Primary), Mods::NONE),
            b(ContextMenu, Click, Some(Secondary), Mods::NONE),
            b(Fit, DoubleClick, Some(Middle), Mods::NONE),
        ],
        "paraview" => vec![
            b(Orbit, Drag, Some(Primary), Mods::NONE),
            b(Pan, Drag, Some(Middle), Mods::NONE),
            b(Zoom, Drag, Some(Secondary), Mods::NONE),
            b(Zoom, Scroll, None, Mods::NONE),
            b(Zoom, Pinch, None, Mods::NONE),
            b(Roll, Drag, Some(Primary), Mods::CTRL),
            b(Pick, Click, Some(Primary), Mods::NONE),
        ],
        _ => return None,
    })
}

/// The navigation settings (`navigation.toml`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Navigation {
    pub format: String,
    /// A preset name, or `custom` after an edit.
    pub preset: String,
    #[serde(default = "yes")]
    pub zoom_to_cursor: bool,
    /// Scroll up moves away instead of closer.
    #[serde(default)]
    pub invert_zoom: bool,
    /// Degrees of orbit per pixel of drag.
    #[serde(default = "orbit_speed")]
    pub orbit_speed: f64,
    /// Zoom factor exponent per scroll point or drag pixel.
    #[serde(default = "zoom_speed")]
    pub zoom_speed: f64,
    /// A press that moves less than this many pixels is a click.
    #[serde(default = "click_slop")]
    pub click_slop_px: f64,
    pub bindings: Vec<Binding>,
}

pub const FORMAT: &str = "murmelcraft.navigation";

fn yes() -> bool {
    true
}
fn orbit_speed() -> f64 {
    0.4
}
fn zoom_speed() -> f64 {
    0.004
}
fn click_slop() -> f64 {
    4.0
}

impl Default for Navigation {
    fn default() -> Self {
        Self::preset("three_button").unwrap_or_else(|| Self::custom(Vec::new()))
    }
}

impl Navigation {
    pub fn preset(name: &str) -> Option<Self> {
        Some(Self {
            preset: name.to_string(),
            ..Self::custom(preset_table(name)?)
        })
    }

    pub fn custom(bindings: Vec<Binding>) -> Self {
        Self {
            format: FORMAT.into(),
            preset: "custom".into(),
            zoom_to_cursor: true,
            invert_zoom: false,
            orbit_speed: orbit_speed(),
            zoom_speed: zoom_speed(),
            click_slop_px: click_slop(),
            bindings,
        }
    }

    /// The default for a platform: the Apple preset on macOS when no middle
    /// button has been seen, the three-button scheme elsewhere.
    pub fn platform_default(macos: bool) -> Self {
        Self::preset(if macos { "apple" } else { "three_button" }).unwrap_or_default()
    }

    /// The action of a drag that started with `button` and `mods`.
    pub fn drag(&self, button: Button, mods: Mods) -> Option<Action> {
        self.find(Drag, Some(button), mods)
    }

    /// The action of a click without movement.
    pub fn click(&self, button: Button, mods: Mods) -> Option<Action> {
        self.find(Click, Some(button), mods)
    }

    pub fn double_click(&self, button: Button, mods: Mods) -> Option<Action> {
        self.find(DoubleClick, Some(button), mods)
    }

    pub fn scroll(&self, mods: Mods) -> Option<Action> {
        self.find(Scroll, None, mods)
    }

    pub fn pinch(&self, mods: Mods) -> Option<Action> {
        self.find(Pinch, None, mods)
    }

    fn find(&self, gesture: Gesture, button: Option<Button>, mods: Mods) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| b.gesture == gesture && b.button == button && b.mods == mods)
            .map(|b| b.action)
    }

    /// The bindings of one action, for hints and the settings table.
    pub fn of(&self, action: Action) -> impl Iterator<Item = &Binding> {
        self.bindings.iter().filter(move |b| b.action == action)
    }

    /// Two bindings with the same gesture, button and modifiers.
    pub fn conflicts(&self) -> Vec<(Binding, Binding)> {
        let mut out = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for c in &self.bindings[i + 1..] {
                if a.gesture == c.gesture && a.button == c.button && a.mods == c.mods {
                    out.push((*a, *c));
                }
            }
        }
        out
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let n: Navigation = toml::from_str(text).map_err(|e| e.to_string())?;
        if n.format != FORMAT {
            return Err(format!("format is {}, not {FORMAT}", n.format));
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the spec's table. Default: left orbit, middle pan, wheel
    /// zoom. Apple: Cmd + click drag orbits, Cmd + right-side drag zooms,
    /// a plain drag pans (decided 2026-11-23).
    #[test]
    fn presets_follow_the_spec() {
        let d = Navigation::preset("three_button").unwrap();
        assert_eq!(d.drag(Primary, Mods::NONE), Some(Orbit));
        assert_eq!(d.drag(Middle, Mods::NONE), Some(Pan));
        assert_eq!(d.drag(Primary, Mods::SHIFT), Some(Pan));
        assert_eq!(d.scroll(Mods::NONE), Some(Zoom));
        assert_eq!(d.click(Primary, Mods::NONE), Some(Pick));

        let a = Navigation::preset("apple").unwrap();
        assert_eq!(a.drag(Primary, Mods::CMD), Some(Orbit));
        assert_eq!(a.drag(Secondary, Mods::CMD), Some(Zoom));
        assert_eq!(a.drag(Primary, Mods::NONE), Some(Pan));
        assert_eq!(a.click(Primary, Mods::CTRL), Some(ContextMenu));

        let f = Navigation::preset("fusion").unwrap();
        assert_eq!(f.drag(Middle, Mods::SHIFT), Some(Orbit));
        assert_eq!(f.drag(Primary, Mods::NONE), None);

        let p = Navigation::preset("paraview").unwrap();
        assert_eq!(p.drag(Secondary, Mods::NONE), Some(Zoom));
        assert!(Navigation::preset("nope").is_none());
    }

    /// Invariant: no preset binds one input twice, and every preset can
    /// orbit, pan and zoom.
    #[test]
    fn presets_are_complete_and_conflict_free() {
        for name in PRESETS {
            let n = Navigation::preset(name).unwrap();
            assert!(n.conflicts().is_empty(), "{name}");
            for a in [Orbit, Pan, Zoom] {
                assert!(n.of(a).next().is_some(), "{name} lacks {a:?}");
            }
        }
    }

    /// Invariant: the names the editor translates are the names of
    /// navigation.toml, and the lists hold every value. Breaks if a new
    /// action is left out of ALL (the editor could not choose it).
    #[test]
    fn names_match_the_file() {
        for a in Action::ALL {
            assert_eq!(serde_json::to_value(a).unwrap(), a.key());
        }
        for b in Button::ALL {
            assert_eq!(serde_json::to_value(b).unwrap(), b.key());
        }
        for g in Gesture::ALL {
            assert_eq!(serde_json::to_value(g).unwrap(), g.key());
        }
        assert_eq!(Action::ALL.len(), 8);
    }

    /// Invariant: navigation.toml round-trips, and a file of another
    /// format is refused.
    #[test]
    fn toml_round_trip() {
        let n = Navigation::preset("apple").unwrap();
        let t = n.to_toml().unwrap();
        assert_eq!(Navigation::from_toml(&t).unwrap(), n);
        assert!(Navigation::from_toml("format = \"x\"\npreset = \"a\"\nbindings = []").is_err());
    }
}
