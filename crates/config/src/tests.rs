use std::{env::VarError, ffi::OsString};

use harness_domain::tool_contract::ToolProfile;

use crate::RuntimeConfig;

fn values(
    profile: Option<&str>,
    watch: Option<&str>,
    eager: Option<&str>,
    presentation: Option<&str>,
    watch_supported: bool,
) -> Result<RuntimeConfig, &'static str> {
    RuntimeConfig::from_values(
        Ok(profile.map(str::to_owned)),
        Ok(watch.map(str::to_owned)),
        eager.map(OsString::from),
        Ok(presentation.map(str::to_owned)),
        watch_supported,
    )
}

#[test]
fn defaults_match_platform_and_full_profile_policy() {
    let supported = values(None, None, None, None, true).unwrap();
    assert_eq!(supported.profile, ToolProfile::Full);
    assert_eq!(supported.index_watch_setting, None);
    assert!(supported.index_watch_enabled);
    assert!(supported.eager_tools_enabled);
    assert!(!supported.context_presentation);

    let unsupported = values(None, None, None, None, false).unwrap();
    assert!(!unsupported.index_watch_enabled);
}

#[test]
fn explicit_watch_overrides_and_enforces_profile_and_platform() {
    assert!(
        !values(Some("full"), Some("0"), None, None, true)
            .unwrap()
            .index_watch_enabled
    );
    assert_eq!(
        values(Some("edit"), Some("1"), None, None, true).unwrap_err(),
        "HARNESS_INDEX_WATCH=1 requires the full tool profile"
    );
    assert_eq!(
        values(Some("full"), Some("1"), None, None, false).unwrap_err(),
        "Automatic index publication currently requires Windows"
    );
    assert_eq!(
        values(None, Some("true"), None, None, true).unwrap_err(),
        "HARNESS_INDEX_WATCH must be 0 or 1"
    );
}

#[test]
fn eager_and_presentation_values_keep_exact_match_semantics() {
    let config = values(None, None, Some("0"), Some("context"), true).unwrap();
    assert!(!config.eager_tools_enabled);
    assert!(config.context_presentation);

    let config = values(None, None, Some("false"), Some("Context"), true).unwrap();
    assert!(config.eager_tools_enabled);
    assert!(!config.context_presentation);
}

#[test]
fn invalid_utf8_profile_and_watch_keep_existing_errors() {
    let invalid = VarError::NotUnicode(OsString::from("invalid"));
    assert_eq!(
        RuntimeConfig::from_values(Err(invalid), Ok(None), None, Ok(None), true).unwrap_err(),
        "HARNESS_TOOL_PROFILE must be valid UTF-8"
    );

    let invalid = VarError::NotUnicode(OsString::from("invalid"));
    assert_eq!(
        RuntimeConfig::from_values(Ok(None), Err(invalid), None, Ok(None), true).unwrap_err(),
        "HARNESS_INDEX_WATCH must be 0 or 1"
    );
}

#[test]
fn non_utf8_eager_value_defaults_to_enabled_and_presentation_to_false() {
    #[cfg(unix)]
    let eager_tools = {
        use std::os::unix::ffi::OsStringExt;
        Some(OsString::from_vec(vec![0xff]))
    };
    #[cfg(windows)]
    let eager_tools = {
        use std::os::windows::ffi::OsStringExt;
        Some(OsString::from_wide(&[0xd800]))
    };
    #[cfg(not(any(unix, windows)))]
    let eager_tools = Some(OsString::from("not-zero"));

    let config = RuntimeConfig::from_values(
        Ok(None),
        Ok(Some("0".to_owned())),
        eager_tools,
        Err(VarError::NotUnicode(OsString::from("invalid"))),
        false,
    )
    .unwrap();
    assert!(config.eager_tools_enabled);
    assert!(!config.context_presentation);
}
