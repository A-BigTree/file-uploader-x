use file_uploader_sdk::models::ctx_stabby::{UploadInputCtxS, UploadOutputCtxS};
use file_uploader_sdk::models::enums::OutputResultType;
use file_uploader_sdk::models::interface::UploadDylibPlugin;
use file_uploader_sdk::utils::ctx_util::convert_input_ctx;
use tracing::info;

pub struct UploaderTestExamplePlugin;

impl UploadDylibPlugin for UploaderTestExamplePlugin {
    extern "C" fn execute(&self, ctx: &UploadInputCtxS) -> UploadOutputCtxS {
        info!("UploaderTestExamplePlugin execute...");
        let ctx = convert_input_ctx(ctx);
        let json = serde_json::to_string(&ctx).unwrap_or("input error".to_string());
        info!("UploaderTestExamplePlugin get input: {}", json);
        UploadOutputCtxS {
            result: OutputResultType::Success,
            message: "成功".to_string().into(),
            file_list: stabby::option::Option::None(),
            extra_info: stabby::option::Option::None(),
        }
    }

    extern "C" fn on_load(&self) {
        info!("UploaderTestExamplePlugin loading...")
    }

    extern "C" fn on_unload(&self) {
        info!("UploaderTestExamplePlugin unloading...")
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn get_dylib_plugin()
-> stabby::dynptr!(stabby::boxed::Box<dyn UploadDylibPlugin + Send + Sync>) {
    stabby::boxed::Box::new(UploaderTestExamplePlugin).into()
}
