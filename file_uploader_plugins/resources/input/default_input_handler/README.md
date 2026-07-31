# 默认输入处理器（default_input_handler）

> 阶段：`Input` ｜ 版本：`0.0.1` ｜ 作者：`A-BigTree` ｜ 类型：进程内插件

## 1. 功能简介

Pipeline 的第一站。把外部输入（本地路径 / 网络 URL）引入本次执行的工作目录，并用魔数嗅探填充
`file_type`(MIME)，为下游 `PreUpload` 校验插件提供可信的类型与体积信息。

## 2. 权限声明（access）

| 权限 | 取值 | 说明 |
|---|---|---|
| `fs_read` | `true` | 需读取用户传入的外部本地文件 |
| `fs_write` | `true` | 需把文件写入 `work_dir` 沙箱 |
| `network` | `true` | 需下载 `NetworkPath` 类型的输入 |

## 3. 分组（groups）

无分组。本插件为单形态插件，运行态配置中**不应**出现 `group` 字段。

## 4. 参数说明

### 4.1 公共参数（common）

| key | 标题 | 控件 | 必填 | 默认值 | 约束 | 说明 |
|---|---|---|---|---|---|---|
| `cache_local` | 缓存本地文件 | `switch` | 否 | `true` | bool | `FilePath` 输入是否拷贝进 `work_dir` |
| `download_network` | 下载网络文件 | `switch` | 否 | `true` | bool | 关闭则保留 URL，交由下游自行下载 |
| `sniff_type` | 嗅探文件类型 | `switch` | 否 | `true` | bool | 用 `infer` 读前 512 字节判定 MIME |
| `download_timeout_secs` | 下载超时(秒) | `number` | 否 | `30` | `1..=3600`，整数 | 网络下载超时 |

### 4.2 分组参数

无。

## 5. 运行态配置示例

```json
{
  "cache_local": true,
  "download_network": true,
  "sniff_type": true,
  "download_timeout_secs": 30
}
```

## 6. 输入 / 输出契约

**读取 `UploadInputCtx`**
- `file`：待处理文件；为 `None` 时直接成功返回（无文件场景）
- `work_dir`：沙箱工作目录。当 `cache_local` 或 `download_network` 生效时**必需**，缺失返回 `WorkDirNotSet`
- `config_info`：见上方参数

**写出 `UploadOutputCtx`**
- `result`：`Success` / `Failed`
- `file`：处理后的文件（`input_path` 指向沙箱内路径，`size` 与 `file_type` 已刷新）
- `extra_info`：不写入

**按 `data_type` 的行为**
- `FilePath`：嗅探类型 → 可选拷贝入沙箱 → 刷新 `size` / `input_path`
- `NetworkPath`：可选下载入沙箱 → 刷新 `size` / `input_path` → 嗅探类型
- `Binary`：原样透传（字段预留，当前不处理内存数据）

## 7. 校验规则

框架声明式校验：四个参数的类型与数值范围（见 4.1「约束」列）。

插件 `validate_params`：无额外业务校验（采用默认实现）。

## 8. 错误与排错

| message 关键字 | 原因 | 处理建议 |
|---|---|---|
| `work_dir is not set` | 开启了缓存/下载但未传 `work_dir` | 宿主预创建工作目录并注入 ctx |
| `download failed for <url>` | 网络不通 / DNS 失败 | 检查网络与 URL 可达性 |
| `download <url> returned status` | 远端返回非 2xx | 检查 URL 与鉴权 |
| `sniff failed for ...`（warn） | 魔数无法识别 | 非致命，`file_type` 保留上游原值 |

## 9. 变更记录

- 0.0.1 初始版本；配置改为 `common` + `switch` / `number` 控件，新增 `download_timeout_secs`
