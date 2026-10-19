//! Named workspace layouts live in the frontend's configuration store.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::EffectcraftApp;
use crate::dock::{DockNode, Floating};

const STORE: &str = "workspaces.json";

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct SavedWorkspaces {
    saved_workspaces: BTreeMap<String, DockNode>,
    saved_floating: BTreeMap<String, Vec<Floating>>,
}

impl EffectcraftApp {
    pub(crate) fn with_saved_workspaces(mut self) -> Self {
        let Some(data) = self.session.config.as_ref().and_then(|store| store.read(STORE)) else { return self };
        let saved = match serde_json::from_str::<SavedWorkspaces>(&data) {
            Ok(saved) => saved,
            Err(error) => {
                log::warn!("load workspaces: {error}");
                return self;
            }
        };
        self.ui.saved_workspaces = saved.saved_workspaces;
        self.ui.saved_floating = saved.saved_floating;
        let name = self.ui.workspace.clone();
        self.set_workspace(&name);
        self
    }

    pub(crate) fn save_workspaces(&self) -> Result<(), String> {
        let Some(store) = &self.session.config else { return Ok(()) };
        let saved = SavedWorkspaces { saved_workspaces: self.ui.saved_workspaces.clone(), saved_floating: self.ui.saved_floating.clone() };
        let data = serde_json::to_string_pretty(&saved).map_err(|error| format!("save workspaces: {error}"))?;
        store.write(STORE, &data).map_err(|error| format!("save workspaces: {error}"))
    }
}
