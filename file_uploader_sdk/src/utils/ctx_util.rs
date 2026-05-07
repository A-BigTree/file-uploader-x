use std::collections::HashMap;
use std::sync::Arc;
use crate::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use crate::models::ctx_stabby::{FileDataTypeS, OutputResultTypeS, UploadFileDataS, UploadInputCtxS, UploadOutputCtxS};
use crate::models::enums::{FileDataType, OutputResultType};

use stabby::option::Option as SOption;
use stabby::string::String as SString;
use stabby::sync::Arc as SArc;
use stabby::vec::Vec as SVec;
use tracing::error;

pub fn convert_file_data_type_s(data: &FileDataType) -> FileDataTypeS {
    match data {
        FileDataType::Binary => FileDataTypeS::Binary,
        FileDataType::FilePath => FileDataTypeS::FilePath,
        FileDataType::NetworkPath => FileDataTypeS::NetworkPath,
    }
}

pub fn convert_file_data_s(input: &UploadFileData) -> UploadFileDataS {
    UploadFileDataS {
        data_type: convert_file_data_type_s(&input.data_type),
        input_path: input.input_path.clone().into(),
        id: input.id.clone().into(),
        name: input.name.clone().into(),
        file_type: input.file_type.clone().into(),
        size: input.size,
        data: input.data.clone().into(),
    }
}

pub fn convert_input_ctx_s(input: &UploadInputCtx) -> UploadInputCtxS {
    let file_list: SVec<SArc<UploadFileDataS>> = input
        .file_list
        .iter()
        .map(|file| SArc::new(convert_file_data_s(file)))
        .collect();

    let extra_info: SOption<SString> = match &input.extra_info {
        None => None.into(),
        Some(map) => {
            if let Ok(json) = serde_json::to_string(map) {
                SOption::Some(json.into())
            } else {
                error!("Failed to serialize extra info");
                None.into()
            }
        }
    };

    UploadInputCtxS {
        file_list,
        extra_info,
    }
}

pub fn convert_file_data_type(data: &FileDataTypeS) -> FileDataType {
    match data {
        FileDataTypeS::Binary => FileDataType::Binary,
        FileDataTypeS::FilePath => FileDataType::FilePath,
        FileDataTypeS::NetworkPath => FileDataType::NetworkPath,
    }
}

pub fn convert_file_data(input: &UploadFileDataS) -> UploadFileData {
    UploadFileData {
        data_type: convert_file_data_type(&input.data_type),
        input_path: input.input_path.clone().into(),
        id: input.id.clone().into(),
        name: input.name.clone().into(),
        file_type: input.file_type.clone().into(),
        size: input.size,
        data: input.data.clone().into(),
    }
}

pub fn convert_output_result_type(data: &OutputResultTypeS) -> OutputResultType {
    match data {
        OutputResultTypeS::Success => OutputResultType::Success,
        OutputResultTypeS::Failed => OutputResultType::Failed,
        OutputResultTypeS::Interrupt => OutputResultType::Interrupt,
    }
}

pub fn convert_output_ctx(input: &UploadOutputCtxS) -> UploadOutputCtx {
    let file_list: Option<Vec<Arc<UploadFileData>>> = input.file_list.match_ref(
        |file_list_s| {
            let file_list: Vec<Arc<UploadFileData>> = file_list_s
                .iter()
                .map(|file| Arc::new(convert_file_data(file)))
                .collect();
            Some(file_list)
        },
        || None,
    );

    let extra_info: Option<HashMap<String, String>> = input.extra_info.match_ref(
        |extra_info_s| {
            return if let Ok(map) = serde_json::from_str(extra_info_s) {
                Some(map)
            } else {
                error!("Failed to deserialize extra info");
                None
            }
        },
        || None,
    );

    UploadOutputCtx {
        result: convert_output_result_type(&input.result),
        message: input.message.clone().into(),
        file_list,
        extra_info,
    }
}
