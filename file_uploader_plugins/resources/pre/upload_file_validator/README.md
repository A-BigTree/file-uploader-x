# 上传文件校验器（upload_file_validator）

> 阶段：`PreUpload` ｜ 版本：`0.0.1` ｜ 作者：`A-BigTree` ｜ 类型：进程内插件

## 1. 功能简介

在真正上传之前，按**文件类型(MIME)、文件名、体积**三个维度校验单个文件。任一维度不通过即返回
`Failed`，立即中断整条 pipeline。

## 2. 权限声明（access）

| 权限 | 取值 | 说明 |
|---|---|---|
| `fs_read` | `false` | 只读 ctx 元信息，不触碰文件内容 |
| `fs_write` | `false` | 不写文件 |
| `network` | `false` | 不联网 |

## 3. 分组（groups）

无分组。本插件为单形态插件，运行态配置中**不应**出现 `group` 字段。

## 4. 参数说明

### 4.1 公共参数（common）

| key | 标题 | 控件 | 必填 | 默认值 | 约束 | 说明 |
|---|---|---|---|---|---|---|
| `pass_type` | 允许类型 | `select` 多选 | 否 | `[]` | `allow_custom`，最多 50 项 | 允许的 MIME，支持 glob（`image/*`）；空 = 全部允许 |
| `reject_type` | 拦截类型 | `select` 多选 | 否 | `[]` | `allow_custom`，最多 50 项 | 命中即剔除，优先级高于 `pass_type` |
| `pass_name` | 允许文件名 | `select` 多选 | 否 | `[]` | `allow_custom` | 文件名 glob（`*.png`）；空 = 全部允许 |
| `max_size` | 最大体积 | `text` | 否 | `"0"` | 最长 16 字符，需匹配 `数字+可选单位` | 支持 `1kb`/`10mb`/`1g`；`0` 或空 = 不限 |
| `strict_mode` | 严格模式 | `switch` | 否 | `false` | bool | 开启后 `file_type` 为空即拒绝 |

### 4.2 分组参数

无。

## 5. 运行态配置示例

```json
{
  "pass_type": ["image/*", "application/pdf"],
  "reject_type": [],
  "pass_name": ["*.png", "*.jpg", "*.pdf"],
  "max_size": "10mb",
  "strict_mode": true
}
```

## 6. 输入 / 输出契约

**读取 `UploadInputCtx`**
- `file`：必需。为 `None` 时返回 `Failed`
- `file.file_type` / `file.name` / `file.size`：三个校验维度的数据来源（由 `Input` 阶段填充）
- `config_info`：见上方参数

**写出 `UploadOutputCtx`**
- 通过：`Success` + 原样透传 `file`
- 不通过：`Failed`，`message` 内含实际生效的四项配置便于排错
- `extra_info`：不写入

**保留判定逻辑**

```
(pass_type 为空 ∨ 命中 pass_type)
∧ (未命中 reject_type)
∧ (pass_name 为空 ∨ 命中 pass_name)
∧ (max_size 为 0/空 ∨ size ≤ max_size)
∧ (strict_mode 关闭 ∨ file_type 非空)
```

## 7. 校验规则

**框架声明式校验**：`max_size` 的长度与单位正则、`select` 的数量上限、`strict_mode` 的 bool 类型。

**插件 `validate_params`（业务级）**
1. `max_size` 非空时必须能被 `config_util::parse_size` 解析
2. `pass_type` / `reject_type` / `pass_name` 的每一项都必须是合法的 glob 模式（`glob::Pattern::new`）

## 8. 错误与排错

| message 关键字 | 原因 | 处理建议 |
|---|---|---|
| `no file to validate` | ctx 中无 file | 检查上游 `Input` 阶段是否正常产出 |
| `rejected (pass_type=..., ...)` | 三维校验未通过 | 对照 message 中打印的生效配置调整 |
| `max_size 无法解析` | 注册配置里的体积字符串非法 | 改为 `10mb` 这类合法写法 |
| `非法 glob '<p>'` | 模式串语法错误 | 修正 glob 表达式 |
| `invalid glob pattern`（warn） | 执行期遇到非法模式 | 非致命，该模式被跳过 |

## 9. 变更记录

- 0.0.1 初始版本；`file_type_filter` 更名而来，配置改为 `common`，新增 `strict_mode` 与 `validate_params`
