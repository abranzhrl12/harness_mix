//! Stable C# and Razor parser entry points.
//!
//! The implementation is split by stage so callers keep a small, compatible
//! surface while lexing, declaration extraction, Razor projection, and
//! validation remain independently understandable.

pub(in crate::adapters) use super::declarations::symbols::collect;
pub(in crate::adapters) use super::razor::{
    collect_cshtml, collect_cshtml_blocks, razor_markup_end,
};
pub(in crate::adapters) use super::validation::validate_syntax;
