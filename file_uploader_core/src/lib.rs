pub mod config;
pub mod pipeline;

pub use pipeline::in_process_catalog::{
    get_in_process_plugin, list_in_process_plugins, InProcessPluginCatalog, PluginInfoSummary,
};
