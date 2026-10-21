use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VerifyAction {
    Build,
    Test,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VerifyArgs {
    pub language: String,
    pub scope: Option<String>,
    /// Rust package name, passed as a single cargo -p argument.
    pub package: Option<String>,
    pub action: Option<VerifyAction>,
    /// .NET build engine: auto (default), dotnet or msbuild.
    #[cfg_attr(feature = "schema", schemars(extend("enum" = ["auto", "dotnet", "msbuild", null])))]
    pub engine: Option<String>,
    /// Warning output: all (default), changed-files or none.
    #[cfg_attr(feature = "schema", schemars(extend("enum" = ["all", "changed-files", "none", null])))]
    pub warnings: Option<String>,
    pub session: Option<String>,
}
