//! Pure browser policies shared by the adapter boundary and its stateful actor.

mod policy;
mod tabs;

pub(crate) use policy::{
    MAX_EVENT_MESSAGE, MAX_EVENTS, MAX_HTML_CHARS, is_auto_endpoint, parse_connect_input,
    required_session, validate_connect_endpoint, validate_endpoint, validate_request,
    validate_session_owner,
};
pub(crate) use tabs::{is_localhost_url, rank_targets, select_target};
