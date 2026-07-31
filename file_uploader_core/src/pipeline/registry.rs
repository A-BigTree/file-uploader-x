use crate::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
use crate::pipeline::plugin::{UploadPluginInfo, ValidationError, errors_to_string};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;
use serde_json::Value;
use std::sync::Arc;
use tracing::warn;

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

    /// **声明式校验**：按插件 config.json 的 schema 校验 `registry_config`，不加载插件。
    pub fn validate_declarative(&self) -> Result<(), Vec<ValidationError>> {
        self.plugin_instance
            .validate_config_declarative(&self.registry_config)
    }

    /// **插件级校验**：以 `registry_config` 构造仅含配置的 ctx 调用插件 `validate_params`
    /// （会触发插件懒加载）。
    pub fn validate_params(&self) -> Result<(), UploadError> {
        let ctx = UploadInputCtx {
            file: None,
            config_info: Arc::new(self.registry_config.clone()),
            extra_info: None,
            work_dir: None,
        };
        self.plugin_instance.validate_params(&ctx)
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
    /// 构建期声明式校验结果缓存：(plugin_id, 错误)
    declarative_errors: Vec<(String, ValidationError)>,
}

impl UploadPluginRegistryTable {
    /// 构建注册表：排序 + **声明式**校验（不加载任何插件，保持懒加载语义）。
    /// 声明式错误仅记录并 warn，不阻断构建；需要硬失败请用 [`Self::try_new`]。
    pub fn new(id: String, mut plugins: Vec<PluginRegistryInfo>) -> Self {
        plugins.sort();
        let declarative_errors = Self::collect_declarative_errors(&plugins);
        for (plugin_id, err) in &declarative_errors {
            warn!(
                "registry '{}' plugin '{}' config invalid: {}",
                id, plugin_id, err
            );
        }
        UploadPluginRegistryTable {
            id,
            plugins,
            declarative_errors,
        }
    }

    /// 严格构建：任一插件声明式校验失败即返回 Err。
    pub fn try_new(
        id: String,
        plugins: Vec<PluginRegistryInfo>,
    ) -> Result<Self, Vec<(String, ValidationError)>> {
        let table = Self::new(id, plugins);
        if table.declarative_errors.is_empty() {
            Ok(table)
        } else {
            Err(table.declarative_errors)
        }
    }

    /// 收集全部插件的声明式校验错误（不加载插件）
    fn collect_declarative_errors(
        plugins: &[PluginRegistryInfo],
    ) -> Vec<(String, ValidationError)> {
        plugins
            .iter()
            .flat_map(|p| {
                let plugin_id = p.plugin_instance.get_id();
                match p.validate_declarative() {
                    Ok(()) => Vec::new(),
                    Err(errs) => errs
                        .into_iter()
                        .map(|e| (plugin_id.clone(), e))
                        .collect::<Vec<_>>(),
                }
            })
            .collect()
    }

    /// 构建期缓存的声明式校验错误（结构化明细）
    pub fn declarative_errors(&self) -> &[(String, ValidationError)] {
        &self.declarative_errors
    }

