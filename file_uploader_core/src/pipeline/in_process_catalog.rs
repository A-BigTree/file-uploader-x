use crate::pipeline::plugin::{PluginConfigInfo, PluginMeta, PluginResource, UploadPluginInfo};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::interface::UploadPlugin;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tracing::warn;

/// 进程内插件的展示信息（不可 execute，纯展示）。
#[derive(Serialize, Clone)]
pub struct PluginInfoSummary {
    pub id: String,
    pub meta: Arc<PluginMeta>,
    pub config: Arc<PluginConfigInfo>,
    pub readme_path: Option<String>,
    pub logo_path: Option<String>,
}

/// 单个进程内插件的注册条目（由宿主提供）。
pub struct InProcessEntry {
    /// 资源目录相对 resources 根的子路径，如 `"input/default_input_handler"`。
    /// 必须与 `resources/<phase>/<name>` 实际目录一致。
    pub resource_subdir: &'static str,
    /// 插件实例工厂（非捕获，可重复调用；由 catalog 内部 `OnceLock` 单例化）。
    pub factory: fn() -> Arc<dyn UploadPlugin>,
}

pub struct InProcessPluginCatalog {
    summaries: Vec<PluginInfoSummary>,
    instances: HashMap<String, OnceLock<Arc<dyn UploadPlugin>>>,
    factories: HashMap<String, fn() -> Arc<dyn UploadPlugin>>,
    resource_dirs: HashMap<String, PathBuf>,
}

impl InProcessPluginCatalog {
    /// 由调用方提供清单与资源根目录构造（宿主注册入口，测试 / 定制场景亦可用）。
    pub fn from_entries(
        entries: &[InProcessEntry],
        resources_root: &Path,
    ) -> Result<Self, UploadError> {
        let mut summaries = Vec::with_capacity(entries.len());
        let mut instances = HashMap::new();
        let mut factories = HashMap::new();
        let mut resource_dirs = HashMap::new();

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
                logo_path: resource.logo_path.clone(),
            });
            factories.insert(id.clone(), entry.factory);
            resource_dirs.insert(id.clone(), dir);
            instances.insert(id, OnceLock::new());
        }

        Ok(InProcessPluginCatalog {
            summaries,
            instances,
            factories,
            resource_dirs,
        })
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

    /// 按 id 构造 `UploadPluginInfo`（meta/config 来自资源目录缓存；插槽复用 per-id 单例）。
    pub fn get_info(&self, id: &str) -> Option<Arc<UploadPluginInfo>> {
        let cell = self.instances.get(id)?;
        let factory = *self.factories.get(id)?;
        let dir = self.resource_dirs.get(id)?;
        let plugin = cell.get_or_init(factory).clone();
        let slot = crate::pipeline::plugin::LazyPluginSlot {
            source: crate::pipeline::plugin::LazySlotSource::InProcess {
                resource_dir: dir.display().to_string(),
                plugin,
            },
            inner: OnceLock::new(),
        };
        let summary = self.summaries.iter().find(|s| s.id == *id)?;
        Some(Arc::new(UploadPluginInfo {
            id: summary.id.clone(),
            meta: summary.meta.clone(),
            config: summary.config.clone(),
            path: dir.display().to_string(),
            readme_path: summary.readme_path.clone(),
            logo_path: summary.logo_path.clone(),
            slot,
        }))
    }
}

/// 全局 catalog 单例（宿主注册后固定；未注册时查询返回空）。
static GLOBAL_CATALOG: OnceLock<InProcessPluginCatalog> = OnceLock::new();

/// 宿主注册进程内插件清单（仅首次生效；重复调用 warn 并跳过，返回 Ok）。
pub fn register_in_process_plugins(
    entries: &[InProcessEntry],
    resources_root: &Path,
) -> Result<(), UploadError> {
    if GLOBAL_CATALOG.get().is_some() {
        warn!("in-process catalog already registered; skip re-registration");
        return Ok(());
    }
    let catalog = InProcessPluginCatalog::from_entries(entries, resources_root)?;
    let _ = GLOBAL_CATALOG.set(catalog); // 竞态由 OnceLock 收敛
    Ok(())
}

