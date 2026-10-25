#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod cli;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: the only mutation is our own process environment before serving;
    // no other thread has started yet.
    // Native extraction resolves imports only over the snapshot filesystem.
    // oxc-resolver's Node fallback consults NODE_PATH; an inherited value would
    // bind bare aliases to directories outside the project view.
    unsafe { std::env::remove_var("NODE_PATH") };
    cli::run(std::env::args_os().skip(1)).await
}
