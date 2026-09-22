pub use super::db::workflow_links::*;
use super::{Core, CoreResult};
impl Core {
    pub fn workflow_link_add(
        &self,
        input: WorkflowLinkAddInput,
    ) -> CoreResult<WorkflowLinkMutation> {
        self.database.workflow_link_add(input)
    }
    pub fn workflow_link_remove(
        &self,
        input: WorkflowLinkRemoveInput,
    ) -> CoreResult<WorkflowLinkMutation> {
        self.database.workflow_link_remove(input)
    }
    pub fn workflow_link_list(
        &self,
        input: WorkflowLinkListInput,
    ) -> CoreResult<Vec<WorkflowLink>> {
        self.database.workflow_link_list(input)
    }
    pub fn workflow_import_preview(
        &self,
        input: WorkflowImportInput,
    ) -> CoreResult<WorkflowImportPreview> {
        self.database.workflow_import_preview(input)
    }
    pub fn workflow_import_apply(
        &self,
        input: WorkflowImportApplyInput,
    ) -> CoreResult<WorkflowImportResult> {
        self.database.workflow_import_apply(input)
    }
}