/// 列出所有已注册进程内插件概要（未注册时返回空切片 + warn，不报错）。
pub fn list_in_process_plugins() -> Result<&'static [PluginInfoSummary], UploadError> {
    match GLOBAL_CATALOG.get() {
        Some(c) => Ok(c.list()),
        None => {
            warn!("in-process catalog not registered; return empty list");
            Ok(&[])
        }
    }
}

/// 按 id 取进程内插件实现对象（未注册时返回 None）。
pub fn get_in_process_plugin(id: &str) -> Result<Option<Arc<dyn UploadPlugin>>, UploadError> {
    Ok(GLOBAL_CATALOG.get().and_then(|c| c.get(id)))
}

/// 按 id 构造进程内插件的 `UploadPluginInfo`（供宿主装配 RegistryTable）。
pub fn get_in_process_plugin_info(id: &str) -> Result<Option<Arc<UploadPluginInfo>>, UploadError> {
    Ok(GLOBAL_CATALOG.get().and_then(|c| c.get_info(id)))
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

    static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 在临时目录下造一个只含 meta.json 的资源子目录，返回资源根。
    /// 目录名含进程 id + 自增计数，保证并发测试互不干扰。
    fn make_tmp_resource_root(subdir: &str, name: &str, phase: &str) -> std::path::PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "fux_catalog_{}_{}_{}",
            std::process::id(),
            n,
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

    // ==================== 宿主注册模式（全局单例，合并为单测试避免并行抢占） ====================

    fn make_entry(subdir: &'static str, name: &'static str) -> (InProcessEntry, std::path::PathBuf) {
        let root = make_tmp_resource_root(subdir, name, "Input");
        (
            InProcessEntry {
                resource_subdir: subdir,
                factory: || -> Arc<dyn UploadPlugin> { Arc::new(CountingMock) },
            },
            root,
        )
    }

    #[test]
    fn global_register_list_get_info_and_skip_semantics() {
        // 1. 首个注册者可强断言；若已被其它会话注册（进程内不会，防御性）则退化为跳过断言
        let (e1, root1) = make_entry("input/mock_a", "mock_a");
        super::register_in_process_plugins(&[e1], &root1).expect("first register should succeed");

        let listed = super::list_in_process_plugins().expect("list after register");
        assert!(!listed.is_empty());
        if listed.iter().any(|s| s.id == "in_process_Input_mock_a") {
            // 我是首个注册者：id 派生规则不变
            assert_eq!(listed.len(), 1);
            // get_in_process_plugin_info 构造 UploadPluginInfo（meta/config 来自资源目录）
            let info = super::get_in_process_plugin_info("in_process_Input_mock_a")
                .expect("get_info should succeed")
                .expect("plugin should exist");
            assert_eq!(info.id, "in_process_Input_mock_a");
            assert_eq!(info.meta.name, "mock_a");
            assert!(matches!(info.meta.phase, UploadPhase::Input));
        }

        // 2. 二次注册（不同条目）被跳过
        let (e2, root2) = make_entry("input/mock_b", "mock_b");
        super::register_in_process_plugins(&[e2], &root2).expect("repeat register returns Ok");
        let listed = super::list_in_process_plugins().expect("list");
        assert!(
            !listed.iter().any(|s| s.id.ends_with("mock_b")),
            "second registration must be skipped, got: {:?}",
            listed.iter().map(|s| &s.id).collect::<Vec<_>>()
        );

        // 3. 未知 id 行为
        assert!(super::get_in_process_plugin_info("no_such_id")
            .expect("query itself should not error")
            .is_none());
    }

    #[test]
    fn summary_carries_logo_path_from_meta() {
        let n = DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("fux_logo_summary_{}_{}", std::process::id(), n));
        let dir = root.join("input/mock_logo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            r#"{"name":"mock_logo","title":"M","description":"d","version":"0.0.1","author":null,"phase":"Input","logo":"https://example.com/x.png"}"#,
        ).unwrap();
        let cat = InProcessPluginCatalog::from_entries(
            &[InProcessEntry {
                resource_subdir: "input/mock_logo",
                factory: || -> Arc<dyn UploadPlugin> { Arc::new(CountingMock) },
            }],
            &root,
        )
        .expect("from_entries should succeed");
        assert_eq!(
            cat.list()[0].logo_path.as_deref(),
            Some("https://example.com/x.png"),
            "summary should carry logo_path from meta"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
