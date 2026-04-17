

// Upload phase
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

// Task status
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

// Process status
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

// 文件输入数据类型
pub enum FileInputDataType {
    // 二进制数据
    Binary,
    // 文件系统路径
    FilePath,
    // 网络路径
    NetworkPath,
}
