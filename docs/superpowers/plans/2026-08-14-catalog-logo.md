# 通用插件 catalog 补登记 & logo 字段实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `common_uploader` / `common_output` 登记进进程内插件 catalog，并为 `PluginMeta` 新增可选 `logo` 字段（链接或本地文件名），解析为 `logo_path` 透传到 `UploadPluginInfo` / `PluginInfoSummary`。

**Architecture:** catalog 登记只需在 `file_uploader_plugins::list_in_process_plugins()` 清单追加两条 `InProcessEntry`；logo 为纯宿主侧改动（meta.json 不跨 stabby ABI，无 ABI 风险）——`PluginMeta` 加 `#[serde(default)] logo: Option<String>`，`PluginResource::load` 模仿 `readme_path` 的宽松语义解析出 `logo_path`（链接原样、本地文件 join、缺失静默 None），三层结构体透传。文档注明用法但**不给现有插件加 logo**。

**Tech Stack:** Rust edition 2024，serde/serde_json，cargo test。

**规格依据：** `docs/superpowers/specs/2026-08-14-catalog-logo-design.md`

---

## File Structure

| 文件 | 动作 | 职责 |
|---|---|---|
| `file_uploader_plugins/src/lib.rs` | Modify | catalog 清单追加 common_uploader / common_output 两条 |
| `file_uploader_core/src/pipeline/plugin.rs` | Modify | `PluginMeta.logo` 字段、`PluginResource.logo_path` 解析、`UploadPluginInfo.logo_path` 透传 |
| `file_uploader_core/src/pipeline/in_process_catalog.rs` | Modify | `PluginInfoSummary.logo_path` 透传 + 真实内置插件断言 |
| `docs/references/plugin-specification.md` | Modify | §2 字段表加 logo 行、加载器说明、§1.7 概要字段 |
| `.agents/skills/designing-in-process-plugins/SKILL.md` | Modify | meta.json 模板注明 logo 可选 |

---

### Task 1: catalog 补登记 common_uploader / common_output

**Files:**
- Modify: `file_uploader_plugins/src/lib.rs`

- [ ] **Step 1: 更新 manifest 测试（先写失败测试）**

在 `file_uploader_plugins/src/lib.rs` 的 `mod tests` 中，将现有测试
`manifest_includes_known_builtin_plugins` 整体替换为：

```rust
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
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_plugins`
Expected: FAIL — `missing common_uploader, got: [...]`（清单尚未登记）

- [ ] **Step 3: 在清单追加两条 InProcessEntry**

在 `file_uploader_plugins/src/lib.rs` 的 `list_in_process_plugins()` 中，
将尾部注释行

```rust
        // 后续 common_uploader / common_output 落地后在此追加
```

替换为：

```rust
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
```

