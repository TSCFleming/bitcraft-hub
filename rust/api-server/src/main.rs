// Note that the standalone server is invoked through standaline/src/main.rs, so you will
// also want to set the allocator there.
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() {
    api::main().await.expect("Failed to start server");
}
