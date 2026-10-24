#[path = "harnessmix/config.rs"]
mod config;
#[path = "harnessmix/images.rs"]
mod images;
#[path = "harnessmix/hooks.rs"]
mod hooks;
#[path = "harnessmix/menu.rs"]
mod menu;
#[path = "harnessmix/activity.rs"]
mod activity;
#[path = "harnessmix/console.rs"]
mod console;
#[path = "harnessmix/rendering.rs"]
mod rendering;
#[path = "harnessmix/streaming.rs"]
mod streaming;
#[path = "harnessmix/session_settings.rs"]
mod session_settings;

#[tokio::main]
async fn main() -> Result<(), String> {
    config::run(std::env::args_os().skip(1).collect()).await
}
