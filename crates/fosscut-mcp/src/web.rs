//! Web compositions and Remotion renders through `media_import` (ADR-0039): no tool of their
//! own, so the tool count stays small (ADR-0002, item 5).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fosscut_core::{ApiError, CommandCall, Request};
use fosscut_engine::EngineError;
use fosscut_engine::project;
use fosscut_engine::remotion::{self, RemotionRequest};
use fosscut_engine::web::CompositionOptions;
use serde_json::{Value, json};

use super::State;
use crate::params::RemotionArgs;

/// The `media.add` arguments for `file`: a web composition sized by `composition` when it is
/// given, otherwise whatever the file is, probed.
pub(crate) fn add_args(
    file: &Path,
    bundle: &Path,
    composition: Option<&CompositionOptions>,
) -> Result<Value, ApiError> {
    match composition {
        Some(options) => fosscut_engine::web::media_add_args(file, None, bundle, options),
        None => fosscut_engine::import::media_add_args(file, None, bundle),
    }
}

/// Starts rendering `args.composition` of the Remotion project `files[0]` into `bundle` as a
/// background job that imports the file when it is done, after the user confirmed (the
/// error without `confirm` carries the licence note to tell them). A dry run returns the
/// command and output without running anything. `template` is the write request whose
/// actor and reason the import uses.
pub(crate) fn remotion_job(
    state: &Arc<State>,
    bundle: PathBuf,
    files: &[String],
    args: RemotionArgs,
    dry_run: bool,
    template: Request,
) -> Result<Value, ApiError> {
    let [folder] = files else {
        return Err(ApiError::new(
            fosscut_core::codes::INVALID_ARGUMENTS,
            "media_import: remotion takes exactly one file, the Remotion project's folder",
        ));
    };
    let request = RemotionRequest {
        project: PathBuf::from(folder),
        composition: args.composition,
        props: args.props,
        entry: args.entry,
        bundle: bundle.clone(),
        confirmed: args.confirm,
        npx: None,
    };
    let plan = remotion::plan(&request)?;
    if dry_run {
        return Ok(json!({ "dry_run": true, "output": plan.output, "command": plan.command }));
    }
    let importer = Arc::clone(state);
    let import: fosscut_engine::jobs::RemotionImport = Box::new(move |file: &Path| {
        let failed = |e: ApiError| EngineError::Failed {
            code: e.code,
            message: e.message,
        };
        let args = fosscut_engine::import::media_add_args(file, None, &bundle).map_err(failed)?;
        let mut request = template;
        request.commands = vec![CommandCall {
            command: "media.add".into(),
            args,
        }];
        request.at_ms = project::now_ms();
        let response = importer
            .projects
            .execute(&bundle, request)
            .map_err(failed)?;
        response
            .results
            .first()
            .and_then(|r| r["asset"].as_str())
            .map(str::to_string)
            .ok_or_else(|| EngineError::Failed {
                code: "fosscut.internal".into(),
                message: "media.add answered without an asset".into(),
            })
    });
    let status = state.jobs.start_remotion(plan, Some(import))?;
    Ok(serde_json::to_value(status).unwrap_or_default())
}
