//! Use-case handlers receive only the workspace capabilities they call.
//!
//! Transport decoding, session leases, common policies and response envelopes
//! remain at the application boundary.
//! These functions do not authorize requests: transports must enter through
//! `Application`, not call handlers directly to bypass its policies.

pub mod edit;
pub mod read;
pub mod search;
pub mod verify;
