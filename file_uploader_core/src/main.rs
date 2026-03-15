mod error;
mod config;

use config::init_logging;
use tracing::info;


fn main() {
    // init_logging;
    if let Err(e) = init_logging("") {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    } else {
        info!("Logging initialized");
    }

    info!("Hello, world!")
}
