//! Pure edit planning, compatible with Node's newline-tolerant unique matching.

mod encoding;
mod hints;
mod matching;
mod prepare;

pub use encoding::{MAX_FILE, utf8_bom};
pub use hints::match_hint;
pub use prepare::prepare;
