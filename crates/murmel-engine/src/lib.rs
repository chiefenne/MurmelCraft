//! The engine of MurmelCraft: everything the viewer knows and does without
//! a window or a GPU. The scene (the open mesh and its triangulated
//! surface), the camera, the navigation bindings, the design tokens and the
//! settings files. Front ends (the egui window, the CLI, the MCP server)
//! change this state only through the tools of `murmel-tools`.

pub mod camera;
pub mod clip;
pub mod cube;
pub mod display;
pub mod inspect;
pub mod navigation;
pub mod pick;
pub mod quality;
pub mod session;
pub mod settings;
pub mod surface;
pub mod theme;

pub use camera::{Camera, Projection, ScreenAxis, StandardView};
pub use clip::{Clip, ClipMode};
pub use display::{ColorBy, Display, Item, PickTarget, RenderMode, Visibility};
pub use navigation::Navigation;
pub use quality::{Metric, Quality};
pub use session::{OpenMesh, Session, Validity};
pub use surface::Surface;
pub use theme::{MeshColors, Rgb, Theme, Tokens};
