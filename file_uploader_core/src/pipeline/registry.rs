use crate::pipeline::plugin::UploadPluginInfo;
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;
use file_uploader_sdk::models::interface::{PipelineCallback, PipelineEvent, PipelineEventKind};
use serde_json::Value;
use std::sync::Arc;

pub enum PluginRegistryStatus {
    // 禁用
    Disable = 0,
    // 启用
    Enable = 1,
}

// 插件注册信息
pub struct PluginRegistryInfo {
    // 插件实例
    pub plugin_instance: Arc<UploadPluginInfo>,
    // 插件优先级(值越小优先级越高)
    pub priority: i32,
    // 插件状态
    pub status: PluginRegistryStatus,
    // 注册插件配置信息 Map<key:String, value: Value>
    pub registry_config: Option<Value>,
}

impl PluginRegistryInfo {
    pub fn new(
        plugin_instance: Arc<UploadPluginInfo>,
        priority: i32,
        status: PluginRegistryStatus,
        registry_config: Option<Value>,
    ) -> Self {
        PluginRegistryInfo {
            plugin_instance,
            priority,
            status,
            registry_config,
        }
    }

    pub fn execute(&self, context: &UploadInputCtx) -> Result<UploadOutputCtx, UploadError> {
        self.plugin_instance.execute(context)
    }

    pub fn on_load(&self) -> Result<(), UploadError> {
        self.plugin_instance.on_load()
    }

    pub fn on_unload(&self) {
        self.plugin_instance.slot.on_unload();
    }

    pub fn get_plugin_instance(&self) -> Arc<UploadPluginInfo> {
        self.plugin_instance.clone()
    }

    pub fn get_plugin_phase(&self) -> UploadPhase {
        self.plugin_instance.get_meta().phase.clone()
    }
}

impl PartialEq for PluginRegistryInfo {
    fn eq(&self, other: &Self) -> bool {
        let self_phase = self.get_plugin_phase();
        let other_phase = other.get_plugin_phase();
        let phase_order = |phase: &UploadPhase| -> u8 {
            match phase {
                UploadPhase::Input => 0,
                UploadPhase::PreUpload => 1,
                UploadPhase::Upload => 2,
                UploadPhase::PostUpload => 3,
                UploadPhase::Output => 4,
            }
        };
        phase_order(&self_phase) == phase_order(&other_phase) && self.priority == other.priority
    }
}

impl Eq for PluginRegistryInfo {}

impl PartialOrd for PluginRegistryInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PluginRegistryInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let self_phase = self.get_plugin_phase();
        let other_phase = other.get_plugin_phase();

        let phase_order = |phase: &UploadPhase| -> u8 {
            match phase {
                UploadPhase::Input => 0,
                UploadPhase::PreUpload => 1,
                UploadPhase::Upload => 2,
                UploadPhase::PostUpload => 3,
                UploadPhase::Output => 4,
            }
        };

        match phase_order(&self_phase).cmp(&phase_order(&other_phase)) {
            std::cmp::Ordering::Equal => self.priority.cmp(&other.priority),
            other => other,
        }
    }
}

pub struct UploadPluginRegistryTable {
    pub id: String,
    plugins: Vec<PluginRegistryInfo>,
}

impl UploadPluginRegistryTable {
    pub fn new(id: String, mut plugins: Vec<PluginRegistryInfo>) -> Self {
        plugins.sort();
        UploadPluginRegistryTable { id, plugins }
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }

    pub fn get_all_plugins(&self) -> &[PluginRegistryInfo] {
        &self.plugins
    }

    pub fn get_plugins_by_phase(&self, phase: UploadPhase) -> Vec<&PluginRegistryInfo> {
        self.plugins
            .iter()
            .filter(|p| {
                let p_phase = p.get_plugin_phase();
                let phase_order = |ph: &UploadPhase| -> u8 {
                    match ph {
                        UploadPhase::Input => 0,
                        UploadPhase::PreUpload => 1,
                        UploadPhase::Upload => 2,
                        UploadPhase::PostUpload => 3,
                        UploadPhase::Output => 4,
                    }
                };
                phase_order(&p_phase) == phase_order(&phase)
            })
            .collect()
    }

