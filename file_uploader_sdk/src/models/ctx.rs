use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use crate::models::enums::{
    FileDataType, OutputResultType, UploadPhase, UploadProcessStatus, UploadTaskStatus,
};
use chrono::Local;
use serde::{Deserialize, Serialize};

use stabby::sync::Arc as SArc;
use stabby::vec::Vec as SVec;

/**
 * 上传任务的context
 */
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadTaskCtx {
    // 任务id
    pub id: String,
    // 任务状态
    pub status: UploadTaskStatus,
    // 创建时间
    pub create_time: i64,
    // 开始时间
    pub start_time: Option<i64>,
    // 结束时间
    pub end_time: Option<i64>,
    // 名称
    pub name: String,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
}

impl UploadTaskCtx {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            status: UploadTaskStatus::Init,
            create_time: Local::now().timestamp_millis(),
            start_time: None,
            end_time: None,
            name,
            extra_info: None,
        }
    }

    pub fn put_extra(&mut self, key: String, value: String) {
        if self.extra_info.is_none() {
            self.extra_info = Some(HashMap::new());
        }
        self.extra_info.as_mut().unwrap().insert(key, value);
    }

    pub fn get_extra(&self, key: &str) -> Option<&String> {
        match &self.extra_info {
            Some(info) => info.get(key),
            None => None,
        }
    }
}

/**
 * 上传流程的context
 */
#[derive(Serialize, Deserialize, Debug)]
pub struct UploadProcessCtx {
    // 流程ID
    pub id: String,
    // 流程状态
    pub status: UploadProcessStatus,
    // 创建时间
    pub create_time: i64,
    // 开始时间
    pub start_time: Option<i64>,
    // 结束时间
    pub end_time: Option<i64>,
    // 当前阶段
    pub phase: UploadPhase,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
    // 关联任务
    #[serde(skip)]
    pub related_task_info: Weak<UploadTaskCtx>,
}

impl UploadProcessCtx {
    pub fn new(id: String, related_task_info: Weak<UploadTaskCtx>) -> Self {
        Self {
            id,
            status: UploadProcessStatus::Init,
            create_time: Local::now().timestamp_millis(),
            start_time: None,
            end_time: None,
            phase: UploadPhase::Input,
            extra_info: None,
            related_task_info,
        }
    }

    pub fn put_extra(&mut self, key: String, value: String) {
        if self.extra_info.is_none() {
            self.extra_info = Some(HashMap::new());
        }
        self.extra_info.as_mut().unwrap().insert(key, value);
    }

    pub fn get_extra(&self, key: &str) -> Option<&String> {
        match &self.extra_info {
            Some(info) => info.get(key),
            None => None,
        }
    }

    pub fn get_related_task(&self) -> Option<Arc<UploadTaskCtx>> {
        self.related_task_info.upgrade()
    }
}

/**
 * 文件数据
 */
#[derive(Serialize, Deserialize)]
pub struct UploadFileData {
    // 数据类型
    pub data_type: FileDataType,
    // 文件输入
    pub input_path: String,
    // 文件ID
    pub id: String,
    // 文件名
    pub name: String,
    // 文件类型
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
}

/**
 * 输入上下文
 */
#[derive(Serialize, Deserialize)]
pub struct UploadInputCtx {
    // 文件数据
    pub file_list: Vec<Arc<UploadFileData>>,
    // 关联流程
    #[serde(skip)]
    pub related_process_info: Weak<UploadProcessCtx>,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
}

/**
 * 输出结果
 */
#[derive(Serialize, Deserialize)]
pub struct UploadOutputCtx {
    // 输出结果
    pub result: OutputResultType,
    // 输出信息
    pub message: String,
    // 文件数据
    pub file_list: Option<Vec<Arc<UploadFileData>>>,
    // 扩展信息
    pub extra_info: Option<HashMap<String, String>>,
}
