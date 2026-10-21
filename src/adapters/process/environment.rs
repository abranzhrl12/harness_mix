use std::process::Command;

pub(super) fn go_environment(command: &mut Command) {
    // These overrides apply to the child only; never mutate the harness environment.
    command
        .env("GOENV", "off")
        .env("GOFLAGS", "")
        .env("GOCACHEPROG", "")
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", "local")
        .env("GO111MODULE", "on")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env("GONOPROXY", "none")
        .env("GOPRIVATE", "")
        .env("GOVCS", "*:off")
        .env("CGO_ENABLED", "0")
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env_remove("GOEXPERIMENT");
}

pub(super) fn dotnet_environment(command: &mut Command) {
    command
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_CLI_UI_LANGUAGE", "en-US")
        .env("VSLANG", "1033")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE", "1")
        .env("MSBUILDDISABLENODEREUSE", "1")
        .env("CORECLR_ENABLE_PROFILING", "0")
        .env("COR_ENABLE_PROFILING", "0")
        .env_remove("DOTNET_STARTUP_HOOKS")
        .env_remove("MSBUILD_EXE_PATH")
        .env_remove("MSBuildSDKsPath");
}

pub(super) fn node_environment(command: &mut Command) {
    command
        .env("NODE_OPTIONS", "")
        .env("NODE_PATH", "")
        .env_remove("NODE_V8_COVERAGE");
}

pub(super) fn git_environment(command: &mut Command) {
    // Never inherit alternate repositories, injected -c values, or askpass.
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("LC_ALL", "C");
}
