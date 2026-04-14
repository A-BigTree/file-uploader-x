

// 上传阶段
pub enum UploadPhase {
    // 输入
    Input,
    // 准备上传
    PreUpload,
    // 上传中
    Upload,
    // 上传完成
    PostUpload,
    // 输出
    Output,
}

// 任务状态
pub enum UploadTaskStatus {
    // 初始化
    Init,
    // 执行中
    Running,
    // 暂停
    Pause,
    // 完成
    Complete,
    // 失败
    Failed,
}

// 流程状态
pub enum UploadProcessStatus {
    // 等待
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
