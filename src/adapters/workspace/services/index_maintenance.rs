use super::IndexMaintenanceService;
use crate::adapters::explore;
use harness_domain::{Failure, contracts::ExploreArgs, execution::OperationContext};
use serde_json::Value;

impl IndexMaintenanceService {
    pub(in crate::adapters) fn explore_synchronized(
        &self,
        args: &ExploreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _guard = self
            .edit_lock
            .lock()
            .map_err(|_| ("INTERNAL", "Workspace mutation lock unavailable"))?;
        context.check()?;
        explore::synchronized(&self.project, args, context)
    }
}
