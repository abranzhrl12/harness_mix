use crate::adapters::{NativeWorkspace, data_query, svg, xlsx};
use harness_application::ports::workspace::ArtifactQueries;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl ArtifactQueries for NativeWorkspace {
    fn xlsx_inspect(
        &self,
        args: &harness_domain::xlsx::XlsxInspectArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| xlsx::inspect(project, args, context),
        )
    }
    fn xlsx_read(
        &self,
        args: &harness_domain::xlsx::XlsxReadArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| xlsx::read(project, args, context),
        )
    }
    fn svg(
        &self,
        args: &harness_domain::svg::SvgArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.read_project(
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| svg::handle_svg(project, args, context),
        )
    }
    fn data_query(
        &self,
        args: &harness_domain::contracts::DataQueryArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        data_query::handle_data_query(&self.project, args, context)
    }
}
