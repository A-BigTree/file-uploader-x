use crate::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use crate::models::ctx_stabby::{UploadFileDataS, UploadInputCtxS, UploadOutputCtxS};
use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use stabby::option::Option as SOption;
use stabby::string::String as SString;
use stabby::sync::Arc as SArc;
use stabby::vec::Vec as SVec;
use tracing::error;

pub fn convert_file_data_s(input: &UploadFileData) -> UploadFileDataS {
    UploadFileDataS {
        data_type: input.data_type.clone(),
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

    let config_info: SOption<SString> = match &input.config_info {
        None => None.into(),
        Some(config) => {
            if let Ok(json) = serde_json::to_string(config) {
                SOption::Some(json.into())
            } else {
                error!("Failed to serialize config info");
                None.into()
            }
        },
    };

    UploadInputCtxS {
        file_list,
        config_info,
        extra_info,
    }
}

pub fn convert_input_ctx(input: &UploadInputCtxS) -> UploadInputCtx {
    let file_list: Vec<Arc<UploadFileData>> = input
        .file_list
        .iter()
        .map(|file| Arc::new(convert_file_data(file)))
        .collect();

    let extra_info: Option<HashMap<String, String>> = input.extra_info.match_ref(
        |extra_info_s| {
            return if let Ok(map) = serde_json::from_str(extra_info_s) {
                Some(map)
            } else {
                error!("Failed to deserialize extra info");
                None
            };
        },
        || None,
    );
    UploadInputCtx {
        file_list,
        config_info: input.config_info.match_ref(
            |config_info_s| {
                get_config(config_info_s)
            },
            || None,
        ),
        extra_info,
        related_process_info: None,
    }
}

pub fn convert_file_data(input: &UploadFileDataS) -> UploadFileData {
    UploadFileData {
        data_type: input.data_type.clone(),
        input_path: input.input_path.clone().into(),
        id: input.id.clone().into(),
        name: input.name.clone().into(),
        file_type: input.file_type.clone().into(),
        size: input.size,
        data: input.data.clone().into(),
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
            };
        },
        || None,
    );

    UploadOutputCtx {
        result: input.result.clone(),
        message: input.message.clone().into(),
        file_list,
        extra_info,
    }
}

/// 获取配置Value
pub fn get_config(config: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str(config) {
        Some(value)
    } else {
        error!("Failed to deserialize config");
        None
    }
}
