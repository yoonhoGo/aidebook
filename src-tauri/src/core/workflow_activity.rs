//! Shared read-only workflow timeline facade.
pub use super::db::workflow_activity::{WorkflowActivityEvent, WorkflowActivityInput};
use super::{Core, CoreResult};
impl Core {
    pub fn workflow_activity_list(
        &self,
        input: WorkflowActivityInput,
    ) -> CoreResult<Vec<WorkflowActivityEvent>> {
        self.database.workflow_activity_list(input)
    }
}
