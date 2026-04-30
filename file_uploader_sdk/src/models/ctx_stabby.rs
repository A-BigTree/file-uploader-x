use stabby::option::Option;
use stabby::string::String;
use stabby::sync::Arc;
use stabby::vec::Vec;

#[stabby::stabby]
#[repr(u8)]
pub enum FileDataTypeS {
    // 二进制数据
    Binary,
    // 文件系统路径
    FilePath,
    // 网络路径
    NetworkPath,
}

// 输出结果类型
#[stabby::stabby]
#[repr(u8)]
pub enum OutputResultTypeS {
    // 成功
    Success,
    // 失败
    Failed,
    // 中断
    Interrupt,
}

/**
 * 文件数据
 */
#[stabby::stabby]
pub struct UploadFileDataS {
    // 数据类型
    pub data_type: FileDataTypeS,
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
    pub data: Option<Arc<Vec<u8>>>,
}

/**
 * 输入上下文
 */
#[stabby::stabby]
pub struct UploadInputCtxS {
    // 文件数据
    pub file_list: Vec<Arc<UploadFileDataS>>,
    // 扩展信息
    pub extra_info: Option<String>,
}

/**
 * 输出结果
 */
#[stabby::stabby]
pub struct UploadOutputCtxS {
    // 输出结果
    pub result: OutputResultTypeS,
    // 输出信息
    pub message: String,
    // 文件数据
    pub file_list: Option<Vec<Arc<UploadFileDataS>>>,
    // 扩展信息
    pub extra_info: Option<String>,
}
