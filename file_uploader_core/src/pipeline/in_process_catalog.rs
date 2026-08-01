use crate::pipeline::plugin::{PluginConfigInfo, PluginMeta, PluginResource};
use file_uploader_plugins::InProcessEntry;
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::interface::UploadPlugin;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// 进程内插件的展示信息（不可 execute，纯展示）。
#[derive(Serialize, Clone)]
pub struct PluginInfoSummary {
    pub id: String,
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
    pub readme_path: Option<String>,
}

pub struct InProcessPluginCatalog {
    summaries: Vec<PluginInfoSummary>,
    instances: HashMap<String, OnceLock<Arc<dyn UploadPlugin>>>,
    factories: HashMap<String, fn() -> Arc<dyn UploadPlugin>>,
}

impl InProcessPluginCatalog {
    /// 由调用方提供清单与资源根目录构造（测试 / 定制场景入口）。
    pub(crate) fn from_entries(
        entries: &[InProcessEntry],
        resources_root: &Path,
    ) -> Result<Self, UploadError> {
        let mut summaries = Vec::with_capacity(entries.len());
        let mut instances = HashMap::new();
        let mut factories = HashMap::new();

        for entry in entries {
            let dir = resources_root.join(entry.resource_subdir);
            let resource = PluginResource::load(&dir)?; // 硬失败
            // ID 派生：与 UploadPluginInfo::new_in_process 完全一致
            let id = format!("in_process_{:?}_{}", resource.meta.phase, resource.meta.name);

            summaries.push(PluginInfoSummary {
                id: id.clone(),
                meta: resource.meta.clone(),
                config: resource.config.clone(),
                readme_path: resource.readme_path.clone(),
            });
            factories.insert(id.clone(), entry.factory);
            instances.insert(id, OnceLock::new());
        }

        Ok(InProcessPluginCatalog {
            summaries,
            instances,
            factories,
        })
    }

    /// 自定义资源根目录加载（用编译期清单 [`file_uploader_plugins::list_in_process_plugins`]）。
    pub fn load_from(resources_root: &Path) -> Result<Self, UploadError> {
        Self::from_entries(
            file_uploader_plugins::list_in_process_plugins(),
            resources_root,
        )
    }

    /// 用编译期 env! 默认资源根目录加载。
    pub fn load_default() -> Result<Self, UploadError> {
        Self::load_from(Path::new(file_uploader_plugins::resources_root()))
    }

    /// 所有进程内插件概要（不触发插件加载、不调 on_load）。
    pub fn list(&self) -> &[PluginInfoSummary] {
        &self.summaries
    }

    /// 按 id 取插件实现对象（per-id 单例复用；on_load 由调用方显式管理）。
    pub fn get(&self, id: &str) -> Option<Arc<dyn UploadPlugin>> {
        let cell = self.instances.get(id)?;
        let factory = *self.factories.get(id)?;
        let arc = cell.get_or_init(factory);
        Some(arc.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
    use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};
    use std::sync::atomic::{AtomicU32, Ordering};

    // factory 必须是非捕获 fn 指针，故 on_load 计数用 static 共享状态
    static MOCK_LOAD_COUNT: AtomicU32 = AtomicU32::new(0);

    struct CountingMock;
    impl UploadPlugin for CountingMock {
        fn name(&self) -> &'static str {
            "mock_input"
        }
        fn phase(&self) -> UploadPhase {
            UploadPhase::Input
        }
        fn execute(&self, _ctx: &UploadInputCtx) -> UploadOutputCtx {
            UploadOutputCtx {
                result: OutputResultType::Success,
                message: "ok".into(),
                file: None,
                extra_info: None,
            }
        }
        fn on_load(&self) {
            MOCK_LOAD_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 在临时目录下造一个只含 meta.json 的资源子目录，返回资源根。
    fn make_tmp_resource_root(subdir: &str, name: &str, phase: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fux_catalog_{}_{}",
            std::process::id(),
            subdir.replace('/', "_")
        ));
        let dir = root.join(subdir);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = format!(
            r#"{{"name":"{name}","title":"M","description":"d","version":"0.0.1","author":null,"phase":"{phase}"}}"#
        );
        std::fs::write(dir.join("meta.json"), meta).unwrap();
        root
    }

    fn one_mock_catalog() -> (InProcessPluginCatalog, std::path::PathBuf) {
        let root = make_tmp_resource_root("input/mock_input", "mock_input", "Input");
        let cat = InProcessPluginCatalog::from_entries(
            &[InProcessEntry {
                resource_subdir: "input/mock_input",
                factory: || -> Arc<dyn UploadPlugin> { Arc::new(CountingMock) },
            }],
            &root,
        )
        .expect("from_entries with valid tmp resource should succeed");
        (cat, root)
    }

    #[test]
    fn list_count_and_derived_id() {
        let (cat, _root) = one_mock_catalog();
        assert_eq!(cat.list().len(), 1);
        assert_eq!(cat.list()[0].id, "in_process_Input_mock_input");
    }

    #[test]
    fn get_returns_singleton_same_arc() {
        let (cat, _root) = one_mock_catalog();
        let a = cat.get("in_process_Input_mock_input").expect("known id");
        let b = cat.get("in_process_Input_mock_input").expect("known id");
        assert!(Arc::ptr_eq(&a, &b), "get must return the same singleton Arc");
    }

    #[test]
    fn get_unknown_id_returns_none() {
        let (cat, _root) = one_mock_catalog();
        assert!(cat.get("does_not_exist").is_none());
    }

    #[test]
    fn get_does_not_trigger_on_load() {
        MOCK_LOAD_COUNT.store(0, Ordering::SeqCst);
        let (cat, _root) = one_mock_catalog();
        let _ = cat.get("in_process_Input_mock_input");
        let _ = cat.get("in_process_Input_mock_input");
        assert_eq!(
            MOCK_LOAD_COUNT.load(Ordering::SeqCst),
            0,
            "get must NOT auto-call on_load"
        );
    }

    #[test]
    fn list_does_not_trigger_on_load() {
        MOCK_LOAD_COUNT.store(0, Ordering::SeqCst);
        let (cat, _root) = one_mock_catalog();
        let _ = cat.list();
        assert_eq!(MOCK_LOAD_COUNT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn from_entries_fails_when_meta_missing() {
        let empty_root =
            std::env::temp_dir().join(format!("fux_catalog_empty_{}", std::process::id()));
        std::fs::create_dir_all(&empty_root).unwrap();
        let res = InProcessPluginCatalog::from_entries(
            &[InProcessEntry {
                resource_subdir: "input/none",
                factory: || -> Arc<dyn UploadPlugin> { Arc::new(CountingMock) },
            }],
            &empty_root,
        );
        assert!(res.is_err(), "missing meta.json should be a hard failure");
        let _ = std::fs::remove_dir_all(&empty_root);
    }
}
