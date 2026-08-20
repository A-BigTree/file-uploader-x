# 通用上传/输出 catalog 补登记 & 插件 logo 字段设计

- 日期：2026-08-14
- 状态：已批准（logo 本地文件缺失不告警）

## 背景

1. `common_uploader`（Upload 阶段）与 `common_output`（Output 阶段）的实现与资源目录均已存在，
   但 `file_uploader_plugins::list_in_process_plugins()` 未登记，导致二者无法被
   `core::pipeline::in_process_catalog` 发现。
2. 插件元数据 `PluginMeta` 缺少 logo 字段，上层 UI 无法展示插件图标。

## 需求范围

- 将 `common_uploader` / `common_output` 登记进进程内插件 catalog 清单
- `PluginMeta` 新增可选 `logo` 字段，支持两种形式：
  - 图片链接（`http://` / `https://` 开头）
  - 本地文件名（与 `meta.json` / `README.md` 同目录，即插件资源目录）
- **本项目现有插件一律不加 logo 字段**，仅在规范文档中注明用法

## 设计

### 1. catalog 补登记

`file_uploader_plugins/src/lib.rs` 的 `list_in_process_plugins()` 末尾追加两条 `InProcessEntry`：

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

同步更新 `manifest_includes_known_builtin_plugins` 测试断言覆盖新两条目。

### 2. logo 字段解析

**PluginMeta**（`file_uploader_core/src/pipeline/plugin.rs`）：

```rust
#[serde(default)]
pub logo: Option<String>,
```

- 可选字段，旧 meta.json 完全兼容
- meta.json 不跨 stabby ABI，加字段无 ABI 风险

**PluginResource::load** 模仿 readme 处理，新增 `logo_path: Option<String>`：

| `meta.logo` 取值 | 解析结果 |
|---|---|
| `None` / 缺失 | `logo_path = None`（静默，不告警） |
| `http(s)://...` 链接 | 原样存入 `logo_path` |
| 本地文件名，文件存在 | 资源目录 join 后的**绝对路径**存入 `logo_path` |
| 本地文件名，文件缺失 | `logo_path = None`（**静默，不告警不报错**，与 README 缺失语义一致） |

**透传链路**：`PluginResource.logo_path` → `UploadPluginInfo.logo_path` →
`PluginInfoSummary.logo_path`（in_process_catalog），均 `Option<String>`。

### 3. 文档更新（不改现有插件 meta.json）

- `docs/references/plugin-specification.md` §2：
  - meta.json 字段表新增 `logo` 行：string / null，可选；本地文件名（资源目录内，与
    README.md 同目录）或图片链接（http/https）
  - 资源文件表注明 logo 文件约定
- `.agents/skills/designing-in-process-plugins/SKILL.md`：meta.json 模板处注明 logo 可选

### 4. 测试

- catalog：`load_default_lists_real_builtins` 断言含 `common_uploader` / `common_output`
- manifest：断言新条目存在
- logo 解析单测：
  1. 无 logo 字段 → `logo_path = None`
  2. 链接 → 原样返回
  3. 本地文件存在 → 绝对路径
  4. 本地文件缺失 → `None`（不 panic）

## 非目标

- 不给现有 4 个进程内插件与示例 dylib 插件的 meta.json 添加 logo
- 不做 logo 内容加载/尺寸校验/格式白名单（纯路径透传，与 readme_path 同语义）
- 不涉及 dylib 插件 ABI 变更
