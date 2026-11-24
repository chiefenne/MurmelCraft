//! The engine of MurmelCraft: everything the viewer knows and does without
//! a window or a GPU. The scene (the open mesh and its triangulated
//! surface), the camera, the tool journal (undo and redo), the navigation bindings, the design tokens, the
//! settings files and session files, and the translations. Front ends (the egui window, the CLI, the MCP server)
//! change this state only through the tools of `murmel-tools`.

pub mod animation;
pub mod blocks;
pub mod calc;
pub mod camera;
pub mod chart;
pub mod checked;
pub mod clip;
pub mod colormap;
pub mod cube;
pub mod defaults;
pub mod derive;
pub mod display;
pub mod extract;
pub mod field;
pub mod glyph;
mod gradient;
pub mod history;
pub mod i18n;
pub mod inspect;
pub mod journal;
pub mod legend;
pub mod measure;
pub mod memory;
pub mod navigation;
pub mod parcel;
pub mod pick;
pub mod quality;
pub mod region;
pub mod series;
pub mod session;
pub mod session_file;
pub mod settings;
pub mod stream;
pub mod surface;
mod tets;
pub mod theme;
pub mod turbo;
pub mod walls;

pub use camera::{Camera, Projection, ScreenAxis, StandardView};
pub use clip::{Clip, ClipMode};
pub use cube::{CubeSettings, Placement};
pub use defaults::Defaults;
pub use display::{ColorBy, Display, Item, PickTarget, RenderMode, Visibility};
pub use extract::{Iso, IsoColor, MeshLook, Slice};
pub use field::{FieldChoice, FieldMode, FieldPaint, FieldSet, FieldStyle, RangeMode};
pub use glyph::Arrows;
pub use journal::{Journal, Snapshot};
pub use legend::Legend;
pub use measure::{Anchor, MeasureKind, Measurement, Probe, SnapKind};
pub use navigation::Navigation;
pub use quality::{Metric, Quality};
pub use region::{Region, RegionCells, Shape};
pub use session::{
    Fit, Loaded, OpenMesh, OpenMode, Session, Validity, load, load_checked, load_with,
};
pub use session_file::SessionFile;
pub use stream::{Direction, Look, Seed, Stream};
pub use surface::Surface;
pub use theme::{MeshColors, Rgb, Theme, Tokens};
