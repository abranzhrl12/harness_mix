//! Native SCSS support: symbols and syntax validation backed by a real parser.
//!
//! The parser understands the whole SCSS grammar (nesting, `&`, interpolation,
//! variables, mixins, functions, `@use`/`@forward`/`@include`/`@extend`), so the
//! harness never has to guess where a construct starts or ends.
pub(in crate::adapters) mod symbols;
pub(in crate::adapters) mod syntax;
