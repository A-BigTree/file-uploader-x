# UploadPluginInfo 动态库加载功能设计

**日期**: 2025-06-09
**状态**: 待审阅

## 概述

为 `UploadPluginInfo` 新增 `new_from_dylib_path` 方法，支持从外部动态库文件（.dylib/.so/.dll）加载插件，配置文件（config.json）与动态库文件同目录。

## 架构设计

### 核心流程

1. 接收动态库路径参数
2. 解析路径获取父目录
3. 在父目录中查找 `config.json`
4. 使用 `libloading` 加载动态库
5. 调用 `get_dylib_plugin` 导出函数获取插件实例
6. 解析配置文件获取元数据和默认配置
7. 构建 `PluginSlot::Dylib` 包装插件
8. 返回完整的 `UploadPluginInfo`

### 文件结构示例

```
plugins/
├── libuploader_example_plugin.dylib  # 动态库
└── config.json                       # 配置文件
```

## API 设计

### 方法签名

```rust
impl UploadPluginInfo {
    pub fn new_from_dylib_path(
        dylib_path: &str,
    ) -> Result<UploadPluginInfo, UploadError>
}
```

### 参数说明

- `dylib_path`: 动态库文件的绝对或相对路径
  - 支持跨平台：`libname.dylib` (macOS)、`libname.so` (Linux)、`libname.dll` (Windows)

### 返回值

- `Ok(UploadPluginInfo)`: 成功加载的插件信息
- `Err(UploadError)`: 加载失败，错误类型包括：
  - `CommonIoError`: 动态库或配置文件读取失败
  - `JsonSerializeError`: 配置文件解析失败
  - `PluginLoadError`: 动态库符号加载或插件初始化失败

### 与现有方法对比

| 方法 | 参数 | 插件类型 | 配置来源 |
|------|------|----------|----------|
| `new_in_process` | `config_path: &str`, `plugin: Box<dyn UploadPlugin>` | 进程内 | 参数指定的配置文件 |
| `new_from_dylib_path` | `dylib_path: &str` | 动态库 | 动态库同目录的 `config.json` |

## 数据流和实现细节

### 完整数据流

```
dylib_path (输入)
    ↓
1. 解析路径获取父目录
    ↓
2. 构建配置文件路径: {parent}/config.json
    ↓
3. 验证文件存在性
    ↓
4. 加载配置文件 → PluginMeta + default_config
    ↓
5. libloading::Library::new(dylib_path)
    ↓
6. 获取符号 "get_dylib_plugin" → FnGetDylibPlugin
    ↓
7. 调用函数 → SBox<dyn UploadDylibPlugin>
    ↓
8. 构建 PluginSlot::Dylib { plugin, _lib }
    ↓
9. 构建 UploadPluginInfo { id, meta, default_config, path, slot }
    ↓
UploadPluginInfo (返回)
```

### 关键实现细节

#### 1. 路径处理

```rust
use std::path::Path;
let dylib_path = Path::new(dylib_path);
let parent_dir = dylib_path.parent()
    .ok_or_else(|| UploadError::PluginLoadError("Invalid dylib path".to_string()))?;
let config_path = parent_dir.join("config.json");
```

#### 2. 动态库加载（保持引用）

```rust
let lib = Arc::new(unsafe {
    Library::new(dylib_path)
        .map_err(|e| UploadError::PluginLoadError(format!("Load dylib failed: {}", e)))?
});
```

#### 3. 符号获取和转换

```rust
let get_plugin: libloading::Symbol<FnGetDylibPlugin> = unsafe {
    lib.get(b"get_dylib_plugin")
        .map_err(|e| UploadError::PluginLoadError(format!("Get symbol failed: {}", e)))?
};
let plugin_box = get_plugin();
```

#### 4. 插件 ID 生成

```rust
let plugin_id = format!(
    "{}_{}_{}",
    "dylib",
    meta.name.clone(),
    meta.author.clone().unwrap_or("unknown".to_string())
);
```

## 错误处理

### 错误场景映射

| 场景 | 错误类型 | 错误信息 |
|------|----------|----------|
| dylib 路径无效（无父目录） | `PluginLoadError` | "Invalid dylib path: no parent directory" |
| config.json 不存在 | `CommonIoError` | (std::io::Error) |
| config.json 解析失败 | `JsonSerializeError` | (serde_json::Error) |
| dylib 文件不存在或无法加载 | `PluginLoadError` | "Load dylib failed: {libloading error}" |
| 符号 "get_dylib_plugin" 不存在 | `PluginLoadError` | "Get symbol failed: {libloading error}" |
| 配置文件缺少必要字段 | `JsonSerializeError` | (serde_json::Error) |

### 错误处理原则

1. 使用 `?` 操作符早期返回
2. 错误信息包含足够上下文用于调试
3. 所有错误都转换为 `UploadError` 统一类型
4. `unsafe` 块集中在 libloading 调用处，外部使用 `Result` 包装

### 示例错误处理

```rust
let lib = unsafe {
    Library::new(dylib_path)
        .map_err(|e| UploadError::PluginLoadError(format!("Load dylib failed: {}", e)))?
};
```

## 测试策略

### 测试覆盖范围

#### 1. 单元测试（plugin.rs 模块内）

- 成功加载动态库插件
- 路径无父目录的错误情况
- config.json 不存在的错误情况
- config.json 解析失败的错误情况
- 动态库加载失败的错误情况
- 符号获取失败的错误情况

#### 2. 集成测试（在 main.rs 中）

- 加载 `uploader_example_plugin` 编译产物
- 调用 `on_load()`、`execute()`、`on_unload()`
- 验证配置文件正确加载

### 测试准备

使用 build.rs 将编译产物复制到测试目录，或在测试中动态构建示例插件。

### 示例测试代码结构

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_load_dylib_plugin_success() {
        let plugin = UploadPluginInfo::new_from_dylib_path(
            "./test_plugins/libexample.dylib"
        );
        assert!(plugin.is_ok());
        // ... 更多断言
    }

    #[test]
    fn test_load_dylib_plugin_invalid_path() {
        let plugin = UploadPluginInfo::new_from_dylib_path("/invalid/path");
        assert!(plugin.is_err());
    }
}
```

## 依赖项

### 所需依赖（已存在）

| 依赖 | 用途 | 版本 | 位置 |
|------|------|------|------|
| `libloading` | 动态库加载 | 0.9.0 | workspace 根 Cargo.toml |
| `file_uploader_sdk` | SDK 接口和错误类型 | workspace | file_uploader_core/Cargo.toml |
| `serde` / `serde_json` | 配置解析 | workspace | 已使用 |
| `stabby` | ABI 稳定类型 | workspace | file_uploader_core 依赖 |
| `std::path::Path` | 路径处理 | std | - |
| `std::sync::Arc` | 线程安全 | std | - |

### 不需要新增依赖

## 后续工作

1. 实现阶段：调用 writing-plans 创建详细实现计划
2. 开发实现
3. 编写测试
4. 文档更新