# 动态加载测试插件（uploader_test_example_plugin）

> 阶段：`PreUpload` ｜ 版本：`0.0.1` ｜ 作者：`A-BigTree` ｜ 类型：**动态库插件（cdylib）**

## 1. 功能简介

框架的 dylib 插件参考实现，用于验证 stabby ABI 通路：日志回调、`execute`、`validate_params`、
以及 **`common` + `groups` 两层配置**的完整示范。业务上不做实际上传，仅打印入参。

## 2. 权限声明（access）

| 权限 | 取值 | 说明 |
|---|---|---|
| `fs_read` | `true` | 示范用（`local` 分组需读本地目录） |
| `fs_write` | `false` | 不写文件 |
| `network` | `true` | 示范用（`oss` 分组需联网） |

## 3. 分组（groups）

本插件为**多形态插件**，运行态配置中 `group` 字段**必填**，且只能激活其中一个：

| group | 标题 | 适用场景 |
|---|---|---|
| `oss` | 阿里云 OSS | 上传到阿里云对象存储，需 endpoint + 凭证 |
| `local` | 本地存储 | 落地到本机目录，需根目录 + 命名策略 |

两者互斥：激活 `oss` 时 `local` 的参数不参与校验，反之亦然。

## 4. 参数说明

### 4.1 公共参数（common，两个分组都生效）

| key | 标题 | 控件 | 必填 | 默认值 | 约束 | 说明 |
|---|---|---|---|---|---|---|
| `pass_type` | 允许上传的文件类型 | `select` 多选 | 否 | `[]` | `allow_custom` | 空 = 全部允许 |
| `reject_type` | 不允许上传的文件类型 | `select` 多选 | 否 | `[]` | `allow_custom` | 空 = 不拦截 |
| `retry_times` | 重试次数 | `number` | 否 | `3` | `0..=10`，整数 | 失败重试 |

### 4.2 分组 `oss` 参数

| key | 标题 | 控件 | 必填 | 默认值 | 约束 | 说明 |
|---|---|---|---|---|---|---|
| `endpoint` | Endpoint | `text` | **是** | `""` | 长度 8–256，须匹配 `^https?://.+` | OSS 服务地址 |
| `bucket` | Bucket | `text` | **是** | `""` | 长度 3–63，须匹配桶名规则 | 存储桶名称 |
| `access_key` | AccessKey ID | `text` | **是** | `""` | 长度 1–128 | 访问凭证 ID |
| `access_secret` | AccessKey Secret | `text` **密码框** | **是** | `""` | 长度 1–256 | `secret: true`，前端不回显 |
| `use_https` | 使用 HTTPS | `switch` | 否 | `true` | bool | 传输是否加密 |

### 4.3 分组 `local` 参数

| key | 标题 | 控件 | 必填 | 默认值 | 约束 | 说明 |
|---|---|---|---|---|---|---|
| `base_dir` | 存储根目录 | `text` | **是** | `/tmp/uploads` | 长度 1–512，须以 `/` 开头 | 落地绝对路径 |
| `dir_mode` | 目录权限 | `number` | 否 | `493` | `0..=511`，整数 | 十进制表示的八进制值（755 → 493） |
| `overwrite` | 允许覆盖 | `switch` | 否 | `false` | bool | 同名文件是否覆盖 |
| `naming` | 命名策略 | `select` 单选 | **是** | `uuid` | **非** `allow_custom`，只能取 `origin`/`uuid`/`hash` | 落地命名方式 |

## 5. 运行态配置示例

激活 `oss`：

```json
{
  "group": "oss",
  "pass_type": ["image/*"],
  "retry_times": 3,
  "endpoint": "https://oss-cn-hangzhou.aliyuncs.com",
  "bucket": "my-bucket",
  "access_key": "AK...",
  "access_secret": "SK...",
  "use_https": true
}
```

激活 `local`：

```json
{
  "group": "local",
  "retry_times": 1,
  "base_dir": "/tmp/uploads",
  "dir_mode": 493,
  "overwrite": false,
  "naming": "uuid"
}
```

## 6. 输入 / 输出契约

**读取 `UploadInputCtxS`**（经 `convert_input_ctx` 转为原生 ctx 后访问）
- `config_info`：JSON 字符串跨 ABI 传递，转换后可用 `config_util::get_*` / `get_group` 读取
- 其余字段仅打印，不做业务处理

**写出 `UploadOutputCtxS`**
- 恒定 `Success`，`message = "成功"`，`file` / `extra_info` 均为 `None`

## 7. 校验规则

**框架声明式校验**：`group` 存在性与合法性、各参数的 required / 长度 / 正则 / 数值范围 /
`naming` 的候选项合法性。

**插件 `validate_params`（业务级）**：校验 `group` 只能是 `oss` 或 `local`，缺失或未知则返回错误信息。

## 8. 错误与排错

| 错误信息 | 原因 | 处理建议 |
|---|---|---|
| `缺少分组标识 group` | 未传 `group` | 补上 `"group": "oss"` 或 `"local"` |
| `未知分组 'xxx'` | `group` 值不在声明内 | 只能用 `oss` / `local` |
| `[oss.endpoint] 必填项未填` | 激活 `oss` 但缺 endpoint | 补齐该分组必填项 |
| `[local.naming] 值不在候选项内` | `naming` 传了非法值 | 改为 `origin`/`uuid`/`hash` |
| `Load dylib failed` | 产物缺失或 ABI 不匹配 | `cargo build --workspace` 全量重编 |
| `Failed to read plugin.id` | 产物同目录缺 `plugin.id` | 检查 `build.rs` 复制是否成功 |

## 9. 变更记录

- 0.0.1 初始版本；配置升级为 `common` + `groups` 两层，新增 `validate_params` 与四类控件示范

---

## 附：构建产物

`build.rs` 会把 `meta.json`、`config.json`、`plugin.id`、`README.md` 复制到与
`libuploader_example_plugin.dylib` 相同的目录（`target/<profile>/`），
供 `UploadPluginInfo::new_from_dylib_path` 通过 `parent()` 定位资源。

> **注意**：`UploadDylibPlugin` trait 每次新增方法都会改变 stabby vtable 布局，
> 旧产物与新宿主不兼容，必须 `cargo build --workspace` 一并重编。
