mod declarations;
mod lexer;
mod razor;
mod validation;

pub(in crate::adapters) mod syntax;

#[cfg(test)]
#[path = "syntax_tests.rs"]
mod syntax_tests;
