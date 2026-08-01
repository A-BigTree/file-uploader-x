pub mod input;
pub mod post_upload;
pub mod pre_upload;
pub mod output;
pub mod upload;

/// 内置进程内插件的资源根目录（编译期由 build.rs 注入）。
/// 指向 `target/<profile>/resources`。
pub fn resources_root() -> &'static str {
    env!("FILE_UPLOADER_RESOURCES_DIR")
}

#[cfg(test)]
mod tests {
    use super::resources_root;

    #[test]
    fn resources_root_is_nonempty_and_ends_with_resources() {
        let r = resources_root();
        assert!(!r.is_empty(), "resources_root must be injected by build.rs");
        assert!(
            r.ends_with("resources"),
            "resources_root should end with 'resources', got: {r}"
        );
    }
}