    /// **全量校验**：声明式 + 插件级（会加载全部插件）。
    pub fn validate_all(&self) -> Result<(), Vec<UploadError>> {
        let mut errors: Vec<UploadError> = self
            .declarative_errors
            .iter()
            .map(|(plugin_id, e)| {
                UploadError::PluginConfigInvalid(format!("plugin '{}': {}", plugin_id, e))
            })
            .collect();

        for p in &self.plugins {
            if p.validate_declarative().is_err() {
                // 声明式已失败，跳过插件级校验避免噪声
                continue;
            }
            if let Err(e) = p.validate_params() {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
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

    /// 预加载全部插件，并在加载成功后执行插件级 `validate_params`。
    /// 构建期的声明式错误也会一并折叠返回。
    pub fn preload_all(&self) -> Result<(), Vec<UploadError>> {
        let mut errors: Vec<UploadError> = Vec::new();

        for (plugin_id, e) in &self.declarative_errors {
            errors.push(UploadError::PluginConfigInvalid(format!(
                "plugin '{}': {}",
                plugin_id, e
            )));
        }

        for p in &self.plugins {
            if let Err(e) = p.plugin_instance.slot.get_or_init() {
                errors.push(e);
                continue;
            }
            // 声明式失败的插件不再跑插件级校验（错误已记录）
            if let Err(errs) = p.validate_declarative() {
                let _ = errors_to_string(&errs);
                continue;
            }
            if let Err(e) = p.validate_params() {
                errors.push(e);
            }
        }

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
            file: output.file.clone(),
            config_info: Arc::new(None),
            extra_info: if extra_info.is_empty() {
                None
            } else {
                Some(extra_info)
            },
            work_dir: source_ctx.work_dir.clone(),
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
                let now_ms = chrono::Local::now().timestamp_millis();
                let event = PipelineEvent {
                    timestamp_ms: now_ms,
                    kind: PipelineEventKind::PhaseStart,
                    phase: phase.clone(),
                    plugin_id: None,
                    plugin_meta: None,
                };
                cb.on_event(&event, &current_ctx, None);
            }

            let mut phase_last_output: Option<UploadOutputCtx> = None;

            for plugin in &phase_plugins {
                let plugin_input = UploadInputCtx {
                    file: current_ctx.file.clone(),
                    config_info: Arc::new(plugin.registry_config.clone()),
                    extra_info: current_ctx.extra_info.clone(),
                    work_dir: current_ctx.work_dir.clone(),
                };

                if let Some(cb) = &callback {
                    let now_ms = chrono::Local::now().timestamp_millis();
                    let event = PipelineEvent {
                        timestamp_ms: now_ms,
                        kind: PipelineEventKind::PluginStart,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                        plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
                    };
                    cb.on_event(&event, &plugin_input, None);
                }

                let output = match plugin.execute(&plugin_input) {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        let fail_ctx = UploadOutputCtx {
                            result: file_uploader_sdk::models::enums::OutputResultType::Failed,
                            message: e.to_string(),
                            file: None,
                            extra_info: None,
                        };
                        if let Some(cb) = &callback {
                            let now_ms = chrono::Local::now().timestamp_millis();
                            let event = PipelineEvent {
                                timestamp_ms: now_ms,
                                kind: PipelineEventKind::PluginEnd,
                                phase: phase.clone(),
                                plugin_id: Some(&plugin.plugin_instance.id),
                                plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
                            };
                            cb.on_event(&event, &plugin_input, Some(&fail_ctx));
                        }
                        return fail_ctx;
                    }
                };

                if let Some(cb) = &callback {
                    let now_ms = chrono::Local::now().timestamp_millis();
                    let event = PipelineEvent {
                        timestamp_ms: now_ms,
                        kind: PipelineEventKind::PluginEnd,
                        phase: phase.clone(),
                        plugin_id: Some(&plugin.plugin_instance.id),
                        plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
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
                let now_ms = chrono::Local::now().timestamp_millis();
                let event = PipelineEvent {
                    timestamp_ms: now_ms,
                    kind: PipelineEventKind::PhaseEnd,
                    phase: phase.clone(),
                    plugin_id: None,
                    plugin_meta: None,
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
                file: current_ctx.file,
                extra_info: current_ctx.extra_info,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::plugin::{
        LazyPluginSlot, LazySlotSource, PluginConfigInfo, PluginMeta, ValidationReason,
    };
    use crate::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
    use std::sync::Mutex;
    use std::sync::OnceLock;

    struct MockPlugin;

    impl file_uploader_sdk::models::interface::UploadPlugin for MockPlugin {
        fn name(&self) -> &'static str {
            "mock_plugin"
        }

        fn phase(&self) -> UploadPhase {
            UploadPhase::Upload
        }

        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: "mock execute".to_string(),
                file: None,
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
                resource_dir: "/test/path".to_string(),
                plugin,
            },
            inner: OnceLock::new(),
        };

        Arc::new(UploadPluginInfo {
            id: format!("test_{}", name),
            meta,
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test/path".to_string(),
            readme_path: None,
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

            fn phase(&self) -> UploadPhase {
                UploadPhase::Upload
            }

            fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
                UploadOutputCtx {
                    result: file_uploader_sdk::models::enums::OutputResultType::Success,
                    message: "mock execute".to_string(),
                    file: None,
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
                resource_dir: "/test/path".to_string(),
                plugin: plugin1.clone()
                    as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
        let slot2 = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: "/test/path".to_string(),
                plugin: plugin2.clone()
                    as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };

        let plugin_info1 = Arc::new(UploadPluginInfo {
            id: "test_plugin_1".to_string(),
            meta: meta.clone(),
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test/path".to_string(),
            readme_path: None,
            slot: slot1,
        });

        let plugin_info2 = Arc::new(UploadPluginInfo {
            id: "test_plugin_2".to_string(),
            meta,
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test/path".to_string(),
            readme_path: None,
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

    struct CallbackRecord {
        pub timestamp_ms: i64,
        pub kind: PipelineEventKind,
        pub phase: UploadPhase,
        pub plugin_id: Option<String>,
        pub plugin_meta_name: Option<String>,
    }

    struct TestCallback {
        pub records: Mutex<Vec<CallbackRecord>>,
    }

    impl TestCallback {
        fn new() -> Self {
            TestCallback {
                records: Mutex::new(Vec::new()),
            }
        }
    }

    impl PipelineCallback for TestCallback {
        fn on_event(
            &self,
            event: &PipelineEvent,
            _ctx: &UploadInputCtx,
            _result: Option<&UploadOutputCtx>,
        ) {
            self.records.lock().unwrap().push(CallbackRecord {
                timestamp_ms: event.timestamp_ms,
                kind: event.kind.clone(),
                phase: event.phase.clone(),
                plugin_id: event.plugin_id.map(|s| s.to_string()),
                plugin_meta_name: event.plugin_meta.map(|m| m.name.clone()),
            });
        }
    }

    struct FailPlugin;

    impl file_uploader_sdk::models::interface::UploadPlugin for FailPlugin {
        fn name(&self) -> &'static str {
            "fail_plugin"
        }

        fn phase(&self) -> UploadPhase {
            UploadPhase::Upload
        }

        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Failed,
                message: "intentional failure".to_string(),
                file: None,
                extra_info: None,
            }
        }
    }

    fn create_fail_plugin_info(name: &str, phase: UploadPhase) -> Arc<UploadPluginInfo> {
        let meta = Arc::new(PluginMeta {
            name: name.to_string(),
            title: "Fail Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "Fail plugin".to_string(),
            author: Some("test".to_string()),
            phase,
        });
        let plugin = std::sync::Arc::new(FailPlugin)
            as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>;
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: "/test/path".to_string(),
                plugin,
            },
            inner: OnceLock::new(),
        };
        Arc::new(UploadPluginInfo {
            id: format!("test_{}", name),
            meta,
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test/path".to_string(),
            readme_path: None,
            slot,
        })
    }

    struct ConfigReadPlugin {
        captured_config: Arc<Mutex<Arc<Option<Value>>>>,
    }

    impl ConfigReadPlugin {
        fn new() -> Self {
            ConfigReadPlugin {
                captured_config: Arc::new(Mutex::new(Arc::new(None))),
            }
        }
    }

    impl file_uploader_sdk::models::interface::UploadPlugin for ConfigReadPlugin {
        fn name(&self) -> &'static str {
            "config_read_plugin"
        }

        fn phase(&self) -> UploadPhase {
            UploadPhase::Upload
        }

        fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
            *self.captured_config.lock().unwrap() = ctx.config_info.clone();
            UploadOutputCtx {
                result: file_uploader_sdk::models::enums::OutputResultType::Success,
                message: "ok".to_string(),
                file: None,
                extra_info: None,
            }
        }
    }

    #[test]
    fn test_execute_pipeline_empty_registry() {
        let registry = UploadPluginRegistryTable::new("test".to_string(), vec![]);
        let input = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            work_dir: None,
        };
        let result = registry.execute_pipeline(input, None);
        assert!(matches!(
            result.result,
            file_uploader_sdk::models::enums::OutputResultType::Success
        ));
    }