    pub fn preload_all(&self) -> Result<(), Vec<UploadError>> {
        let errors: Vec<UploadError> = self
            .plugins
            .iter()
            .filter_map(|p| p.plugin_instance.slot.get_or_init().err())
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn output_to_input(
        output: &UploadOutputCtx,
        source_ctx: &UploadInputCtx,
    ) -> UploadInputCtx {
        let mut extra_info = source_ctx.extra_info.clone().unwrap_or_default();
        if let Some(ref output_extra) = output.extra_info {
            for (k, v) in output_extra {
                extra_info.insert(k.clone(), v.clone());
            }
        }

        UploadInputCtx {
            file_list: output.file_list.clone().unwrap_or_default(),
            config_info: Arc::new(None),
            extra_info: if extra_info.is_empty() {
                None
            } else {
                Some(extra_info)
            },
            related_process_info: source_ctx.related_process_info.clone(),
        }
    }

    pub fn execute_pipeline(
        &self,
        input_ctx: UploadInputCtx,
        callback: Option<&dyn PipelineCallback>,
    ) -> UploadOutputCtx {
        let phases = [
            UploadPhase::Input,
            UploadPhase::PreUpload,
            UploadPhase::Upload,
            UploadPhase::PostUpload,
            UploadPhase::Output,
        ];

        let mut current_ctx = input_ctx;
        let mut last_output: Option<UploadOutputCtx> = None;

        for phase in &phases {
            let phase_plugins = self.get_plugins_by_phase(phase.clone());
            if phase_plugins.is_empty() {
                continue;
            }

            if let Some(cb) = &callback {
                let event = PipelineEvent {
                    kind: PipelineEventKind::PhaseStart,
                    phase: phase.clone(),
                    plugin_id: None,
                };
                cb.on_event(&event, &current_ctx, None);
            }

            let mut phase_last_output: Option<UploadOutputCtx> = None;

            for plugin in &phase_plugins {
                let plugin_input = UploadInputCtx {
                    file_list: current_ctx.file_list.clone(),
                    config_info: Arc::new(plugin.registry_config.clone()),
                    extra_info: current_ctx.extra_info.clone(),
                    related_process_info: current_ctx.related_process_info.clone(),
                };

                if let Some(cb) = &callback {
                    let event = PipelineEvent {
                        kind: PipelineEventKind::PluginStart,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                    };
                    cb.on_event(&event, &plugin_input, None);
                }

                let output = match plugin.execute(&plugin_input) {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        let fail_ctx = UploadOutputCtx {
                            result: file_uploader_sdk::models::enums::OutputResultType::Failed,
                            message: e.to_string(),
                            file_list: None,
                            extra_info: None,
                        };
                        if let Some(cb) = &callback {
                            let event = PipelineEvent {
                                kind: PipelineEventKind::PluginEnd,
                                phase: phase.clone(),
                                plugin_id: Some(&plugin.plugin_instance.id),
                            };
                            cb.on_event(&event, &plugin_input, Some(&fail_ctx));
                        }
                        return fail_ctx;
                    }
                };

                if let Some(cb) = &callback {
                    let event = PipelineEvent {
                        kind: PipelineEventKind::PluginEnd,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                    };
                    cb.on_event(&event, &plugin_input, Some(&output));
                }

                if matches!(
                    output.result,
                    file_uploader_sdk::models::enums::OutputResultType::Failed
                ) {
                    return output;
                }

                phase_last_output = Some(output.clone());
                current_ctx = Self::output_to_input(&output, &current_ctx);
            }

            if let Some(cb) = &callback {
                let event = PipelineEvent {
                    kind: PipelineEventKind::PhaseEnd,
                    phase: phase.clone(),
                    plugin_id: None,
                };
                cb.on_event(&event, &current_ctx, phase_last_output.as_ref());
            }

            last_output = phase_last_output;
        }

        match last_output {
            Some(output) => output,
            None => UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: String::new(),
                file_list: Some(current_ctx.file_list),
                extra_info: current_ctx.extra_info,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::plugin::{LazyPluginSlot, LazySlotSource, PluginMeta};
    use std::sync::OnceLock;

    struct MockPlugin;

    impl file_uploader_sdk::models::interface::UploadPlugin for MockPlugin {
        fn name(&self) -> &'static str {
            "mock_plugin"
        }

        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: "mock execute".to_string(),
                file_list: None,
                extra_info: None,
            }
        }
    }

    fn create_mock_plugin_info(name: &str, phase: UploadPhase) -> Arc<UploadPluginInfo> {
        let meta = Arc::new(PluginMeta {
            name: name.to_string(),
            title: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin".to_string(),
            author: Some("test".to_string()),
            phase,
        });

        let plugin = std::sync::Arc::new(MockPlugin)
            as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>;
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: "/test/path".to_string(),
                plugin,
            },
            inner: OnceLock::new(),
        };

        Arc::new(UploadPluginInfo {
            id: format!("test_{}", name),
            meta,
            default_config: None,
            path: "/test/path".to_string(),
            slot,
        })
    }

    fn phase_order(phase: &UploadPhase) -> u8 {
        match phase {
            UploadPhase::Input => 0,
            UploadPhase::PreUpload => 1,
            UploadPhase::Upload => 2,
            UploadPhase::PostUpload => 3,
            UploadPhase::Output => 4,
        }
    }

    #[test]
    fn test_plugin_registry_info_sort_by_phase() {
        let p1 = PluginRegistryInfo::new(
            create_mock_plugin_info("p1", UploadPhase::PostUpload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let p2 = PluginRegistryInfo::new(
            create_mock_plugin_info("p2", UploadPhase::Input),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let p3 = PluginRegistryInfo::new(
            create_mock_plugin_info("p3", UploadPhase::Upload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let mut plugins = vec![p1, p2, p3];
        plugins.sort();

        assert_eq!(
            phase_order(&plugins[0].get_plugin_phase()),
            phase_order(&UploadPhase::Input)
        );
        assert_eq!(
            phase_order(&plugins[1].get_plugin_phase()),
            phase_order(&UploadPhase::Upload)
        );
        assert_eq!(
            phase_order(&plugins[2].get_plugin_phase()),
            phase_order(&UploadPhase::PostUpload)
        );
    }

    #[test]
    fn test_plugin_registry_info_sort_by_priority() {
        let p1 = PluginRegistryInfo::new(
            create_mock_plugin_info("p1", UploadPhase::Upload),
            10,
            PluginRegistryStatus::Enable,
            None,
        );

        let p2 = PluginRegistryInfo::new(
            create_mock_plugin_info("p2", UploadPhase::Upload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let p3 = PluginRegistryInfo::new(
            create_mock_plugin_info("p3", UploadPhase::Upload),
            5,
            PluginRegistryStatus::Enable,
            None,
        );

        let mut plugins = vec![p1, p2, p3];
        plugins.sort();

        assert_eq!(plugins[0].priority, 1);
        assert_eq!(plugins[1].priority, 5);
        assert_eq!(plugins[2].priority, 10);
    }

    #[test]
    fn test_plugin_registry_info_sort_by_phase_then_priority() {
        let p1 = PluginRegistryInfo::new(
            create_mock_plugin_info("p1", UploadPhase::Upload),
            5,
            PluginRegistryStatus::Enable,
            None,
        );

        let p2 = PluginRegistryInfo::new(
            create_mock_plugin_info("p2", UploadPhase::Upload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let p3 = PluginRegistryInfo::new(
            create_mock_plugin_info("p3", UploadPhase::PreUpload),
            10,
            PluginRegistryStatus::Enable,
            None,
        );

        let p4 = PluginRegistryInfo::new(
            create_mock_plugin_info("p4", UploadPhase::PreUpload),
            5,
            PluginRegistryStatus::Enable,
            None,
        );

        let mut plugins = vec![p1, p2, p3, p4];
        plugins.sort();

        assert_eq!(
            phase_order(&plugins[0].get_plugin_phase()),
            phase_order(&UploadPhase::PreUpload)
        );
        assert_eq!(plugins[0].priority, 5);
        assert_eq!(
            phase_order(&plugins[1].get_plugin_phase()),
            phase_order(&UploadPhase::PreUpload)
        );
        assert_eq!(plugins[1].priority, 10);
        assert_eq!(
            phase_order(&plugins[2].get_plugin_phase()),
            phase_order(&UploadPhase::Upload)
        );
        assert_eq!(plugins[2].priority, 1);
        assert_eq!(
            phase_order(&plugins[3].get_plugin_phase()),
            phase_order(&UploadPhase::Upload)
        );
        assert_eq!(plugins[3].priority, 5);
    }

    #[test]
    fn test_upload_plugin_registry_table_creation() {
        let p1 = PluginRegistryInfo::new(
            create_mock_plugin_info("p1", UploadPhase::Upload),
            5,
            PluginRegistryStatus::Enable,
            None,
        );

        let p2 = PluginRegistryInfo::new(
            create_mock_plugin_info("p2", UploadPhase::PreUpload),
            10,
            PluginRegistryStatus::Enable,
            None,
        );

        let registry = UploadPluginRegistryTable::new("test_registry".to_string(), vec![p1, p2]);

        assert_eq!(registry.get_id(), "test_registry");
        assert_eq!(registry.get_all_plugins().len(), 2);
        assert_eq!(
            phase_order(&registry.get_all_plugins()[0].get_plugin_phase()),
            phase_order(&UploadPhase::PreUpload)
        );
    }

    #[test]
    fn test_upload_plugin_registry_table_get_by_phase() {
        let p1 = PluginRegistryInfo::new(
            create_mock_plugin_info("p1", UploadPhase::Input),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let p2 = PluginRegistryInfo::new(
            create_mock_plugin_info("p2", UploadPhase::Upload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );

        let p3 = PluginRegistryInfo::new(
            create_mock_plugin_info("p3", UploadPhase::Upload),
            2,
            PluginRegistryStatus::Enable,
            None,
        );

        let registry =
            UploadPluginRegistryTable::new("test_registry".to_string(), vec![p1, p2, p3]);

        let input_plugins = registry.get_plugins_by_phase(UploadPhase::Input);
        assert_eq!(input_plugins.len(), 1);

        let upload_plugins = registry.get_plugins_by_phase(UploadPhase::Upload);
        assert_eq!(upload_plugins.len(), 2);
        assert_eq!(upload_plugins[0].priority, 1);
        assert_eq!(upload_plugins[1].priority, 2);

        let post_upload_plugins = registry.get_plugins_by_phase(UploadPhase::PostUpload);
        assert_eq!(post_upload_plugins.len(), 0);
    }

    #[test]
    fn test_upload_plugin_registry_table_on_load_called() {
        struct MockPluginWithLoadCounter {
            load_count: std::sync::atomic::AtomicU32,
        }

        impl MockPluginWithLoadCounter {
            fn new() -> Self {
                MockPluginWithLoadCounter {
                    load_count: std::sync::atomic::AtomicU32::new(0),
                }
            }

            fn get_load_count(&self) -> u32 {
                self.load_count.load(std::sync::atomic::Ordering::SeqCst)
            }
        }

        impl file_uploader_sdk::models::interface::UploadPlugin for MockPluginWithLoadCounter {
            fn name(&self) -> &'static str {
                "mock_plugin_with_counter"
            }

            fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
                UploadOutputCtx {
                    result: file_uploader_sdk::models::enums::OutputResultType::Success,
                    message: "mock execute".to_string(),
                    file_list: None,
                    extra_info: None,
                }
            }

            fn on_load(&self) {
                self.load_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let meta = Arc::new(PluginMeta {
            name: "test_plugin".to_string(),
            title: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Test plugin with load counter".to_string(),
            author: Some("test".to_string()),
            phase: UploadPhase::Upload,
        });

        let plugin1 = std::sync::Arc::new(MockPluginWithLoadCounter::new());
        let plugin2 = std::sync::Arc::new(MockPluginWithLoadCounter::new());

        let slot1 = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: "/test/path".to_string(),
                plugin: plugin1.clone()
                    as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
        let slot2 = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                config_path: "/test/path".to_string(),
                plugin: plugin2.clone()
                    as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };

        let plugin_info1 = Arc::new(UploadPluginInfo {
            id: "test_plugin_1".to_string(),
            meta: meta.clone(),
            default_config: None,
            path: "/test/path".to_string(),
            slot: slot1,
        });

        let plugin_info2 = Arc::new(UploadPluginInfo {
            id: "test_plugin_2".to_string(),
            meta,
            default_config: None,
            path: "/test/path".to_string(),
            slot: slot2,
        });

        let p1 = PluginRegistryInfo::new(plugin_info1, 1, PluginRegistryStatus::Enable, None);

        let p2 = PluginRegistryInfo::new(plugin_info2, 2, PluginRegistryStatus::Enable, None);

        assert_eq!(plugin1.get_load_count(), 0);
        assert_eq!(plugin2.get_load_count(), 0);

        let table = UploadPluginRegistryTable::new("test_registry".to_string(), vec![p1, p2]);

        assert_eq!(plugin1.get_load_count(), 0);
        assert_eq!(plugin2.get_load_count(), 0);

        table.preload_all().expect("preload should succeed");

        assert_eq!(plugin1.get_load_count(), 1);
        assert_eq!(plugin2.get_load_count(), 1);
    }
}
