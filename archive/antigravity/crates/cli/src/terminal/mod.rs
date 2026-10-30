//! Interactive terminal interface for HarnessMix.

mod activity;
mod clipboard;
mod config;
mod console;
mod hooks;
mod images;
mod menu;
mod rendering;
mod session_settings;
mod streaming;

pub use config::{ConversationConfig, ConversationFactory, run};
