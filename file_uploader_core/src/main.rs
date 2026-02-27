mod config;

use config::init_logging;
use tracing::info;


fn main() {
    // init_logging;
    if let Err(e) = init_logging("temp_uploader.log") {
        eprintln!("Failed to initialize logging: {}", e);
        return;
    }

    info!("Hello, world!")
}
