use serde::Serialize;

/// The small, transport-neutral status returned by the local core.
///
/// UI, CLI, and MCP will use the same core boundary as those surfaces are
/// added. Connector and persistence contracts deliberately come later.
#[derive(Debug, Serialize)]
pub struct CoreStatus {
    pub product: &'static str,
    pub version: &'static str,
    pub persistence: &'static str,
    pub connectors: &'static str,
}

pub fn status() -> CoreStatus {
    CoreStatus {
        product: "Aidebook",
        version: env!("CARGO_PKG_VERSION"),
        persistence: "not configured",
        connectors: "not configured",
    }
}
