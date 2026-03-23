use serde::Deserialize;

/// PVE API version info from /api2/json/version
#[derive(Debug, Deserialize)]
pub struct PveVersion {
    pub version: String,
    pub release: String,
    pub repoid: String,
}
