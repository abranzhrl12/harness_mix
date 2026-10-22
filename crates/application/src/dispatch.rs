//! Exhaustive request routing and response-wide application policy.
mod artifacts;
mod diagnostics;
mod editing;
mod queries;
mod sessions;
mod sql;

use crate::Application;
use crate::ports::{SessionRepository, Workspace};
use harness_domain::{contracts::Request, execution::OperationContext};
use serde_json::{Value, json};

impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub(super) fn execute_admitted(&self, request: Request, context: &OperationContext) -> Value {
        let mutating = request.is_mutating();
        let name = request.name();
        let request_arguments = request.arguments();
        let mut lease = None;
        let result = context.check().and_then(|()| match request {
            Request::Explore(args) => queries::explore(self, args, context, &mut lease),
            Request::Bootstrap(args) => sessions::bootstrap(self, args, context, &mut lease),
            Request::ExternalScope(args) => {
                sessions::external_scope(self, args, context, &mut lease)
            }
            Request::Config(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.config(&args, context)),
            Request::Logs(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.metrics.logs.read(&args)),
            Request::Batch(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.batch(args, context)),
            Request::Doctor(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.doctor(&args, context)),
            Request::GitStatus(args) => queries::git_status(self, args, context, &mut lease),
            Request::GitDiff(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.git_diff(&args, context)),
            Request::SqlStatus(args) => sql::with_session(
                self,
                args.session.as_deref(),
                context,
                &mut lease,
                |workspace, context| workspace.sql_status(&args, context),
            ),
            Request::SqlSearch(args) => sql::with_session(
                self,
                args.session.as_deref(),
                context,
                &mut lease,
                |workspace, context| workspace.sql_search(&args, context),
            ),
            Request::SqlMap(args) => sql::with_session(
                self,
                args.session.as_deref(),
                context,
                &mut lease,
                |workspace, context| workspace.sql_map(&args, context),
            ),
            Request::SqlDescribe(args) => sql::with_session(
                self,
                args.session.as_deref(),
                context,
                &mut lease,
                |workspace, context| workspace.sql_describe(&args, context),
            ),
            Request::SqlQuery(args) => sql::with_session(
                self,
                args.session.as_deref(),
                context,
                &mut lease,
                |workspace, context| workspace.sql_query(&args, context),
            ),
            Request::XlsxInspect(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.xlsx_inspect(&args, context)),
            Request::XlsxRead(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.xlsx_read(&args, context)),
            Request::Svg(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.svg(&args, context)),
            Request::Handover(args) => sessions::handover(self, args, context, &mut lease),
            Request::ProcessStatus(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.process_status(&args, context)),
            Request::Diagnostics(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.diagnostics(&args, context)),
            Request::DataQuery(args) => artifacts::data_query(self, args, context, &mut lease),
            Request::PatchPreview(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.patch_preview(&args, context)),
            Request::Outline(args) => queries::outline(self, args, context, &mut lease),
            Request::ReadSymbol(args) => queries::read_symbol(self, args, context, &mut lease),
            Request::VueExtractPreview(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.vue_extract_preview(&args, context)),
            Request::Replace(args) => {
                editing::replace(self, args, context, name, &request_arguments, &mut lease)
            }
            Request::Write(args) => {
                editing::write(self, args, context, name, &request_arguments, &mut lease)
            }
            Request::Move(args) => {
                editing::move_file(self, args, context, name, &request_arguments, &mut lease)
            }
            Request::Delete(args) => {
                editing::delete_file(self, args, context, name, &request_arguments, &mut lease)
            }
            Request::Proposal(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.prepare_proposal(&args, context)),
            Request::Read(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.read(&args, context)),
            Request::Search(args) => self
                .lease(args.session.as_deref(), &mut lease)
                .and_then(|()| self.workspace.search(&args, context)),
            Request::Ignore(args) => {
                editing::ignore(self, args, context, name, &request_arguments, &mut lease)
            }
            Request::Verify(args) => {
                editing::verify(self, args, context, name, &request_arguments, &mut lease)
            }
            Request::Tasks(args) => sessions::tasks(self, args, context, &mut lease),
            Request::Checkpoint(args) => sessions::checkpoint(self, args, context, &mut lease),
            Request::Detail(args) => artifacts::detail(self, args, &mut lease),
        });
        let result = result.and_then(|value| {
            // Search can return an explicit partial result on cancellation/deadline.
            if !mutating
                && value.get("cancelled") != Some(&json!(true))
                && value.get("timedOut") != Some(&json!(true))
            {
                context.check()?;
            }
            Ok(value)
        });
        let result = result.map(|mut value| {
            if name == "harness_doctor" {
                diagnostics::enrich_doctor(self, &mut value);
            } else if name == "harness_checkpoint" {
                let stats = self.metrics.snapshot();
                value["stats"] = stats.clone();
                if let Some(full) = value.get_mut("_detail") {
                    full["stats"] = stats;
                }
            }
            value
        });
        let output = self.output(name, &request_arguments, result);
        drop(lease);
        output
    }
}
