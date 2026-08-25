//! 阶段编排：宿主可按阶段注入编排回调，接管该阶段全部插件的执行顺序与策略。

use crate::pipeline::callback::{PipelineCallback, PipelineEvent, PipelineEventKind};
use crate::pipeline::registry::{PluginRegistryInfo, UploadPluginRegistryTable};
use file_uploader_sdk::error::UploadError;
use file_uploader_sdk::models::ctx::{UploadInputCtx, UploadOutputCtx};
use file_uploader_sdk::models::enums::{OutputResultType, UploadPhase};

/// 阶段编排器：接管某一阶段全部插件的执行顺序与策略。
///
/// 约定：
/// - 返回 `Err` → 整条 pipeline 以 Failed 终止；
/// - 返回 `Ok(输出)` 且输出 `result == Failed` → pipeline 中断后续阶段；
/// - 编排回调应遵循「插件输出 Failed 即短路」的历史语义。
pub trait StageExecute: Send + Sync {
    fn execute(&self, ctx: &mut StageExecutionContext) -> Result<UploadOutputCtx, UploadError>;
}

/// 阶段执行上下文：框架构造，回调方只做编排决策。
pub struct StageExecutionContext<'a> {
    phase: UploadPhase,
    plugins: Vec<&'a PluginRegistryInfo>,
    callback: Option<&'a dyn PipelineCallback>,
    current_ctx: UploadInputCtx,
}

impl<'a> StageExecutionContext<'a> {
    pub(crate) fn new(
        phase: UploadPhase,
        plugins: Vec<&'a PluginRegistryInfo>,
        callback: Option<&'a dyn PipelineCallback>,
        current_ctx: UploadInputCtx,
    ) -> Self {
        StageExecutionContext {
            phase,
            plugins,
            callback,
            current_ctx,
        }
    }

    /// 当前阶段。
    pub fn phase(&self) -> &UploadPhase {
        &self.phase
    }

    /// 本阶段已排序（phase → priority）的启用插件列表。
    pub fn plugins(&self) -> &[&PluginRegistryInfo] {
        &self.plugins
    }

    /// 阶段当前输入 ctx（随 `run_plugin` 推进）。
    pub fn current_input(&self) -> &UploadInputCtx {
        &self.current_ctx
    }

    /// 替换阶段当前输入 ctx（自定义编排器用于输入重置/恢复）。
    pub fn reset_input(&mut self, ctx: UploadInputCtx) {
        self.current_ctx = ctx;
    }

    /// 执行本阶段第 `idx` 个插件：注入 registry_config、发 PluginStart/PluginEnd 事件、
    /// 执行插件、完成 output→input 转换并推进 `current_ctx`。
    ///
    /// - 插件 `execute` 返回 `Err`：发携带失败输出的 PluginEnd 后返回 `Err`；
    /// - 输出 `Failed`：返回 `Ok(输出)`，由调用方（编排器/pipeline）决定短路。
    pub fn run_plugin(&mut self, idx: usize) -> Result<UploadOutputCtx, UploadError> {
        let plugin = *self.plugins.get(idx).ok_or_else(|| {
            UploadError::PluginParamInvalid(format!("plugin index {idx} out of range"))
        })?;
        let plugin_input = UploadInputCtx {
            file: self.current_ctx.file.clone(),
            config_info: std::sync::Arc::new(plugin.registry_config.clone()),
            extra_info: self.current_ctx.extra_info.clone(),
            work_dir: self.current_ctx.work_dir.clone(),
        };
        self.emit_plugin_event(PipelineEventKind::PluginStart, plugin, &plugin_input, None);

        let output = match plugin.execute(&plugin_input) {
            Ok(o) => o,
            Err(e) => {
                let fail_ctx = UploadOutputCtx {
                    result: OutputResultType::Failed,
                    message: e.to_string(),
                    file: None,
                    extra_info: None,
                };
                self.emit_plugin_event(
                    PipelineEventKind::PluginEnd,
                    plugin,
                    &plugin_input,
                    Some(&fail_ctx),
                );
                return Err(e);
            }
        };
        self.emit_plugin_event(
            PipelineEventKind::PluginEnd,
            plugin,
            &plugin_input,
            Some(&output),
        );
        self.current_ctx =
            UploadPluginRegistryTable::output_to_input(&output, &self.current_ctx);
        Ok(output)
    }

