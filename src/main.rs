#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod cli;
mod index_progress;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::run(std::env::args_os().skip(1)).await
}
