pub mod input;
pub mod post_upload;
pub mod pre_upload;
pub mod output;
pub mod upload;

use std::sync::Arc;
use file_uploader_sdk::models::interface::UploadPlugin;

/// 内置进程内插件的资源根目录（编译期由 build.rs 注入）。
/// 指向 `target/<profile>/resources`。
pub fn resources_root() -> &'static str {
    env!("FILE_UPLOADER_RESOURCES_DIR")
}

/// 单个进程内插件的注册条目（编译期固定）。
///
/// 新增插件只需在 [`list_in_process_plugins`] 末尾追加一条 —— 这是进程内插件的
/// **唯一注册点**。
pub struct InProcessEntry {
    /// 资源目录相对 resources 根的子路径，如 `"input/default_input_handler"`。
    /// 必须与 `resources/<phase>/<name>` 实际目录一致。
    pub resource_subdir: &'static str,
    /// 插件实例工厂（非捕获，可重复调用；由 catalog 内部 `OnceLock` 单例化）。
    pub factory: fn() -> Arc<dyn UploadPlugin>,
}

/// 所有内置进程内插件的显式清单。
///
/// 新增插件在此追加一条 `InProcessEntry` 即可被 `InProcessPluginCatalog` 发现。
pub fn list_in_process_plugins() -> &'static [InProcessEntry] {
    &[
        InProcessEntry {
            resource_subdir: "input/default_input_handler",
            factory: || -> Arc<dyn UploadPlugin> {
                Arc::new(input::default_input_handler::DefaultInputHandler)
            },
        },
        InProcessEntry {
            resource_subdir: "pre/upload_file_validator",
            factory: || -> Arc<dyn UploadPlugin> {
                Arc::new(pre_upload::upload_file_validator::UploadFileValidator)
            },
        },
        InProcessEntry {
            resource_subdir: "upload/common_uploader",
            factory: || -> Arc<dyn UploadPlugin> {
                Arc::new(upload::common_uploader::CommonUploader)
            },
        },
        InProcessEntry {
            resource_subdir: "output/common_output",
            factory: || -> Arc<dyn UploadPlugin> {
                Arc::new(output::common_output::CommonOutput)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{list_in_process_plugins, resources_root};

    #[test]
    fn resources_root_is_nonempty_and_ends_with_resources() {
        let r = resources_root();
        assert!(!r.is_empty(), "resources_root must be injected by build.rs");
        assert!(
            r.ends_with("resources"),
            "resources_root should end with 'resources', got: {r}"
        );
    }

    #[test]
    fn manifest_includes_known_builtin_plugins() {
        let entries = list_in_process_plugins();
        assert!(entries.len() >= 4, "should list at least 4 builtin plugins");

        let subdirs: Vec<&str> = entries.iter().map(|e| e.resource_subdir).collect();
        assert!(
            subdirs.contains(&"input/default_input_handler"),
            "missing default_input_handler, got: {subdirs:?}"
        );
        assert!(
            subdirs.contains(&"pre/upload_file_validator"),
            "missing upload_file_validator, got: {subdirs:?}"
        );
        assert!(
            subdirs.contains(&"upload/common_uploader"),
            "missing common_uploader, got: {subdirs:?}"
        );
        assert!(
            subdirs.contains(&"output/common_output"),
            "missing common_output, got: {subdirs:?}"
        );
    }

    #[test]
    fn manifest_is_nonempty_static_slice() {
        let e1 = list_in_process_plugins();
        let e2 = list_in_process_plugins();
        assert!(!e1.is_empty());
        assert!(std::ptr::eq(e1.as_ptr(), e2.as_ptr()), "should be the same 'static slice");
    }

    #[test]
    fn entry_factory_yields_plugin_with_correct_name() {
        // factory 必须可重复调用且产出实现 UploadPlugin 的对象
        let entries = list_in_process_plugins();
        let input_entry = entries
            .iter()
            .find(|e| e.resource_subdir == "input/default_input_handler")
            .expect("default_input_handler entry present");
        let p = (input_entry.factory)();
        assert_eq!(p.name(), "default_input_handler");
        assert!(matches!(p.phase(), file_uploader_sdk::models::enums::UploadPhase::Input));
    }
}
