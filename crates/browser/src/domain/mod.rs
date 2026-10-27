//! Pure browser policies shared by the adapter boundary and its stateful actor.

mod policy;

pub(crate) use policy::{
    MAX_EVENT_MESSAGE, MAX_EVENTS, MAX_HTML_CHARS, required_session,
    validate_connect_endpoint, validate_endpoint, validate_request, validate_session_owner,
};
