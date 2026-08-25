use std::{collections::HashMap, sync::Arc};

use crate::models::enums::{FileDataType, OutputResultType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stabby::sync::Arc as SArc;
use stabby::vec::Vec as SVec;

/**
 * 文件数据
 */
#[derive(Serialize, Deserialize, Clone)]
pub struct UploadFileData {
    // 数据类型
    pub data_type: FileDataType,
    // 文件输入
    pub input_path: String,
    // 文件ID
    pub id: String,
    // 文件名
    pub name: String,
    // 文件类型（MIME，由 Input 阶段 default_input_handler 魔数嗅探填充；未嗅探时为上游原值）
    pub file_type: String,
    // 文件大小
    pub size: usize,
    // 二进制数据
    #[serde(skip)]
    pub data: Option<SArc<SVec<u8>>>,
}

impl UploadFileData {
    pub fn new(
        data_type: FileDataType,
        input_path: String,
        id: String,
        name: String,
        file_type: String,
        size: usize,
    ) -> Self {
        Self {
            data_type,
            input_path,
            id,
            name,
            file_type,
            size,
            data: None,
        }
    }

    /// 内存字节构造：data_type=Binary，`data` 为权威数据，
    /// `input_path` 仅保留来源路径作标识（不保证可读），size 取字节长度。
    pub fn binary(
        input_path: String,
        id: String,
        name: String,
        file_type: String,
        data: Vec<u8>,
    ) -> Self {
        Self {
            data_type: crate::models::enums::FileDataType::Binary,
            input_path,
            id,
            name,
            file_type,
            size: data.len(),
            data: Some(stabby::sync::Arc::new(stabby::vec::Vec::from(data.as_slice()))),
        }
    }
}

/**
 * 输入上下文
 */
#[derive(Serialize, Deserialize)]
pub struct UploadInputCtx {
    // 文件数据
    pub file: Option<Arc<UploadFileData>>,
    // 配置信息
    pub config_info: Arc<Option<Value>>,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
    // 活动目录：本次执行流程的唯一工作目录（流程级常量，宿主预创建并透传）。
    // None 表示未设置。#[serde(default)] 兼容旧 JSON。
    #[serde(default)]
    pub work_dir: Option<String>,
}

/**
 * 输出结果
 */
#[derive(Serialize, Deserialize, Clone)]
pub struct UploadOutputCtx {
    // 输出结果
    pub result: OutputResultType,
    // 输出信息
    pub message: String,
    // 文件数据
    pub file: Option<Arc<UploadFileData>>,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
}

impl UploadOutputCtx {
    /// 成功（无文件产出）
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Success,
            message: msg.into(),
            file: None,
            extra_info: None,
        }
    }

    /// 成功（携带处理后文件）
    pub fn success_file(msg: impl Into<String>, file: Arc<UploadFileData>) -> Self {
        Self {
            result: OutputResultType::Success,
            message: msg.into(),
            file: Some(file),
            extra_info: None,
        }
    }

    /// 失败（会中断 pipeline）
    pub fn failed(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Failed,
            message: msg.into(),
            file: None,
            extra_info: None,
        }
    }

    /// 中断
    pub fn interrupt(msg: impl Into<String>) -> Self {
        Self {
            result: OutputResultType::Interrupt,
            message: msg.into(),
            file: None,
            extra_info: None,
        }
    }
}

#[cfg(test)]
mod output_helper_tests {
    use super::*;
    use crate::models::enums::OutputResultType;

    #[test]
    fn success_has_no_file() {
        let o = UploadOutputCtx::success("ok");
        assert!(matches!(o.result, OutputResultType::Success));
        assert_eq!(o.message, "ok");
        assert!(o.file.is_none());
        assert!(o.extra_info.is_none());
    }

    #[test]
    fn success_file_carries_file() {
        let f = Arc::new(UploadFileData::new(
            crate::models::enums::FileDataType::FilePath,
            "/tmp/a".into(),
            "a".into(),
            "a".into(),
            "image/png".into(),
            0,
        ));
        let o = UploadOutputCtx::success_file("done", f);
        assert!(matches!(o.result, OutputResultType::Success));
        assert_eq!(o.file.as_ref().unwrap().name, "a");
    }

    #[test]
    fn failed_sets_failed_result() {
        let o = UploadOutputCtx::failed("boom");
        assert!(matches!(o.result, OutputResultType::Failed));
        assert_eq!(o.message, "boom");
        assert!(o.file.is_none());
    }

    #[test]
    fn interrupt_sets_interrupt_result() {
        let o = UploadOutputCtx::interrupt("stop");
        assert!(matches!(o.result, OutputResultType::Interrupt));
        assert_eq!(o.message, "stop");
    }
}
