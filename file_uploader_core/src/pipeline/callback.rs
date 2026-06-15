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
    pub kind: PipelineEventKind,
    pub phase: UploadPhase,
    pub plugin_id: Option<&'a str>,
}

pub trait PipelineCallback: Send + Sync {
    fn on_event(
        &self,
        event: &PipelineEvent,
        ctx: &UploadInputCtx,
        result: Option<&UploadOutputCtx>,
    );
}
