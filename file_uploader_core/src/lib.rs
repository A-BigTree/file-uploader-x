pub mod config;
pub mod pipeline;

pub use pipeline::in_process_catalog::{
    get_in_process_plugin, get_in_process_plugin_info, list_in_process_plugins,
    register_in_process_plugins, InProcessEntry, InProcessPluginCatalog, PluginInfoSummary,
};
