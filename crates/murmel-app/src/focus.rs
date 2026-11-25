//! Keyboard focus (UI spec, Principles: keyboard use and an accessibility
//! tree): the regions F6 and Shift + F6 move between, the focus ring
//! shown after keyboard use, and the names of the panes in the AccessKit
//! tree.

use egui::{Context, Id, Rect, Response, Ui};

/// The regions of the window, in the order F6 visits them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Title,
    Toolbar,
    Tree,
    Viewport,
    /// The time series' player bar, below the viewport while a series is
    /// open.
    Player,
    /// The chart and statistics panel, above the player bar while a
    /// run's tables are open.
    Charts,
    Properties,
    /// The Help panel (the manual), right of the properties while open.
    Help,
    Status,
}

impl Region {
    pub const ORDER: [Region; 9] = [
        Region::Title,
        Region::Toolbar,
        Region::Tree,
        Region::Viewport,
        Region::Player,
        Region::Charts,
        Region::Properties,
        Region::Help,
        Region::Status,
    ];

    /// The translation key of the region's accessible name.
    pub fn key(self) -> &'static str {
        match self {
            Region::Title => "region-title",
            Region::Toolbar => "region-toolbar",
            Region::Tree => "region-tree",
            Region::Viewport => "region-viewport",
            Region::Player => "region-player",
            Region::Charts => "region-charts",
            Region::Properties => "region-properties",
            Region::Help => "region-help",
            Region::Status => "region-status",
        }
    }

    fn index(self) -> usize {
        Region::ORDER.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// Where each region was drawn and the widget F6 gives the focus to.
#[derive(Clone, Copy, Default)]
struct Seen {
    rect: Option<Rect>,
    first: Option<Id>,
}

/// The focus state the window keeps between frames.
#[derive(Default)]
pub struct Focus {
    /// The focus ring shows: the keyboard moved the focus (Tab, F6) and
    /// no pointer button was pressed since (like CSS `:focus-visible`).
    pub visible: bool,
    /// The widget with the focus at the end of the last frame. egui drops
    /// the focus on Esc before the window reads the keys, so the keys
    /// look at this one.
    pub last: Option<Id>,
    /// The viewport canvas, which keeps the arrow keys for orbiting.
    pub viewport: Option<Id>,
    /// The regions as drawn last frame, and as drawn this frame.
    shown: [Seen; 9],
    drawing: [Seen; 9],
}

impl Focus {
    /// Start a frame: a pointer press hides the ring, Tab and F6 show it.
    pub fn begin(&mut self, ctx: &Context) {
        let (pressed, keyboard) = ctx.input(|i| {
            let pressed = i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerButton { pressed: true, .. } | egui::Event::Touch { .. }
                )
            });
            let keyboard = i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Tab | egui::Key::F6,
                        pressed: true,
                        ..
                    }
                )
            });
            (pressed, keyboard)
        });
        if pressed {
            self.visible = false;
        }
        if keyboard {
            self.visible = true;
        }
    }

    /// Remember where `region` was drawn this frame.
    pub fn region(&mut self, region: Region, rect: Rect) {
        self.drawing[region.index()].rect = Some(rect);
    }

    /// Offer `resp` as the widget F6 focuses in `region`; the first one
    /// offered in a frame wins, so offer them in the order Tab visits.
    /// A disabled widget cannot take the focus and is passed over.
    pub fn offer(&mut self, region: Region, resp: &Response) {
        let seen = &mut self.drawing[region.index()];
        if seen.first.is_none() && resp.enabled() && resp.sense.is_focusable() {
            seen.first = Some(resp.id);
        }
    }

    /// End a frame: keep what was drawn and the focused widget.
    pub fn end(&mut self, ctx: &Context) {
        self.shown = std::mem::take(&mut self.drawing);
        self.last = ctx.memory(|m| m.focused());
    }

    /// The region the focused widget is in, by where it was drawn.
    pub fn current(&self, ctx: &Context) -> Option<Region> {
        let id = self.last?;
        if let Some(r) = Region::ORDER
            .into_iter()
            .find(|r| self.shown[r.index()].first == Some(id))
        {
            return Some(r);
        }
        let at = ctx.read_response(id)?.rect.center();
        Region::ORDER.into_iter().find(|r| {
            self.shown[r.index()]
                .rect
                .is_some_and(|rect| rect.contains(at))
        })
    }

    /// The region after (or before) the current one that was drawn and
    /// has a widget to focus, and that widget. With nothing focused F6
    /// starts at the first region and Shift + F6 at the last.
    pub fn next(&self, ctx: &Context, forward: bool) -> Option<(Region, Id)> {
        let n = Region::ORDER.len();
        let start = match self.current(ctx) {
            Some(r) => r.index(),
            None if forward => n - 1,
            None => 0,
        };
        (1..=n)
            .map(|k| {
                if forward {
                    (start + k) % n
                } else {
                    (start + n - k) % n
                }
            })
            .find_map(|i| self.shown[i].first.map(|id| (Region::ORDER[i], id)))
    }

    /// Whether the arrow keys and Esc belong to the focused widget (a
    /// tree row, a slider, a button the keyboard reached) instead of the
    /// viewport: the focus ring shows on a widget other than the canvas.
    pub fn widget_keys(&self) -> bool {
        self.visible && self.last.is_some() && self.last != self.viewport
    }
}

