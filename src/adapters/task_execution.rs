//! Native verifier bridge; the scheduler never imports language-specific adapters.
use super::{process::NativeProcessRunner, verify};
use harness_domain::{
    Failure, contracts::VerifyArgs, execution::OperationContext, process::ProcessOutcome,
};
use harness_jobs::{PreparedVerification, VerificationExecutor};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) struct NativeVerificationExecutor;

struct NativeVerification {
    root: PathBuf,
    args: VerifyArgs,
    plan: verify::BackgroundVerificationPlan,
}

impl VerificationExecutor for NativeVerificationExecutor {
    fn prepare(
        &self,
        root: &Path,
        args: &VerifyArgs,
        timeout: Option<Duration>,
    ) -> Result<Box<dyn PreparedVerification>, Failure> {
        Ok(Box::new(NativeVerification {
            root: root.to_owned(),
            args: args.clone(),
            plan: verify::background_plan(root, args, timeout)?,
        }))
    }
}

impl PreparedVerification for NativeVerification {
    fn execute(
        &self,
        context: &OperationContext,
        observer: &mut dyn FnMut(bool, &[u8]),
    ) -> Result<ProcessOutcome, Failure> {
        match &self.plan {
            verify::BackgroundVerificationPlan::Process(spec) => {
                NativeProcessRunner::new(&self.root)?.run_observed(spec, context, observer)
            }
            verify::BackgroundVerificationPlan::TypeScriptSyntax => {
                let outcome = verify::run_typescript_syntax(&self.root, &self.args, context)?;
                for chunk in outcome.stdout.chunks(8192) {
                    observer(false, chunk);
                }
                for chunk in outcome.stderr.chunks(8192) {
                    observer(true, chunk);
                }
                Ok(outcome)
            }
        }
    }

    fn annotate(&self, outcome: &ProcessOutcome, result: &mut Value) {
        if matches!(
            &self.plan,
            verify::BackgroundVerificationPlan::TypeScriptSyntax
        ) {
            result["language"] = json!("typescript");
            result["verificationMode"] = json!("typescript_syntax_only");
            result["typeCheckPerformed"] = json!(false);
            result["codeExecuted"] = json!(false);
            result["testCountVerified"] = json!(false);
        }
        if self.args.language.eq_ignore_ascii_case("angular") {
            let stdout = String::from_utf8_lossy(&outcome.stdout);
            let report = verify::angular::report(&stdout);
            verify::angular::annotate(result, report.as_ref());
            if let Some(report) = &report {
                let display = verify::angular::display(report);
                result["output"] = json!(display.chars().take(12000).collect::<String>());
                result["outputTruncated"] = json!(display.chars().count() > 12000);
            }
        }
    }
}

#[cfg(test)]
#[path = "task_execution_tests.rs"]
mod tests;
