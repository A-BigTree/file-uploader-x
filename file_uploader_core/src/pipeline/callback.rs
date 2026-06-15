use crate::pipeline::plugin::PluginMeta;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::UploadPhase;

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineEventKind {
    PhaseStart,
    PhaseEnd,
    PluginStart,
    PluginEnd,
}

pub struct PipelineEvent<'a> {
    /// 回调触发瞬间的毫秒时间戳（当地系统时间，i64 epoch 毫秒）
    pub timestamp_ms: i64,
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
    /// 插件元信息：插件级事件填充对应插件；阶段级事件为 None
    pub plugin_meta: Option<&'a PluginMeta>,
}

pub trait PipelineCallback: Send + Sync {
    fn on_event(
        &self,
        event: &PipelineEvent,
        ctx: &UploadInputCtx,
        result: Option<&UploadOutputCtx>,
    );
}
