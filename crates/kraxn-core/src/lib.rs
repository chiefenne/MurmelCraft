//! Kraxn, the installer of MurmelCraft, KoflCraft and FoehnCraft, without
//! its window: the catalogue of products, the folders of each system and
//! scope, release lists (GitHub or a folder of downloads), downloads with
//! their SHA-256 check, unpacking, install steps, several versions side by
//! side with a default, and removal. Every step is a tool of the registry
//! ([`tools`]), shared by the window, the text mode and plan files.
//!
//! *Kraxn* is Tyrolean for the carrying frame on your back.

pub mod actions;
pub mod archive;
pub mod build;
pub mod catalogue;
pub mod desktop;
pub mod diag;
pub mod envfiles;
pub mod extras;
pub mod fetch;
pub mod install;
pub mod installed;
pub mod layout;
pub mod plan;
pub mod platform;
pub mod release;
pub mod tools;

pub use catalogue::{Catalogue, Product};
pub use diag::{Diagnostic, Res};
pub use layout::{Env, Layout, Scope};
pub use plan::{Origin, Plan, Want};
pub use platform::Platform;
