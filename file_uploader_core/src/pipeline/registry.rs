use crate::pipeline::plugin::UploadPluginInfo;
use serde_json::Value;
use std::sync::Arc;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;

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

    pub fn execute(&self, context: &UploadInputCtx) -> UploadOutputCtx {
        self.plugin_instance.execute(context)
    }

    pub fn on_load(&self) {
        self.plugin_instance.on_load();
    }

    pub fn on_unload(&self) {
        self.plugin_instance.on_unload();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::plugin::{PluginMeta, PluginSlot};

    struct MockPlugin;

    impl file_uploader_sdk::models::interface::UploadPlugin for MockPlugin {
        fn name(&self) -> &'static str {
            "mock_plugin"
        }

        fn execute(
            &self,
            ctx: &file_uploader_sdk::models::ctx::UploadInputCtx,
        ) -> file_uploader_sdk::models::ctx::UploadOutputCtx {
            file_uploader_sdk::models::ctx::UploadOutputCtx {
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
        
        let plugin = Box::new(MockPlugin);
        let slot = Arc::new(PluginSlot::InProcess(std::sync::Arc::new(*plugin) as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>));
        
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
        
        assert_eq!(phase_order(&plugins[0].get_plugin_phase()), phase_order(&UploadPhase::Input));
        assert_eq!(phase_order(&plugins[1].get_plugin_phase()), phase_order(&UploadPhase::Upload));
        assert_eq!(phase_order(&plugins[2].get_plugin_phase()), phase_order(&UploadPhase::PostUpload));
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
        
        assert_eq!(phase_order(&plugins[0].get_plugin_phase()), phase_order(&UploadPhase::PreUpload));
        assert_eq!(plugins[0].priority, 5);
        assert_eq!(phase_order(&plugins[1].get_plugin_phase()), phase_order(&UploadPhase::PreUpload));
        assert_eq!(plugins[1].priority, 10);
        assert_eq!(phase_order(&plugins[2].get_plugin_phase()), phase_order(&UploadPhase::Upload));
        assert_eq!(plugins[2].priority, 1);
        assert_eq!(phase_order(&plugins[3].get_plugin_phase()), phase_order(&UploadPhase::Upload));
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
        
        let registry = UploadPluginRegistryTable::new(
            "test_registry".to_string(),
            vec![p1, p2],
        );
        
        assert_eq!(registry.get_id(), "test_registry");
        assert_eq!(registry.get_all_plugins().len(), 2);
        assert_eq!(phase_order(&registry.get_all_plugins()[0].get_plugin_phase()), phase_order(&UploadPhase::PreUpload));
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
        
        let registry = UploadPluginRegistryTable::new(
            "test_registry".to_string(),
            vec![p1, p2, p3],
        );
        
        let input_plugins = registry.get_plugins_by_phase(UploadPhase::Input);
        assert_eq!(input_plugins.len(), 1);
        
        let upload_plugins = registry.get_plugins_by_phase(UploadPhase::Upload);
        assert_eq!(upload_plugins.len(), 2);
        assert_eq!(upload_plugins[0].priority, 1);
        assert_eq!(upload_plugins[1].priority, 2);
        
        let post_upload_plugins = registry.get_plugins_by_phase(UploadPhase::PostUpload);
        assert_eq!(post_upload_plugins.len(), 0);
    }
}
