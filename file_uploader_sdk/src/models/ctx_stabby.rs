use crate::models::enums::{FileDataType, OutputResultType};
use serde::{Deserialize, Serialize};
use stabby::option::Option as SOption;
use stabby::string::String as SString;
use stabby::sync::Arc as SArc;
use stabby::vec::Vec as SVec;

/**
 * 文件数据
 */
#[stabby::stabby]
pub struct UploadFileDataS {
    // 数据类型
    pub data_type: FileDataType,
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
    // 配置信息
    pub config_info: SOption<SString>,
    // 扩展信息
    pub extra_info: SOption<SString>,
}

/**
 * 输出结果
 */
#[stabby::stabby]
pub struct UploadOutputCtxS {
    // 输出结果
    pub result: OutputResultType,
    // 输出信息
    pub message: SString,
    // 文件数据
    pub file_list: SOption<SVec<SArc<UploadFileDataS>>>,
    // 扩展信息
    pub extra_info: SOption<SString>,
}
