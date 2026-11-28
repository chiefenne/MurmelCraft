//! The front ends' handle on the tool registry: the catalogue, this
//! system, the folder for downloads and the way to administrator rights.

use std::path::PathBuf;

use kraxn_core::actions::Elevator;
use kraxn_core::fetch::Source;
use kraxn_core::install::Event;
use kraxn_core::tools::{self, Envelope, Host};
use kraxn_core::{Catalogue, Env, Platform};
use serde_json::Value;

pub struct Runner {
    pub catalogue: Catalogue,
    pub platform: Platform,
    pub env: Env,
    pub work: PathBuf,
    pub elevator: Box<dyn Elevator + Send>,
    /// Instead of GitHub (tests).
    pub source: Option<Source>,
}

impl Runner {
    pub fn call(&mut self, id: &str, input: &Value, progress: &mut dyn FnMut(Event)) -> Envelope {
        let mut host = Host {
            catalogue: &self.catalogue,
            platform: self.platform,
            env: &self.env,
            work: self.work.clone(),
            elevator: self.elevator.as_mut(),
            progress,
            source: self.source.clone(),
        };
        tools::call(&mut host, id, input)
    }

    pub fn quiet(&mut self, id: &str, input: &Value) -> Envelope {
        self.call(id, input, &mut |_| {})
    }
}
