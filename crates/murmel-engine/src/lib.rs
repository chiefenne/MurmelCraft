//! The engine of MurmelCraft: everything the viewer knows and does without
//! a window or a GPU. The scene (the open mesh and its triangulated
//! surface), the camera, the navigation bindings, the design tokens and the
//! settings files. Front ends (the egui window, the CLI, the MCP server)
//! change this state only through the tools of `murmel-tools`.

pub mod camera;
pub mod display;
pub mod inspect;
pub mod navigation;
pub mod session;
pub mod settings;
pub mod surface;
pub mod theme;

pub use camera::{Camera, Projection, StandardView};
pub use display::{ColorBy, Display, Item, Visibility};
pub use navigation::Navigation;
pub use session::{OpenMesh, Session, Validity};
pub use surface::Surface;
pub use theme::{MeshColors, Rgb, Theme, Tokens};
