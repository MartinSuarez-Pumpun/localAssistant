use serde::{Deserialize, Serialize};

/// Descriptor de plugin tal como llega del servidor (/plugins/).
#[derive(Clone, Serialize, Deserialize, Default,PartialEq)]
pub struct PluginInfo {
    pub id:   String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
}

fn default_icon() -> String { "extension".into() }
