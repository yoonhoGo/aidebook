//! Public dashboard facade shared by all transports.
pub use super::db::workflow_dashboard::{
    DashboardEntry, DashboardInput, DashboardPage, DashboardSection,
};
use super::{Core, CoreResult};
impl Core {
    pub fn dashboard_get(&self, input: DashboardInput) -> CoreResult<DashboardPage> {
        self.database.dashboard_get(input)
    }
}