（`upload` / `output` 两个 mod 已在文件顶部 `pub mod` 声明，无需新增 use。）

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_plugins`
Expected: PASS（全部 5 个测试）

- [ ] **Step 5: Commit**

```bash
git add file_uploader_plugins/src/lib.rs
git commit -m "feat(plugins): register common_uploader and common_output in in-process catalog"
```

---

### Task 2: PluginMeta.logo 字段与 PluginResource.logo_path 解析

**Files:**
- Modify: `file_uploader_core/src/pipeline/plugin.rs`

- [ ] **Step 1: 写失败测试（logo 解析四分支）**

在 `file_uploader_core/src/pipeline/plugin.rs` 的 `mod tests` 内追加
（放在 `test_plugin_resource_readme_path_absent_is_none` 之后）：

```rust
    // ==================== logo 解析 ====================

    fn make_logo_tmp_dir(tag: &str, meta_json: &str) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("fux_logo_{}_{}", tag, std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("meta.json"), meta_json).unwrap();
        tmp
    }

    #[test]
    fn test_plugin_resource_logo_absent_is_none() {
        let tmp = make_logo_tmp_dir(
            "absent",
            r#"{"name":"t","title":"t","version":"0.0.1","description":"d","author":null,"phase":"PreUpload"}"#,
        );
        let r = PluginResource::load(&tmp).unwrap();
        assert!(r.logo_path.is_none(), "no logo field => logo_path None");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_plugin_resource_logo_url_passthrough() {
        let tmp = make_logo_tmp_dir(
            "url",
            r#"{"name":"t","title":"t","version":"0.0.1","description":"d","author":null,"phase":"PreUpload","logo":"https://example.com/a.png"}"#,
        );
        let r = PluginResource::load(&tmp).unwrap();
        assert_eq!(
            r.logo_path.as_deref(),
            Some("https://example.com/a.png"),
            "url logo should pass through as-is"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_plugin_resource_logo_local_file_resolved_absolute() {
        let tmp = make_logo_tmp_dir(
            "local",
            r#"{"name":"t","title":"t","version":"0.0.1","description":"d","author":null,"phase":"PreUpload","logo":"logo.png"}"#,
        );
        std::fs::write(tmp.join("logo.png"), b"png-bytes").unwrap();
        let r = PluginResource::load(&tmp).unwrap();
        let logo = r.logo_path.expect("local existing logo => Some");
        assert!(logo.ends_with("logo.png"), "got: {logo}");
        assert!(
            Path::new(&logo).is_absolute(),
            "local logo should resolve to absolute path, got: {logo}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_plugin_resource_logo_local_missing_silent_none() {
        let tmp = make_logo_tmp_dir(
            "missing",
            r#"{"name":"t","title":"t","version":"0.0.1","description":"d","author":null,"phase":"PreUpload","logo":"nope.png"}"#,
        );
        // 不创建 nope.png：缺失 => None，静默不报错不告警
        let r = PluginResource::load(&tmp).unwrap();
        assert!(r.logo_path.is_none(), "missing local logo file => None");
        let _ = std::fs::remove_dir_all(&tmp);
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_core plugin_resource_logo`
Expected: 编译失败 — `no field 'logo_path' on struct PluginResource`

- [ ] **Step 3: 实现 logo 字段与解析**

**3a.** `PluginMeta` 结构体（`pub struct PluginMeta { ... }`）末尾、`pub phase: UploadPhase,`
之后追加：

```rust
    // 插件 logo（可选）：本地文件名（与 meta.json 同目录）或图片链接（http/https）
    #[serde(default)]
    pub logo: Option<String>,
```

**3b.** `PluginResource` 结构体的 `readme_path` 字段之后追加：

```rust
    /// logo 引用（可选）：http(s) 链接原样保留；本地文件解析为绝对路径；缺失 → None（静默）
    pub logo_path: Option<String>,
```

**3c.** `PluginResource::load` 中，在 `let readme = dir.join("README.md");` 块之后、
`Ok(PluginResource {` 之前追加：

```rust
        let logo_path = match &meta.logo {
            None => None,
            Some(logo) if logo.starts_with("http://") || logo.starts_with("https://") => {
                Some(logo.clone())
            }
            Some(file) => {
                let p = dir.join(file);
                if p.is_file() {
                    Some(p.display().to_string())
                } else {
                    None // 本地文件缺失：静默，与 README 缺失语义一致
                }
            }
        };
```

并在返回值构造 `Ok(PluginResource { ... })` 中 `readme_path,` 之后加一行：

```rust
            logo_path,
```

**3d.** `UploadPluginInfo` 结构体的 `readme_path` 字段之后追加：

```rust
    // logo 引用（可选，不加载内容）
    pub logo_path: Option<String>,
```

`new_in_process` 与 `new_from_dylib_path` 两个构造器的 `Ok(UploadPluginInfo { ... })`
中 `readme_path: resource.readme_path,` 之后各加一行：

```rust
            logo_path: resource.logo_path,
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_core plugin_resource_logo`
Expected: PASS（4 个新测试）

Run: `cargo build --workspace`
Expected: 编译通过（若 main.rs 有构造 `UploadPluginInfo` 字面量处报缺字段，按上述字段补 `logo_path: resource.logo_path` 或 `logo_path: None`）

- [ ] **Step 5: Commit**

```bash
git add file_uploader_core/src/pipeline/plugin.rs
git commit -m "feat(core): add optional PluginMeta.logo with logo_path resolution (url or local file)"
```

---

### Task 3: PluginInfoSummary.logo_path 透传 + catalog 真实插件断言

**Files:**
- Modify: `file_uploader_core/src/pipeline/in_process_catalog.rs`

- [ ] **Step 1: 写失败测试（真实资源断言新插件 + logo 透传）**

**1a.** 在 `in_process_catalog.rs` 的 `mod tests` 中，将现有测试
`load_default_lists_real_builtins` 整体替换为：

```rust
    #[test]
    fn load_default_lists_real_builtins() {
        let root = target_resources_root();
        if !root.join("input/default_input_handler").exists() {
            eprintln!("skip: {} not ready yet", root.display());
            return;
        }
        let cat = InProcessPluginCatalog::load_default().expect("load_default should succeed");
        let ids: Vec<&str> = cat.list().iter().map(|s| s.id.as_str()).collect();
        assert!(
            ids.iter().any(|id| id.ends_with("default_input_handler")),
            "should include default_input_handler, got: {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id.ends_with("upload_file_validator")),
            "should include upload_file_validator, got: {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id.ends_with("common_uploader")),
            "should include common_uploader, got: {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id.ends_with("common_output")),
            "should include common_output, got: {ids:?}"
        );
    }
```

**1b.** 在 `mod tests` 末尾追加 logo 透传单测（用临时资源目录，含 http 链接 logo 的 meta）：

```rust
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
```

注意：`DIR_COUNTER` / `CountingMock` / `InProcessEntry` / `UploadPlugin` / `Arc` 均已在
该 tests 模块现有 use / 定义中可用，无需新增导入。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p file_uploader_core in_process_catalog`
Expected: 编译失败 — `no field 'logo_path' on struct PluginInfoSummary`

（若 `load_default_lists_real_builtins` 因资源未复制而 skip，先跑一次
`cargo build -p file_uploader_plugins` 让 build.rs 复制资源再测。）

- [ ] **Step 3: 实现 logo_path 透传**

**3a.** `PluginInfoSummary` 结构体的 `readme_path` 字段之后追加：

```rust
    pub logo_path: Option<String>,
```

**3b.** `from_entries` 中 `summaries.push(PluginInfoSummary { ... })` 的
`readme_path: resource.readme_path.clone(),` 之后追加：

```rust
                logo_path: resource.logo_path.clone(),
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p file_uploader_core in_process_catalog`
Expected: PASS（含新增 2 处断言）

Run: `cargo test --workspace`
Expected: 全部通过（Task 1/2 改动不回归）

- [ ] **Step 5: Commit**

```bash
git add file_uploader_core/src/pipeline/in_process_catalog.rs
git commit -m "feat(catalog): expose logo_path in PluginInfoSummary and assert common plugins listed"
```

---

### Task 4: 文档更新（规范 + skill）

**Files:**
- Modify: `docs/references/plugin-specification.md`
- Modify: `.agents/skills/designing-in-process-plugins/SKILL.md`

- [ ] **Step 1: 更新 plugin-specification.md §2 meta.json 字段表**

找到 §2「meta.json」小节的字段表（`| author | string / null | 是 | 作者，可为 null |` 所在表），
在 `phase` 行之后追加一行：

```markdown
| `logo` | string / null | 否 | 插件 logo：**本地文件名**（与 `meta.json` / `README.md` 同目录，即插件资源目录）或**图片链接**（`http://` / `https://` 开头）。链接原样保留；本地文件存在则解析为绝对路径；缺失或字段缺省 → `logo_path = None`（静默，不报错不告警） |
```

> 本项目内置插件与示例 dylib 插件的 meta.json **均不添加** logo 字段，字段仅作为规范预留。

- [ ] **Step 2: 更新 plugin-specification.md §2 资源加载说明**

找到「插件资源加载（`PluginResource`）」小节的有序列表（`3. README.md —— 选读...` 之后），
追加第 4 条：

```markdown
4. `meta.logo` —— 选读：`http(s)://` 链接原样存入 `logo_path`；本地文件名 join 资源目录后存在则记绝对路径、缺失 → `None`（静默不告警）
```

- [ ] **Step 3: 更新 plugin-specification.md 其余引用点**

- §2「`UploadPluginInfo`」小节首句改为：
  `封装插件 ID、meta、config（Arc<PluginConfigInfo>）、加载路径 path、readme_path、logo_path、LazyPluginSlot。`
- §1.7 表格中 `&[PluginInfoSummary]` 一行的括号说明改为：
  `（id + meta + config + readme_path + logo_path，不可 execute）`

- [ ] **Step 4: 更新 SKILL.md meta.json 模板**

找到 `.agents/skills/designing-in-process-plugins/SKILL.md` 中 `meta.json：` 代码块
（含 `"version": "0.0.1", "author": "you", "phase": "PreUpload"`），在该代码块之后
紧跟一行说明：

```markdown
可选字段 `logo`（本地文件名或 http/https 图片链接）预留为规范，本项目内置插件不加。
```

- [ ] **Step 5: 校验与 Commit**

Run: `cargo test --workspace`
Expected: PASS（文档改动不影响）

```bash
git add docs/references/plugin-specification.md .agents/skills/designing-in-process-plugins/SKILL.md
git commit -m "docs(spec): document optional meta.json logo field and logo_path semantics"
```

---

## 验收清单（对照规格）

- [ ] `cargo test --workspace` 全绿
- [ ] catalog `list` 含 4 个内置插件（input/pre/upload/output 各一）
- [ ] logo 四分支行为与规格表一致（None / 链接 / 本地存在 / 本地缺失静默）
- [ ] 现有 4 个进程内插件 + 示例 dylib 插件 meta.json 未改动
- [ ] 规范文档与 skill 已同步（AGENTS.md「注意事项」的规范变更要求）