    #[test]
    fn test_execute_pipeline_single_plugin_callback_order() {
        let plugin_info = create_mock_plugin_info("p1", UploadPhase::PreUpload);
        let reg_info = PluginRegistryInfo::new(plugin_info, 1, PluginRegistryStatus::Enable, None);
        let registry = UploadPluginRegistryTable::new("test".to_string(), vec![reg_info]);

        let input = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            work_dir: None,
        };

        let cb = TestCallback::new();
        let result = registry.execute_pipeline(input, Some(&cb));
        assert!(matches!(
            result.result,
            file_uploader_sdk::models::enums::OutputResultType::Success
        ));

        let records = cb.records.lock().unwrap();
        assert_eq!(records.len(), 4);
        assert!(matches!(records[0].kind, PipelineEventKind::PhaseStart));
        assert!(matches!(records[1].kind, PipelineEventKind::PluginStart));
        assert!(matches!(records[2].kind, PipelineEventKind::PluginEnd));
        assert!(matches!(records[3].kind, PipelineEventKind::PhaseEnd));
        assert_eq!(records[1].plugin_id.as_deref(), Some("test_p1"));
        assert!(matches!(records[0].phase, UploadPhase::PreUpload));
        for r in records.iter() {
            assert!(r.timestamp_ms > 0, "timestamp_ms should be positive");
        }
        // 阶段级事件无插件元信息
        assert!(matches!(records[0].kind, PipelineEventKind::PhaseStart));
        assert!(records[0].plugin_meta_name.is_none());
        assert!(matches!(records[3].kind, PipelineEventKind::PhaseEnd));
        assert!(records[3].plugin_meta_name.is_none());
        // 插件级事件携带插件元信息
        assert!(matches!(records[1].kind, PipelineEventKind::PluginStart));
        assert_eq!(records[1].plugin_meta_name.as_deref(), Some("p1"));
        assert!(matches!(records[2].kind, PipelineEventKind::PluginEnd));
        assert_eq!(records[2].plugin_meta_name.as_deref(), Some("p1"));
    }

    #[test]
    fn test_execute_pipeline_plugin_failed_interrupts() {
        let p1 = PluginRegistryInfo::new(
            create_mock_plugin_info("p1", UploadPhase::PreUpload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );
        let p2 = PluginRegistryInfo::new(
            create_fail_plugin_info("p2", UploadPhase::Upload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );
        let p3 = PluginRegistryInfo::new(
            create_mock_plugin_info("p3", UploadPhase::PostUpload),
            1,
            PluginRegistryStatus::Enable,
            None,
        );
        let registry =
            UploadPluginRegistryTable::new("test".to_string(), vec![p1, p2, p3]);

        let input = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            work_dir: None,
        };

        let cb = TestCallback::new();
        let result = registry.execute_pipeline(input, Some(&cb));

        assert!(matches!(
            result.result,
            file_uploader_sdk::models::enums::OutputResultType::Failed
        ));
        assert_eq!(result.message, "intentional failure");

        let records = cb.records.lock().unwrap();
        let phase_ends: Vec<_> = records
            .iter()
            .filter(|r| matches!(r.kind, PipelineEventKind::PhaseEnd))
            .collect();
        assert_eq!(phase_ends.len(), 1);
        assert!(matches!(phase_ends[0].phase, UploadPhase::PreUpload));

        let plugin_ends: Vec<_> = records
            .iter()
            .filter(|r| matches!(r.kind, PipelineEventKind::PluginEnd))
            .collect();
        assert_eq!(plugin_ends.len(), 2);
        // 失败分支的 PluginEnd 仍携带失败插件(p2)的元信息
        assert_eq!(
            plugin_ends[1].plugin_meta_name.as_deref(),
            Some("p2"),
            "failed plugin PluginEnd should carry its plugin_meta"
        );
    }

    #[test]
    fn test_execute_pipeline_registry_config_injected() {
        let captured = Arc::new(Mutex::new(Arc::new(None)));
        let plugin = Arc::new(ConfigReadPlugin {
            captured_config: captured.clone(),
        });
        let meta = Arc::new(PluginMeta {
            name: "config_read".to_string(),
            title: "Config Read".to_string(),
            version: "1.0.0".to_string(),
            description: "reads config".to_string(),
            author: None,
            phase: UploadPhase::Upload,
        });
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: "/test".to_string(),
                plugin: plugin as Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
        let plugin_info = Arc::new(UploadPluginInfo {
            id: "test_config_read".to_string(),
            meta,
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test".to_string(),
            readme_path: None,
            slot,
        });

        let config_value = serde_json::json!({"key": "value"});
        let reg_info = PluginRegistryInfo::new(
            plugin_info,
            1,
            PluginRegistryStatus::Enable,
            Some(config_value.clone()),
        );
        let registry = UploadPluginRegistryTable::new("test".to_string(), vec![reg_info]);

        let input = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            work_dir: None,
        };

        registry.execute_pipeline(input, None);

        let config = captured.lock().unwrap();
        assert_eq!(**config, Some(config_value));
    }

    #[test]
    fn test_work_dir_propagates_to_plugin_and_across_output_to_input() {
        struct CaptureWorkDir {
            seen: Arc<Mutex<Option<String>>>,
        }
        impl file_uploader_sdk::models::interface::UploadPlugin for CaptureWorkDir {
            fn name(&self) -> &'static str {
                "capture_wd"
            }
            fn phase(&self) -> UploadPhase {
                UploadPhase::Upload
            }
            fn execute(&self, ctx: &UploadInputCtx) -> UploadOutputCtx {
                *self.seen.lock().unwrap() = ctx.work_dir.clone();
                UploadOutputCtx {
                    result: file_uploader_sdk::models::enums::OutputResultType::Success,
                    message: "ok".into(),
                    file: ctx.file.clone(),
                    extra_info: None,
                }
            }
        }
        let seen = Arc::new(Mutex::new(None));
        let plugin = Arc::new(CaptureWorkDir {
            seen: seen.clone(),
        });
        let meta = Arc::new(PluginMeta {
            name: "capture_wd".to_string(),
            title: "Capture".to_string(),
            version: "1.0.0".to_string(),
            description: "capture work_dir".to_string(),
            author: None,
            phase: UploadPhase::Upload,
        });
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: "/test".to_string(),
                plugin: plugin as Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>,
            },
            inner: OnceLock::new(),
        };
        let info = Arc::new(UploadPluginInfo {
            id: "test_capture_wd".to_string(),
            meta,
            config: Arc::new(PluginConfigInfo::default()),
            path: "/test".to_string(),
            readme_path: None,
            slot,
        });
        let reg = PluginRegistryInfo::new(info, 1, PluginRegistryStatus::Enable, None);
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);

        let input = UploadInputCtx {
            file: None,
            config_info: Arc::new(None),
            extra_info: None,
            work_dir: Some("/data/wd-flow".to_string()),
        };
        table.execute_pipeline(input, None);
        assert_eq!(seen.lock().unwrap().as_deref(), Some("/data/wd-flow"));
    }

    // ==================== 配置校验 ====================

    /// 带 schema 的插件：一个 required text（common） + 一个分组
    fn schema_plugin_info(id: &str, config_json: &str) -> Arc<UploadPluginInfo> {
        let meta = Arc::new(PluginMeta {
            name: id.to_string(),
            title: "Schema Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: "schema plugin".to_string(),
            author: None,
            phase: UploadPhase::Upload,
        });
        let config: PluginConfigInfo = serde_json::from_str(config_json).unwrap();
        let plugin = std::sync::Arc::new(CountingValidatePlugin::default())
            as std::sync::Arc<dyn file_uploader_sdk::models::interface::UploadPlugin>;
        let slot = LazyPluginSlot {
            source: LazySlotSource::InProcess {
                resource_dir: "/test".to_string(),
                plugin,
            },
            inner: OnceLock::new(),
        };
        Arc::new(UploadPluginInfo {
            id: id.to_string(),
            meta,
            config: Arc::new(config),
            path: "/test".to_string(),
            readme_path: None,
            slot,
        })
    }

    /// 一个会在 validate_params 中拒绝特定配置的插件，并记录加载次数
    #[derive(Default)]
    struct CountingValidatePlugin;

    impl file_uploader_sdk::models::interface::UploadPlugin for CountingValidatePlugin {
        fn name(&self) -> &'static str {
            "counting_validate_plugin"
        }
        fn phase(&self) -> UploadPhase {
            UploadPhase::Upload
        }
        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx::success("ok")
        }
        fn validate_params(&self, ctx: &UploadInputCtx) -> Result<(), String> {
            match file_uploader_sdk::utils::config_util::get_str(&ctx.config_info, "token") {
                Some(t) if t == "forbidden" => Err("token 不允许为 forbidden".to_string()),
                _ => Ok(()),
            }
        }
    }

    const COMMON_REQUIRED_SCHEMA: &str = r#"{
        "common": [
            {
                "key": "token",
                "title": "凭证",
                "config_type": "Default",
                "default_value": "",
                "required": true,
                "form": { "type": "text" }
            }
        ]
    }"#;

    const GROUPED_SCHEMA: &str = r#"{
        "common": [],
        "groups": [
            { "group": "oss", "title": "OSS", "params": [] },
            { "group": "local", "title": "Local", "params": [] }
        ]
    }"#;

    #[test]
    fn test_new_records_declarative_errors_without_loading() {
        let info = schema_plugin_info("schema_p", COMMON_REQUIRED_SCHEMA);
        // registry_config 缺 required 的 token
        let reg = PluginRegistryInfo::new(info.clone(), 1, PluginRegistryStatus::Enable, None);
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);

        // 记录了声明式错误
        let errs = table.declarative_errors();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].0, "schema_p");
        assert_eq!(errs[0].1.key.as_deref(), Some("token"));
        assert_eq!(errs[0].1.reason, ValidationReason::Required);

        // 但插件未被加载（懒加载语义保持）
        assert!(info.slot.inner.get().is_none());
    }

    #[test]
    fn test_new_with_valid_config_has_no_errors() {
        let info = schema_plugin_info("schema_ok", COMMON_REQUIRED_SCHEMA);
        let reg = PluginRegistryInfo::new(
            info.clone(),
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"token": "abc"})),
        );
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);
        assert!(table.declarative_errors().is_empty());
        assert!(info.slot.inner.get().is_none());
    }

    #[test]
    fn test_try_new_rejects_missing_required_param() {
        let info = schema_plugin_info("schema_p", COMMON_REQUIRED_SCHEMA);
        let reg = PluginRegistryInfo::new(info, 1, PluginRegistryStatus::Enable, None);
        let result = UploadPluginRegistryTable::try_new("t".to_string(), vec![reg]);
        let errs = result.err().expect("try_new should reject invalid config");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].1.reason, ValidationReason::Required);

        // 合法配置可以构建成功
        let info2 = schema_plugin_info("schema_ok", COMMON_REQUIRED_SCHEMA);
        let reg2 = PluginRegistryInfo::new(
            info2,
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"token": "abc"})),
        );
        assert!(UploadPluginRegistryTable::try_new("t".to_string(), vec![reg2]).is_ok());
    }

    #[test]
    fn test_preload_all_runs_plugin_validate_params() {
        // 声明式通过，但插件级 validate_params 拒绝
        let info = schema_plugin_info("schema_p", COMMON_REQUIRED_SCHEMA);
        let reg = PluginRegistryInfo::new(
            info.clone(),
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"token": "forbidden"})),
        );
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);
        assert!(table.declarative_errors().is_empty());

        let errs = table
            .preload_all()
            .err()
            .expect("preload_all should surface plugin validate_params error");
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], UploadError::PluginParamInvalid(_)));
        assert!(errs[0].to_string().contains("forbidden"));
        // 插件已被加载
        assert!(info.slot.inner.get().is_some());
    }

    #[test]
    fn test_preload_all_ok_when_all_valid() {
        let info = schema_plugin_info("schema_ok", COMMON_REQUIRED_SCHEMA);
        let reg = PluginRegistryInfo::new(
            info,
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"token": "abc"})),
        );
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);
        assert!(table.preload_all().is_ok());
    }

    #[test]
    fn test_validate_all_reports_group_unknown() {
        let info = schema_plugin_info("grouped_p", GROUPED_SCHEMA);
        let reg = PluginRegistryInfo::new(
            info,
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"group": "s3"})),
        );
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);

        let errs = table.validate_all().err().expect("group s3 is unknown");
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], UploadError::PluginConfigInvalid(_)));
        let msg = errs[0].to_string();
        assert!(msg.contains("grouped_p"), "got: {msg}");
        assert!(msg.contains("s3"), "got: {msg}");
    }

    #[test]
    fn test_validate_all_ok_for_known_group() {
        let info = schema_plugin_info("grouped_ok", GROUPED_SCHEMA);
        let reg = PluginRegistryInfo::new(
            info,
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"group": "oss"})),
        );
        let table = UploadPluginRegistryTable::new("t".to_string(), vec![reg]);
        assert!(table.validate_all().is_ok());
    }

    #[test]
    fn test_registry_info_validate_declarative_and_params() {
        let info = schema_plugin_info("p", COMMON_REQUIRED_SCHEMA);
        let reg = PluginRegistryInfo::new(
            info,
            1,
            PluginRegistryStatus::Enable,
            Some(serde_json::json!({"token": "forbidden"})),
        );
        // 声明式通过（token 非空）
        assert!(reg.validate_declarative().is_ok());
        // 插件级拒绝
        let e = reg.validate_params().unwrap_err();
        assert!(matches!(e, UploadError::PluginParamInvalid(_)));
    }
}
