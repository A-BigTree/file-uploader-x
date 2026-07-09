use crate::models::ctx::{UploadFileData, UploadInputCtx, UploadOutputCtx};
use crate::models::ctx_stabby::{UploadFileDataS, UploadInputCtxS, UploadOutputCtxS};
use serde_json::Value;
use stabby::option::Option as SOption;
use stabby::string::String as SString;
use stabby::sync::Arc as SArc;
use std::collections::HashMap;
use std::sync::Arc;
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
    let file = match &input.file {
        Some(f) => SOption::Some(SArc::new(convert_file_data_s(f))),
        None => stabby::option::Option::None(),
    };

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

    let config_info: SOption<SString> = match &*input.config_info {
        None => None.into(),
        Some(config) => {
            if let Ok(json) = serde_json::to_string(config) {
                SOption::Some(json.into())
            } else {
                error!("Failed to serialize config info");
                None.into()
            }
        }
    };

    UploadInputCtxS {
        file,
        config_info,
        extra_info,
        work_dir: match &input.work_dir {
            None => None.into(),
            Some(s) => SOption::Some(s.clone().into()),
        },
    }
}

pub fn convert_input_ctx(input: &UploadInputCtxS) -> UploadInputCtx {
    let file: Option<Arc<UploadFileData>> = input.file.match_ref(
        |f| Some(Arc::new(convert_file_data(f))),
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
    UploadInputCtx {
        file,
        config_info: input.config_info.match_ref(
            |config_info_s| Arc::new(get_config(config_info_s)),
            || Arc::new(None),
        ),
        extra_info,
        related_process_info: None,
        work_dir: input.work_dir.match_ref(|s| Some(s.clone().into()), || None),
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
    let file: Option<Arc<UploadFileData>> = input.file.match_ref(
        |f| Some(Arc::new(convert_file_data(f))),
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
        file,
        extra_info,
    }
}

#[cfg(test)]
mod work_dir_tests {
    use super::*;
    use crate::models::ctx::UploadFileData;
    use crate::models::enums::FileDataType;

    fn input_with_work_dir(wd: Option<&str>) -> UploadInputCtx {
        UploadInputCtx {
            file: Some(Arc::new(UploadFileData::new(
                FileDataType::FilePath,
                "/tmp/a".into(),
                "a".into(),
                "a".into(),
                "image/png".into(),
                0,
            ))),
            config_info: Arc::new(None),
            extra_info: None,
            related_process_info: None,
            work_dir: wd.map(String::from),
        }
    }

    #[test]
    fn roundtrip_preserves_work_dir_some() {
        let original = input_with_work_dir(Some("/data/wd-1"));
        let s = convert_input_ctx_s(&original);
        let back = convert_input_ctx(&s);
        assert_eq!(back.work_dir.as_deref(), Some("/data/wd-1"));
    }

    #[test]
    fn roundtrip_preserves_work_dir_none() {
        let original = input_with_work_dir(None);
        let s = convert_input_ctx_s(&original);
        let back = convert_input_ctx(&s);
        assert!(back.work_dir.is_none());
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
