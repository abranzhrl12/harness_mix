pub(in crate::adapters) mod astro;
pub(in crate::adapters) mod csharp;
pub(in crate::adapters) mod css;
pub(in crate::adapters) mod ecmascript;
pub(in crate::adapters) mod model;
pub(in crate::adapters) mod razor;
mod registry;
pub(in crate::adapters) mod rust;
pub(in crate::adapters) mod vue;

#[cfg(test)]
use registry::ParserRegistry;
pub(in crate::adapters) use registry::default_syntax_registry;

#[cfg(test)]
#[path = "registry_tests.rs"]
mod registry_tests;
