use serde::{Deserialize, Serialize};

const MIN_VERIFICATION_TIMEOUT_SECONDS: u64 = 1;
pub const DEFAULT_VERIFICATION_TIMEOUT_SECONDS: u64 = 900;
pub const MAX_VERIFICATION_TIMEOUT_SECONDS: u64 = 3600;

fn deserialize_timeout_seconds<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<u64>::deserialize(deserializer)?;
    if value.is_some_and(|seconds| {
        !(MIN_VERIFICATION_TIMEOUT_SECONDS..=MAX_VERIFICATION_TIMEOUT_SECONDS).contains(&seconds)
    }) {
        return Err(serde::de::Error::custom(
            "timeoutSeconds must be between 1 and 3600",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VerifyAction {
    Build,
    Test,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VerifyArgs {
    /// Verification language, e.g. rust, dotnet, csharp, razor, cshtml, javascript, js, typescript, ts, css, scss, vue, astro, python, go or angular.
    pub language: String,
    /// Target scope for verification.
    /// For fast in-process per-file verification without running a full compilation, pass one or more
    /// workspace-relative file paths (single path or comma/semicolon-separated paths) such as
    /// `Scripts/app.js`, `Views/Taller/Itinerario.cshtml`, `src/component.tsx` or `styles/site.scss`.
    /// For full .NET builds/tests, pass an explicit `.csproj` or `.sln` file (e.g. `src/App/App.csproj`;
    /// dependencies must already be restored). Rust uses an optional workspace directory; Angular selects `angular.json`.
    #[serde(alias = "path", alias = "file", alias = "files")]
    pub scope: Option<String>,
    /// Rust package name, passed as a single cargo -p argument.
    pub package: Option<String>,
    /// Angular application/library name from angular.json (not a workspace alias).
    pub project: Option<String>,
    /// Named Angular build configuration; defaults to the target's defaultConfiguration.
    pub configuration: Option<String>,
    /// Verification action: build (default; runs fast syntax/structural check for per-file scopes) or test.
    pub action: Option<VerifyAction>,
    /// Build or verification engine: auto (default), dotnet, msbuild, or fast/syntax (forces in-process per-file verification).
    #[cfg_attr(
        feature = "schema",
        schemars(extend("enum" = ["auto", "dotnet", "msbuild", "fast", "syntax", null]))
    )]
    pub engine: Option<String>,
    /// Warning output: all (default), changed-files or none.
    #[cfg_attr(feature = "schema", schemars(extend("enum" = ["all", "changed-files", "none", null])))]
    pub warnings: Option<String>,
    /// Per-process build/test deadline in seconds; defaults to 900, valid range 1..=3600.
    #[serde(
        rename = "timeoutSeconds",
        alias = "timeout_seconds",
        default,
        deserialize_with = "deserialize_timeout_seconds"
    )]
    #[cfg_attr(feature = "schema", schemars(extend("minimum" = 1, "maximum" = 3600)))]
    pub timeout_seconds: Option<u64>,
    pub session: Option<String>,
}

impl VerifyArgs {
    /// Returns the effective per-process deadline, defaulting to 15 minutes.
    pub fn effective_timeout(&self) -> Result<std::time::Duration, crate::Failure> {
        let seconds = self
            .timeout_seconds
            .unwrap_or(DEFAULT_VERIFICATION_TIMEOUT_SECONDS);
        if !(MIN_VERIFICATION_TIMEOUT_SECONDS..=MAX_VERIFICATION_TIMEOUT_SECONDS).contains(&seconds)
        {
            return Err((
                "VERIFY_TIMEOUT",
                "timeoutSeconds must be between 1 and 3600",
            ));
        }
        Ok(std::time::Duration::from_secs(seconds))
    }
}
