use serde::{Deserialize, Serialize};

/// Upload phase
#[derive(Serialize, Deserialize, Debug, Clone)]
#[stabby::stabby]
#[repr(u8)]
pub enum UploadPhase {
    // Input
    Input,
    // Preparing to upload
    PreUpload,
    // Uploading
    Upload,
    // Upload complete
    PostUpload,
    // Output
    Output,
}

/// Task status
#[derive(Serialize, Deserialize, Debug)]
#[stabby::stabby]
#[repr(u8)]
pub enum UploadTaskStatus {
    // Initialization
    Init,
    // Running
    Running,
    // Paused
    Pause,
    // Completed
    Complete,
    // Failed
    Failed,
}

/// Process status
#[derive(Serialize, Deserialize, Debug)]
#[stabby::stabby]
#[repr(u8)]
pub enum UploadProcessStatus {
    // Waiting
    Wait,
    // 初始化
    Init,
    // 执行中
    Running,
    // 完成
    Complete,
    // 失败
    Failed,
}

/// 文件输入数据类型
#[derive(Serialize, Deserialize, Debug, Clone)]
#[stabby::stabby]
#[repr(u8)]
pub enum FileDataType {
    // 二进制数据
    Binary,
    // 文件系统路径
    FilePath,
    // 网络路径
    NetworkPath,
}

/// 输出结果类型
#[derive(Serialize, Deserialize, Debug, Clone)]
#[stabby::stabby]
#[repr(u8)]
pub enum OutputResultType {
    // 成功
    Success,
    // 失败
    Failed,
    // 中断
    Interrupt,
}

/// 上传组件配置类型
#[derive(Serialize, Deserialize, Debug, Clone)]
#[stabby::stabby]
#[repr(u8)]
pub enum UploadConfigType {
    // 默认值
    Default,
    // 自定义
    Custom,
}

/// 插件输入输出类型
#[derive(Serialize, Deserialize, Debug)]
#[stabby::stabby]
#[repr(u8)]
pub enum PluginIOType {
    // 1输入-1输出
    OneOne,
    // 1输入-N输出
    OneMany,
    // N输入-1输出
    ManyOne,
}

/// 插件日志级别
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[stabby::stabby]
#[repr(u8)]
pub enum PluginLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
