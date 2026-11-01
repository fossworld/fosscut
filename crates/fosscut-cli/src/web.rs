//! `fosscut web ...`: web compositions and Remotion renders (ADR-0039).

use std::path::PathBuf;

use clap::Subcommand;
use fosscut_core::{ApiError, Request, codes};
use fosscut_engine::remotion::{self, RemotionRequest};
use fosscut_engine::web::CompositionOptions;
use fosscut_time::TimeInput;
use serde_json::{Value, json};

use crate::args::Cli;
use crate::{Outcome, apply_options, bundle, engine_error, progress_printer, read_input, write};

fn parse_time(text: &str) -> Result<TimeInput, String> {
    text.parse()
        .map_err(|e: fosscut_time::TimeError| e.to_string())
}

/// `fosscut web ...`
#[derive(Debug, Subcommand)]
pub enum WebCommand {
    /// Import a web composition (a folder with an index.html, or one .html file) as a web
    /// asset: copied into the bundle and rendered frame by frame in a headless browser.
    /// Values not given come from the page's
    /// <meta name="fosscut:composition" content="width=1080, height=1920, fps=30, duration=6">.
    Import {
        /// The folder or .html file.
        source: PathBuf,
        /// Canvas width in pixels.
        #[arg(long)]
        width: Option<u32>,
        /// Canvas height in pixels.
        #[arg(long)]
        height: Option<u32>,
        /// Frames per second, such as 30 or 30000/1001; 30 when neither given nor in the
        /// page.
        #[arg(long)]
        fps: Option<String>,
        /// Length, such as 6s or 180f.
        #[arg(long, value_parser = parse_time)]
        duration: Option<TimeInput>,
        /// Display name; the folder or file name when omitted.
        #[arg(long)]
        name: Option<String>,
    },
    /// Render a composition of your own Remotion project with the Remotion installed in it
    /// (FossCut never installs Remotion) into the bundle's renders folder as ProRes 4444
    /// with alpha, then import it. Remotion's licence needs a paid company licence above a
    /// small company size; --yes confirms you may use it.
    Remotion {
        /// The Remotion project folder (its package.json depends on remotion).
        #[arg(value_name = "PROJECT")]
        remotion_project: PathBuf,
        /// The composition id.
        composition: String,
        /// Input props as JSON (inline, @file or -).
        #[arg(long)]
        props: Option<String>,
        /// The entry point relative to the project, such as src/index.ts; Remotion's usual
        /// places when omitted.
        #[arg(long)]
        entry: Option<String>,
        /// Confirms running Remotion under its licence.
        #[arg(long)]
        yes: bool,
    },
    /// Show the browser that renders web compositions, if one is found.
    Browser,
}

/// Runs a `fosscut web` command.
pub fn web(cli: &Cli, command: &WebCommand) -> Result<Outcome, ApiError> {
    match command {
        WebCommand::Import {
            source,
            width,
            height,
            fps,
            duration,
            name,
        } => {
            let bundle = bundle::bundle_path(cli.project.as_deref())?;
            let options = CompositionOptions {
                width: *width,
                height: *height,
                fps: fps.clone(),
                duration: duration.clone(),
            };
            let args =
                fosscut_engine::web::media_add_args(source, name.as_deref(), &bundle, &options)?;
            write(cli, "media.add", args)
        }
        WebCommand::Remotion {
            remotion_project,
            composition,
            props,
            entry,
            yes,
        } => remotion_render(cli, remotion_project, composition, props, entry, *yes),
        WebCommand::Browser => Ok(Outcome::Query(
            "web.browser".into(),
            fosscut_engine::web::status(),
        )),
    }
}

fn remotion_render(
    cli: &Cli,
    project: &std::path::Path,
    composition: &str,
    props: &Option<String>,
    entry: &Option<String>,
    yes: bool,
) -> Result<Outcome, ApiError> {
    let bundle = bundle::bundle_path(cli.project.as_deref())?;
    let props = props
        .as_deref()
        .map(|text| {
            let text = read_input(text)?;
            serde_json::from_str::<Value>(&text).map_err(|e| {
                ApiError::new(
                    codes::INVALID_ARGUMENTS,
                    format!("--props is not JSON: {e}"),
                )
            })
        })
        .transpose()?;
    let request = RemotionRequest {
        project: project.to_path_buf(),
        composition: composition.to_string(),
        props,
        entry: entry.clone(),
        bundle: bundle.clone(),
        confirmed: yes,
        npx: None,
    };
    let plan = remotion::plan(&request).map_err(|e| engine_error(&e))?;
    if cli.write.dry_run {
        return Ok(Outcome::Query(
            "web.remotion".into(),
            json!({ "dry_run": true, "output": plan.output, "command": plan.command }),
        ));
    }
    eprintln!(
        "rendering {composition} with the Remotion installed in {}",
        project.display()
    );
    let stop = std::sync::atomic::AtomicBool::new(false);
    let report = remotion::render(&plan, &stop, &mut progress_printer("rendering"))
        .map_err(|e| engine_error(&e))?;
    let mut add = Request::single(
        "media.add",
        fosscut_engine::import::media_add_args(&PathBuf::from(&report.output), None, &bundle)?,
    );
    apply_options(cli, &mut add, true);
    let imported = bundle::execute(&bundle, add)?;
    let mut report = report;
    report.asset = imported
        .results
        .first()
        .and_then(|r| r["asset"].as_str().map(str::to_string));
    Ok(Outcome::Query(
        "web.remotion".into(),
        json!({ "remotion": report, "import": imported }),
    ))
}