/// Give the pane (side panel, bar or central panel) that `ui` is drawn
/// in its accessible name.
pub fn name_pane(ui: &Ui, name: &str) {
    let Some(pane) = ui
        .stack()
        .iter()
        .find(|s| s.kind().is_some_and(|k| k.is_panel()))
    else {
        return;
    };
    let id = pane.unique_id;
    ui.ctx().accesskit_node_builder(id, |n| n.set_label(name));
}

/// Give the widget `id` the accessible name `name`, keeping the rest of
/// its node (role, value, selection) as the widget set it.
pub fn set_name(ui: &Ui, id: Id, name: &str) {
    ui.ctx().accesskit_node_builder(id, |n| n.set_label(name));
}

/// Draw the focus ring around the focused widget, in the accent colour
/// (2 px, outside the widget; inside it for the viewport, which fills its
/// pane).
pub fn ring(ctx: &Context, focus: &Focus, accent: egui::Color32) {
    if !focus.visible {
        return;
    }
    let Some(id) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let Some(r) = ctx.read_response(id) else {
        return;
    };
    let (rect, kind) = if Some(id) == focus.viewport {
        (r.rect.shrink(1.0), egui::StrokeKind::Inside)
    } else {
        (r.interact_rect.expand(1.0), egui::StrokeKind::Outside)
    };
    ctx.layer_painter(r.layer_id)
        .rect_stroke(rect, 6.0, egui::Stroke::new(2.0, accent), kind);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(rect: Rect, first: u64) -> Seen {
        Seen {
            rect: Some(rect),
            first: Some(Id::unique(first)),
        }
    }

    /// Invariant: F6 visits the regions that have a widget in the order
    /// title, toolbar, tree, viewport, properties, status and wraps
    /// round; Shift + F6 goes back; a region without a widget (a hidden
    /// panel) is skipped. Breaks when the order or the skip changes; the
    /// window test checks the same on the real window, this one without
    /// egui's focus.
    #[test]
    fn f6_cycles_and_skips() {
        let ctx = Context::default();
        let r = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(10.0, 10.0));
        let mut f = Focus {
            shown: [
                seen(r, 1),
                seen(r, 2),
                Seen::default(),
                seen(r, 4),
                Seen::default(),
                Seen::default(),
                seen(r, 5),
                Seen::default(),
                seen(r, 6),
            ],
            ..Focus::default()
        };
        assert_eq!(f.next(&ctx, true), Some((Region::Title, Id::unique(1))));
        assert_eq!(f.next(&ctx, false), Some((Region::Status, Id::unique(6))));
        f.last = Some(Id::unique(2));
        assert_eq!(f.next(&ctx, true), Some((Region::Viewport, Id::unique(4))));
        f.last = Some(Id::unique(6));
        assert_eq!(f.next(&ctx, true), Some((Region::Title, Id::unique(1))));
        f.last = Some(Id::unique(4));
        assert_eq!(f.next(&ctx, false), Some((Region::Toolbar, Id::unique(2))));
    }
}
