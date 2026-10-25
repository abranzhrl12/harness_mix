//! Runtime environment configuration shared by native server entry points.

mod environment;
mod runtime;

pub use runtime::RuntimeConfig;

#[cfg(test)]
mod tests;
