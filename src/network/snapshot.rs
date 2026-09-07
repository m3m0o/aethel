use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NetworkSnapshot {
    pub interface: String,
    pub interface_index: u32,
    pub routes: Vec<RouteState>,
    pub sysctls: BTreeMap<String, String>,
    pub capabilities: CapabilityState,
    pub ndp_backend: BackendState,
    pub restoration_available: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RouteState {
    pub destination: Option<String>,
    pub kind: String,
    pub table: u32,
    pub output_interface: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapabilityState {
    pub net_admin: bool,
    pub net_raw: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackendState {
    Native,
    Ndppd,
}