    /// 在自定义编排位置发事件（plugin 可选）。
    pub fn emit_event(&self, kind: PipelineEventKind, plugin: Option<&PluginRegistryInfo>) {
        if let Some(plugin) = plugin {
            let ctx_view = UploadInputCtx {
                file: self.current_ctx.file.clone(),
                config_info: self.current_ctx.config_info.clone(),
                extra_info: self.current_ctx.extra_info.clone(),
                work_dir: self.current_ctx.work_dir.clone(),
            };
            self.emit_plugin_event(kind, plugin, &ctx_view, None);
        }
    }

    fn emit_plugin_event(
        &self,
        kind: PipelineEventKind,
        plugin: &PluginRegistryInfo,
        ctx: &UploadInputCtx,
        result: Option<&UploadOutputCtx>,
    ) {
        if let Some(cb) = self.callback {
            let event = PipelineEvent {
                timestamp_ms: chrono::Local::now().timestamp_millis(),
                kind,
                phase: self.phase.clone(),
                plugin_id: Some(&plugin.plugin_instance.id),
                plugin_meta: Some(plugin.plugin_instance.meta.as_ref()),
            };
            cb.on_event(&event, ctx, result);
        }
    }

    /// 归还内部 ctx（阶段结束时由 framework 取回推进后的输入）。
    pub(crate) fn into_inner(self) -> UploadInputCtx {
        self.current_ctx
    }
}

/// 默认串行执行器：与历史 execute_pipeline 插件循环行为一致（顺序执行 + Failed 短路）。
pub struct DefaultStageExecutor;

impl StageExecute for DefaultStageExecutor {
    fn execute(&self, ctx: &mut StageExecutionContext) -> Result<UploadOutputCtx, UploadError> {
        let mut last: Option<UploadOutputCtx> = None;
        for i in 0..ctx.plugins().len() {
            let output = ctx.run_plugin(i)?;
            if matches!(output.result, OutputResultType::Failed) {
                return Ok(output);
            }
            last = Some(output);
        }
        last.ok_or_else(|| UploadError::PluginParamInvalid("stage has no plugins to execute".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use file_uploader_sdk::models::ctx::{UploadFileData, UploadInputCtx};
    use file_uploader_sdk::models::enums::FileDataType;
    use std::sync::Arc;

    fn ctx_with_marker(marker: &str) -> UploadInputCtx {
        UploadInputCtx {
            file: Some(Arc::new(UploadFileData::new(
                FileDataType::FilePath,
                "/tmp/x".into(),
                "id".into(),
                "x".into(),
                String::new(),
                1,
            ))),
            config_info: Arc::new(None),
            extra_info: Some(
                [("/marker".to_string(), marker.to_string())]
                    .into_iter()
                    .collect(),
            ),
            work_dir: None,
        }
    }

    #[test]
    fn reset_input_replaces_current_ctx() {
        let initial = ctx_with_marker("base");
        let mut sc = StageExecutionContext::new(
            UploadPhase::Upload,
            vec![],
            None,
            ctx_with_marker("other"),
        );
        assert_eq!(
            sc.current_input().extra_info.as_ref().unwrap().get("/marker"),
            Some(&"other".to_string())
        );
        sc.reset_input(initial);
        assert_eq!(
            sc.current_input().extra_info.as_ref().unwrap().get("/marker"),
            Some(&"base".to_string())
        );
    }
}
