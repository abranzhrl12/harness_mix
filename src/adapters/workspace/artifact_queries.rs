use crate::adapters::{NativeWorkspace, data_query, svg, xlsx};
use harness_application::ports::workspace::ArtifactQueries;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl ArtifactQueries for NativeWorkspace {
    fn word_preview(&self, args: &harness_domain::word::WordPreviewArgs, context: &OperationContext) -> Result<Value, Failure> {
        args.validate()?;
        let bytes = self.project.symbol_bytes_with_context(&args.path, context)?;
        context.check()?;
        let document = rdocx::Document::from_bytes(&bytes).map_err(|_| ("WORD_PREVIEW", "Unable to open DOCX"))?;
        let layout = document.layout().map_err(|_| ("WORD_PREVIEW", "Unable to lay out DOCX"))?;
        let page_count = layout.layout.pages.len();
        let page = layout.layout.pages.get((args.page - 1) as usize).ok_or(("ARGUMENT", "Page does not exist"))?;
        if !page.width.is_finite() || !page.height.is_finite() || page.width <= 0.0 || page.height <= 0.0
            || page.width > 2000.0 || page.height > 2000.0 {
            return Err(("SIZE", "Page dimensions exceed preview limits"));
        }
        let png = document.render_page_to_png((args.page - 1) as usize, 100.0)
            .map_err(|_| ("WORD_PREVIEW", "Unable to render DOCX page"))?
            .ok_or(("ARGUMENT", "Page does not exist"))?;
        context.check()?;
        if png.len() > 8 * 1024 * 1024 { return Err(("SIZE", "Rendered page exceeds 8 MiB")); }
        use base64::Engine as _;
        Ok(serde_json::json!({"path":args.path,"page":args.page,"pageCount":page_count,"mimeType":"image/png",
            "imageBase64":base64::engine::general_purpose::STANDARD.encode(png),
            "notice":"Preview rendered by rdocx; pagination and layout may differ from Microsoft Word."}))
    }
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
