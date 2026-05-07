use stabby::option::Option as SOption;
use stabby::string::String as SString;
use stabby::sync::Arc as SArc;
use stabby::vec::Vec as SVec;

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
    pub input_path: SString,
    // 文件ID
    pub id: SString,
    // 文件名
    pub name: SString,
    // 文件类型
    pub file_type: SString,
    // 文件大小
    pub size: usize,
    // 二进制数据
    pub data: SOption<SArc<SVec<u8>>>,
}

/**
 * 输入上下文
 */
#[stabby::stabby]
pub struct UploadInputCtxS {
    // 文件数据
    pub file_list: SVec<SArc<UploadFileDataS>>,
    // 扩展信息
    pub extra_info: SOption<SString>,
}

/**
 * 输出结果
 */
#[stabby::stabby]
pub struct UploadOutputCtxS {
    // 输出结果
    pub result: OutputResultTypeS,
    // 输出信息
    pub message: SString,
    // 文件数据
    pub file_list: SOption<SVec<SArc<UploadFileDataS>>>,
    // 扩展信息
    pub extra_info: SOption<SString>,
}
