//! Running the user's own agent on a project (ADR-0043): the reasoning stays with the
//! user's agent (ADR-0012), and FossCut only starts the command line the user installed,
//! headless, after the user agreed to the exact command.
//!
//! - **Finding** ([`find`]): the `agent.command` preference, then `FOSSCUT_AGENT`, then
//!   Claude Code's `claude` on the path and in its usual install places. FossCut never
//!   installs or bundles an agent, and never reads or stores keys: the agent uses its own
//!   login.
//! - **Planning** ([`plan`]): the brief ([`default_brief`] by default, editable), a working
//!   folder holding FossCut's skills and an MCP configuration that names only FossCut's MCP
//!   server for the project, and the command. Without `confirmed` it refuses with
//!   `fosscut.agent.confirmation_required`, whose details carry the command, the agent and
//!   the notes the user must see first.
//! - **Running** ([`run`]): the agent runs in its own process group through
//!   [`crate::external`]; its `stream-json` output is read into an [`AgentReport`] (turns,
//!   tool calls, the last message, the cost when reported) and the final summary.
//!   Cancelling ends the whole group. The agent's edits reach the project through FossCut's
//!   MCP server, so they land in the live session as `agent:<client>` and undo like any
//!   other.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use fosscut_core::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::EngineError;
use crate::external::{self, RunError, Stream};

/// The environment variable naming the agent to run: a path, or a name looked up like
/// `claude`.
pub const AGENT_VARIABLE: &str = "FOSSCUT_AGENT";

/// The skill the default brief follows.
pub const REEL_SKILL: &str = "talking-head-reel";

/// The MCP server's name in the agent's configuration, which prefixes its tools.
pub const SERVER_NAME: &str = "fosscut";

/// Turns the agent may take when the request names no other cap.
pub const DEFAULT_MAX_TURNS: u32 = 200;

/// The most turns a request may allow.
pub const MAX_TURNS_LIMIT: u32 = 1000;

/// The longest brief, in characters.
pub const MAX_BRIEF: usize = 20_000;

/// The tools the agent may use without asking: FossCut's own, and loading skills.
pub const ALLOWED_TOOLS: &str = "mcp__fosscut__*,Skill";

/// The agent's own tools that run commands, change files or reach the web, denied for the
/// run even when the user's settings allow them.
pub const DENIED_TOOLS: &str = "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch";

/// What the user is told about their account before the agent runs.
pub const ACCOUNT_NOTE: &str = "The agent is your own installed program and runs with your own account and login; FossCut never sees or stores your keys. Its use of the AI model may cost money on your plan or bill.";

/// What the user is told about what the agent may do.
pub const TOOLS_NOTE: &str = "It connects only to FossCut's tools for this project: other MCP servers are left out, and running commands, changing files and reaching the web are denied. Every change it makes appears live and can be undone.";

/// How long a kept log line or a message in the report may be, in characters.
const LINE: usize = 400;

/// An agent found on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundAgent {
    /// The program.
    pub path: PathBuf,
    /// Its name for people: `Claude Code`.
    pub name: String,
    /// How it was found: `preference`, `FOSSCUT_AGENT` or `path` (the path or a usual
    /// install place).
    pub via: String,
}

/// The program names Claude Code installs as.
fn program_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    }
}

/// Where agents are usually installed, beyond the path: apps started from the dock see a
/// short `PATH`. `home` is the user's home folder.
pub fn places(home: Option<&Path>) -> Vec<PathBuf> {
    let mut places = Vec::new();
    if let Some(home) = home {
        places.push(home.join(".local").join("bin"));
        places.push(home.join(".claude").join("local"));
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            places.push(PathBuf::from(appdata).join("npm"));
        }
    } else {
        places.push(PathBuf::from("/opt/homebrew/bin"));
        places.push(PathBuf::from("/usr/local/bin"));
    }
    places
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Finds the agent to run: `configured` (the `agent.command` preference) when given, else
/// [`AGENT_VARIABLE`], else Claude Code's `claude` on the path or in [`places`]. A
/// configured program that does not exist is not replaced by another.
pub fn find(configured: Option<&Path>) -> Result<FoundAgent, AgentError> {
    locate(
        configured,
        external::overridden(AGENT_VARIABLE).as_deref(),
        &places(home().as_deref()),
    )
}

fn locate(
    configured: Option<&Path>,
    variable: Option<&Path>,
    places: &[PathBuf],
) -> Result<FoundAgent, AgentError> {
    let found = |path: PathBuf, via: &str| FoundAgent {
        path,
        name: "Claude Code".into(),
        via: via.into(),
    };
    let named = [(configured, "preference"), (variable, AGENT_VARIABLE)]
        .into_iter()
        .find_map(|(program, via)| program.map(|p| (p, via)));
    if let Some((program, via)) = named {
        return external::find_program(program, places)
            .map(|path| found(path, via))
            .ok_or_else(|| AgentError::NotFound {
                tried: vec![format!("{} ({via})", program.display())],
            });
    }
    for name in program_names() {
        if let Some(path) = external::find_program(Path::new(name), places) {
            return Ok(found(path, "path"));
        }
    }
    let mut tried = vec![format!("{} on the path", program_names().join(", "))];
    tried.extend(places.iter().map(|p| p.display().to_string()));
    Err(AgentError::NotFound { tried })
}

/// What the default brief names.
#[derive(Clone, Debug, PartialEq)]
pub struct BriefContext {
    /// The project's name.
    pub project: String,
    /// The project's bundle.
    pub bundle: PathBuf,
    /// The canvas's width and height in pixels.
    pub width: u32,
    /// Canvas height.
    pub height: u32,
    /// The video's file name, when the project was made from one.
    pub video: Option<String>,
}

/// The brief FossCut suggests for turning a raw talking-head video into a reel with the
/// [`REEL_SKILL`] skill; the user may change it before the agent runs.
pub fn default_brief(context: &BriefContext) -> String {
    let video = context
        .video
        .as_deref()
        .map(|name| format!("the raw talking-head video {name}"))
        .unwrap_or_else(|| "the raw talking-head video".into());
    format!(
        "Edit the FossCut project \"{project}\" into a finished short-form reel. Follow FossCut's {skill} skill: load it with the Skill tool, or read .claude/skills/{skill}/SKILL.md in your working folder (it is also the MCP resource fosscut://skills/{skill}).\n\n\
The project is {bundle}. It is open in the FossCut app with {video} on its timeline, on a {width}x{height} canvas. Work only through the fosscut MCP tools: every change appears live in the app, where the user can undo it.\n\n\
- Clean up the speech: cut silences, filler words and retakes.\n\
- Give every line a fitting visual from FossCut's library, with sound effects on entrances and punch-ins on emphasis.\n\
- Add bold word-by-word captions in the spoken language, with the key words highlighted.\n\
- Lay a music bed under the voice, ducked under speech and levelled.\n\
- Check frames with frame_grab or contact_sheet as you go.\n\n\
Do not export: the user reviews and exports in the app. End with a short summary of what you changed.",
        project = context.project,
        skill = REEL_SKILL,
        bundle = context.bundle.display(),
        width = context.width,
        height = context.height,
    )
}

/// How the agent starts FossCut's MCP server: a program, its arguments and environment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    /// The program: the desktop app's own executable in `mcp` mode, or `fosscut`.
    pub command: PathBuf,
    /// Its arguments, such as `mcp -p <bundle>`.
    pub args: Vec<String>,
    /// Variables it needs, which the agent may not pass on by itself.
    pub env: BTreeMap<String, String>,
}

impl McpServer {
    /// The line that adds this server to Claude Code by hand, for users who run their agent
    /// themselves: `claude mcp add fosscut -- <command> mcp`.
    pub fn setup_line(&self) -> String {
        format!(
            "claude mcp add {SERVER_NAME} -- {} mcp",
            quoted(&self.command.display().to_string())
        )
    }
}

/// The variables of this process the MCP server is given: FossCut's own (but the agent
/// override), and where programs and libraries are found, which macOS drops from children
/// of system programs.
pub fn server_environment() -> BTreeMap<String, String> {
    std::env::vars()
        .filter(|(name, _)| {
            (name.starts_with("FOSSCUT_") && name != AGENT_VARIABLE)
                || [
                    "PATH",
                    "DYLD_LIBRARY_PATH",
                    "DYLD_FALLBACK_LIBRARY_PATH",
                    "LD_LIBRARY_PATH",
                ]
                .contains(&name.as_str())
        })
        .collect()
}

/// A user-facing skill to install in the agent's working folder: its name and files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillFiles {
    /// Folder and frontmatter name.
    pub name: String,
    /// Paths inside the skill folder with their contents.
    pub files: Vec<(String, String)>,
}

/// An agent run to plan.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRequest {
    /// The `.fosscut` bundle the agent edits.
    pub bundle: PathBuf,
    /// What the agent is asked to do.
    pub brief: String,
    /// The most turns it may take, 1 to [`MAX_TURNS_LIMIT`].
    pub max_turns: u32,
    /// The most it may spend in US dollars, when given.
    pub max_budget_usd: Option<f64>,
    /// The agent to run (the `agent.command` preference); found by [`find`] otherwise.
    pub agent: Option<PathBuf>,
    /// How the agent starts FossCut's MCP server.
    pub server: McpServer,
    /// Skills installed in the agent's working folder.
    pub skills: Vec<SkillFiles>,
    /// The folder that holds the agents' working folders, such as `agent` in the cache.
    pub folder: PathBuf,
    /// True once the user agreed to the command and the notes.
    pub confirmed: bool,
}

/// A checked run, ready to start.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentPlan {
    request: AgentRequest,
    /// The agent that runs.
    pub agent: FoundAgent,
    /// Its working folder, with the skills and the MCP configuration.
    pub folder: PathBuf,
    /// The MCP configuration.
    pub config: PathBuf,
    /// The command line, as shown to the user.
    pub command: Vec<String>,
    /// What the user is told before agreeing.
    pub notes: Vec<String>,
}

impl AgentPlan {
    /// The turn cap.
    pub fn max_turns(&self) -> u32 {
        self.request.max_turns
    }
}

/// What an agent run did so far, or in all once it ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    /// The program that runs.
    pub agent: String,
    /// The model it uses, once it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Its session id, with which the user can resume it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Whether it reached FossCut's MCP server, once it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<bool>,
    /// Turns taken: the model's answers.
    pub turns: u64,
    /// The turn cap.
    pub max_turns: u64,
    /// Tool calls made.
    pub tool_calls: u64,
    /// Tool calls that came back as errors.
    pub tool_errors: u64,
    /// The last tool called, without the `mcp__fosscut__` prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    /// The last thing the agent said, shortened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    /// What the run cost in US dollars, as the agent reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// How long the agent says it ran, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// How it ended, as the agent says: `success`, `error_max_turns`,
    /// `error_max_budget_usd` or `error_during_execution`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// The agent's final answer: its summary of what it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The command that ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
}

/// The agent could not be found, run, or finish.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum AgentError {
    /// No agent was found.
    #[error("no agent command line (Claude Code) was found on this computer")]
    NotFound {
        /// The places looked at.
        tried: Vec<String>,
    },
    /// Running the agent needs the user's confirmation first.
    #[error("running {agent} needs the user's confirmation: {}", notes.join(" "))]
    Confirmation {
        /// The agent that would run.
        agent: String,
        /// The command that would run.
        command: Vec<String>,
        /// What the user must be told.
        notes: Vec<String>,
    },
    /// The agent failed, or ended without an answer.
    #[error("the agent failed: {message}")]
    Failed {
        /// What went wrong.
        message: String,
        /// The last lines it printed, summarised.
        log: Vec<String>,
    },
    /// The agent stopped at its turn or budget cap.
    #[error("the agent stopped at its {limit} cap before it finished")]
    LimitReached {
        /// `turns` or `budget`.
        limit: String,
        /// Turns taken.
        turns: u64,
        /// The cost, when reported.
        cost_usd: Option<f64>,
    },
}

impl AgentError {
    /// The stable error code (ADR-0006).
    pub fn code(&self) -> &'static str {
        match self {
            AgentError::NotFound { .. } => "fosscut.agent.not_found",
            AgentError::Confirmation { .. } => "fosscut.agent.confirmation_required",
            AgentError::Failed { .. } => "fosscut.agent.failed",
            AgentError::LimitReached { .. } => "fosscut.agent.limit_reached",
        }
    }

    /// What to do about it.
    pub fn hint(&self) -> Option<String> {
        Some(match self {
            AgentError::NotFound { .. } => format!(
                "Install Claude Code yourself (FossCut never installs an agent), or name its command in Settings or with {AGENT_VARIABLE}; or copy the brief and run your own agent with FossCut's MCP server."
            ),
            AgentError::Confirmation { .. } => "Show the user the command and the notes in details; start again with confirmed only if they agree.".into(),
            AgentError::Failed { .. } => "Read details.log. Check that the agent is logged in (run it once in a terminal) and can start FossCut's MCP server.".into(),
            AgentError::LimitReached { .. } => "Its edits so far are in the project and can be undone. Run it again with a higher cap, or finish by hand.".into(),
        })
    }

    /// Machine-readable details.
    pub fn details(&self) -> Value {
        match self {
            AgentError::NotFound { tried } => json!({ "tried": tried }),
            AgentError::Confirmation {
                agent,
                command,
                notes,
            } => json!({ "agent": agent, "command": command, "notes": notes }),
            AgentError::Failed { log, .. } => json!({ "log": log }),
            AgentError::LimitReached {
                limit,
                turns,
                cost_usd,
            } => json!({ "limit": limit, "turns": turns, "cost_usd": cost_usd }),
        }
    }
}

impl From<AgentError> for EngineError {
    fn from(error: AgentError) -> EngineError {
        EngineError::Agent(error)
    }
}

impl From<AgentError> for ApiError {
    fn from(error: AgentError) -> ApiError {
        ApiError::from(EngineError::Agent(error))
    }
}

/// Every `fosscut.agent.*` error code with what it means and what to do.
pub fn error_catalogue() -> Vec<ErrorCode> {
    let entry = |code, meaning, hint| ErrorCode {
        code,
        meaning,
        hint,
    };
    vec![
        entry(
            "fosscut.agent.not_found",
            "No agent command line (Claude Code) was found to run on the project.",
            "Install one yourself or name it with FOSSCUT_AGENT; FossCut never installs an agent.",
        ),
        entry(
            "fosscut.agent.confirmation_required",
            "Running the user's agent needs their agreement to the command, their account and the possible cost.",
            "Show details.command and details.notes; start again with confirmed only if the user agrees.",
        ),
        entry(
            "fosscut.agent.failed",
            "The agent could not start, failed, or ended without an answer.",
            "Read details.log; check that the agent is logged in and can start FossCut's MCP server.",
        ),
        entry(
            "fosscut.agent.limit_reached",
            "The agent stopped at its turn or budget cap; its edits so far stay and can be undone.",
            "Run it again with a higher cap, or finish by hand.",
        ),
    ]
}

/// `text` quoted for a POSIX shell when it needs it.
fn quoted(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// The working folder of runs on `bundle`, inside `folder`: the same for every run on that
/// project, so the command a user agreed to is the command that runs.
fn working_folder(folder: &Path, bundle: &Path) -> PathBuf {
    let hash = blake3::hash(bundle.display().to_string().as_bytes()).to_hex();
    folder.join(&hash.as_str()[..16])
}

/// Checks `request` before anything runs and builds the command: the brief, the caps, and an
/// agent found on this computer. Without confirmation it fails with
/// `fosscut.agent.confirmation_required`, which carries the command, the agent and the
/// notes the user must see.
pub fn plan(request: &AgentRequest) -> Result<AgentPlan, EngineError> {
    let invalid = |message: String| EngineError::InvalidArguments { message };
    let brief = request.brief.trim();
    if brief.is_empty() {
        return Err(invalid("write what the agent should do".into()));
    }
    if brief.chars().count() > MAX_BRIEF {
        return Err(invalid(format!(
            "the brief is longer than {MAX_BRIEF} characters"
        )));
    }
    if !(1..=MAX_TURNS_LIMIT).contains(&request.max_turns) {
        return Err(invalid(format!(
            "max_turns must be from 1 to {MAX_TURNS_LIMIT}, not {}",
            request.max_turns
        )));
    }
    if let Some(budget) = request.max_budget_usd
        && !(budget.is_finite() && budget > 0.0 && budget <= 1000.0)
    {
        return Err(invalid(format!(
            "max_budget_usd must be more than 0 and at most 1000, not {budget}"
        )));
    }
    let bundle = std::path::absolute(&request.bundle).unwrap_or_else(|_| request.bundle.clone());
    if !bundle.is_dir() {
        return Err(invalid(format!(
            "{} is not a FossCut project",
            bundle.display()
        )));
    }
    let agent = find(request.agent.as_deref())?;
    let folder = working_folder(&request.folder, &bundle);
    let config = folder.join("mcp.json");
    // The brief is the prompt right after -p; one starting with a dash would read as an
    // option.
    let prompt = if brief.starts_with('-') {
        format!("Brief: {brief}")
    } else {
        brief.to_string()
    };
    let mut command = vec![
        agent.path.display().to_string(),
        "-p".into(),
        prompt,
        "--mcp-config".into(),
        config.display().to_string(),
        "--strict-mcp-config".into(),
        "--allowedTools".into(),
        ALLOWED_TOOLS.into(),
        "--disallowedTools".into(),
        DENIED_TOOLS.into(),
        "--max-turns".into(),
        request.max_turns.to_string(),
    ];
    let mut caps = format!("It stops after {} turns", request.max_turns);
    if let Some(budget) = request.max_budget_usd {
        command.push("--max-budget-usd".into());
        command.push(format!("{budget:.2}"));
        caps.push_str(&format!(" or US${budget:.2} of model use"));
    }
    caps.push_str(", and you can cancel it at any time.");
    command.extend([
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ]);
    let notes = vec![ACCOUNT_NOTE.to_string(), TOOLS_NOTE.to_string(), caps];
    if !request.confirmed {
        return Err(AgentError::Confirmation {
            agent: agent.path.display().to_string(),
            command,
            notes,
        }
        .into());
    }
    let mut request = request.clone();
    request.bundle = bundle;
    Ok(AgentPlan {
        request,
        agent,
        folder,
        config,
        command,
        notes,
    })
}

/// The MCP configuration of `plan`: FossCut's server only.
fn configuration(server: &McpServer) -> Value {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "command": server.command.display().to_string(),
                "args": server.args,
                "env": server.env,
            }
        }
    })
}

/// Writes the working folder afresh: the MCP configuration and the skills under
/// `.claude/skills`, where the agent finds project skills.
fn prepare(plan: &AgentPlan) -> Result<(), AgentError> {
    let failed = |message: String| AgentError::Failed {
        message,
        log: Vec::new(),
    };
    let skills = plan.folder.join(".claude").join("skills");
    if skills.exists() {
        std::fs::remove_dir_all(&skills)
            .map_err(|e| failed(format!("cannot clear {}: {e}", skills.display())))?;
    }
    std::fs::create_dir_all(&skills)
        .map_err(|e| failed(format!("cannot create {}: {e}", skills.display())))?;
    for skill in &plan.request.skills {
        for (path, text) in &skill.files {
            let file = skills.join(&skill.name).join(path);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| failed(format!("cannot create {}: {e}", parent.display())))?;
            }
            std::fs::write(&file, text)
                .map_err(|e| failed(format!("cannot write {}: {e}", file.display())))?;
        }
    }
    let text = serde_json::to_string_pretty(&configuration(&plan.request.server))
        .map_err(|e| failed(e.to_string()))?;
    std::fs::write(&plan.config, text)
        .map_err(|e| failed(format!("cannot write {}: {e}", plan.config.display())))
}

/// `text` cut to `LINE` characters, on one line.
fn short(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= LINE {
        flat
    } else {
        let cut: String = flat.chars().take(LINE - 1).collect();
        format!("{cut}\u{2026}")
    }
}

/// What the agent's output said so far.
#[derive(Default)]
struct Events {
    report: AgentReport,
    /// Ids of the model's answers counted as turns.
    answers: Vec<String>,
    /// The final result event, once it came.
    result: Option<Value>,
}

impl Events {
    /// Reads one line of the agent's standard output; answers what the log keeps of it.
    fn line(&mut self, line: &str) -> Option<String> {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return Some(short(line));
        };
        let report = &mut self.report;
        match event["type"].as_str() {
            Some("system") if event["subtype"] == "init" => {
                report.model = event["model"].as_str().map(str::to_string);
                report.session = event["session_id"].as_str().map(str::to_string);
                let status = event["mcp_servers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|server| server["name"] == SERVER_NAME)
                    .and_then(|server| server["status"].as_str());
                report.connected = Some(status == Some("connected"));
                Some(format!(
                    "started {}; FossCut's tools {}",
                    report.model.as_deref().unwrap_or("the agent"),
                    status.unwrap_or("missing")
                ))
            }
            Some("assistant") => {
                let message = &event["message"];
                match message["id"].as_str() {
                    Some(id) if self.answers.iter().any(|seen| seen == id) => {}
                    Some(id) => {
                        self.answers.push(id.to_string());
                        report.turns += 1;
                    }
                    None => report.turns += 1,
                }
                let mut kept = Vec::new();
                for block in message["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => {
                            let text = block["text"].as_str().unwrap_or_default().trim();
                            if !text.is_empty() {
                                report.last_message = Some(short(text));
                                kept.push(format!("agent: {}", short(text)));
                            }
                        }
                        Some("tool_use") => {
                            let name = block["name"].as_str().unwrap_or("tool");
                            let name = name
                                .strip_prefix(&format!("mcp__{SERVER_NAME}__"))
                                .unwrap_or(name);
                            report.tool_calls += 1;
                            report.last_tool = Some(name.to_string());
                            kept.push(format!("tool: {name}"));
                        }
                        _ => {}
                    }
                }
                (!kept.is_empty()).then(|| kept.join(" | "))
            }
            Some("user") => {
                let mut kept = Vec::new();
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result" && block["is_error"] == true {
                        report.tool_errors += 1;
                        let text = match &block["content"] {
                            Value::String(text) => text.clone(),
                            Value::Array(parts) => parts
                                .iter()
                                .filter_map(|p| p["text"].as_str())
                                .collect::<Vec<_>>()
                                .join(" "),
                            other => other.to_string(),
                        };
                        kept.push(format!("tool error: {}", short(&text)));
                    }
                }
                (!kept.is_empty()).then(|| kept.join(" | "))
            }
            Some("result") => {
                if let Some(turns) = event["num_turns"].as_u64() {
                    report.turns = turns;
                }
                report.cost_usd = event["total_cost_usd"].as_f64().or(report.cost_usd);
                report.duration_ms = event["duration_ms"].as_u64();
                report.outcome = event["subtype"].as_str().map(str::to_string);
                report.summary = event["result"]
                    .as_str()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty());
                if report.session.is_none() {
                    report.session = event["session_id"].as_str().map(str::to_string);
                }
                let kept = format!("result: {}", report.outcome.as_deref().unwrap_or("unknown"));
                self.result = Some(event);
                Some(kept)
            }
            _ => None,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs a planned agent in its working folder until it ends or is cancelled, reporting
/// what it does as it goes; it stops, with everything it started, when `cancel` is set or
/// `progress` answers `false`. Its edits are already in the project when it ends, whatever
/// the outcome: a run that ends with an answer succeeds with the report; one that stops at a
/// cap fails with `fosscut.agent.limit_reached`, and any other ending with
/// `fosscut.agent.failed` and the last lines it printed.
pub fn run(
    plan: &AgentPlan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&AgentReport) -> bool,
) -> Result<AgentReport, EngineError> {
    prepare(plan)?;
    let state = Arc::new(Mutex::new(Events {
        report: AgentReport {
            agent: plan.agent.path.display().to_string(),
            max_turns: u64::from(plan.max_turns()),
            command: plan.command.clone(),
            ..AgentReport::default()
        },
        ..Events::default()
    }));
    let reader = Arc::clone(&state);
    let lines: external::Lines = Box::new(move |which, line| match which {
        Stream::Out => lock(&reader).line(line),
        Stream::Err => Some(short(line)),
    });
    let mut command = external::command(&plan.agent.path, &plan.command[1..], "fosscut-agent");
    command
        .current_dir(&plan.folder)
        // The run is a session of its own, even when FossCut was started from one.
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT");
    let mut tick = || {
        let report = lock(&state).report.clone();
        progress(&report)
    };
    let finished = external::run(command, cancel, lines, &mut tick);
    let (report, result) = {
        let state = lock(&state);
        (state.report.clone(), state.result.clone())
    };
    let finished = match finished {
        Ok(finished) => finished,
        Err(RunError::Cancelled) => {
            return Err(EngineError::JobCancelled {
                kind: "agent".into(),
            });
        }
        Err(RunError::Spawn(e)) => {
            return Err(AgentError::Failed {
                message: format!("cannot run {}: {e}", plan.agent.path.display()),
                log: Vec::new(),
            }
            .into());
        }
        Err(RunError::Lost(e)) => {
            return Err(AgentError::Failed {
                message: format!("lost the agent: {e}"),
                log: Vec::new(),
            }
            .into());
        }
    };
    let Some(result) = result else {
        let message = if finished.status.success() {
            "the agent ended without an answer".to_string()
        } else {
            format!("the agent exited with {}", finished.status)
        };
        return Err(AgentError::Failed {
            message,
            log: finished.log,
        }
        .into());
    };
    let limit = match report.outcome.as_deref() {
        Some("error_max_turns") => Some("turns"),
        Some("error_max_budget_usd") => Some("budget"),
        _ => None,
    };
    if let Some(limit) = limit {
        return Err(AgentError::LimitReached {
            limit: limit.into(),
            turns: report.turns,
            cost_usd: report.cost_usd,
        }
        .into());
    }
    if result["is_error"] == true || report.outcome.as_deref() != Some("success") {
        let message = report.summary.clone().unwrap_or_else(|| {
            format!(
                "it ended with {}",
                report.outcome.as_deref().unwrap_or("an error")
            )
        });
        return Err(AgentError::Failed {
            message,
            log: finished.log,
        }
        .into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_stream_becomes_a_report() {
        let mut stream = Events::default();
        let lines = [
            r#"{"type":"system","subtype":"init","model":"claude-test","session_id":"s1","mcp_servers":[{"name":"fosscut","status":"connected"}]}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"I will read the timeline."}]}}"#,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","name":"mcp__fosscut__timeline_read","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"fosscut.revision.conflict"}]}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","name":"mcp__fosscut__edit","input":{}}]}}"#,
            "not json",
            r#"{"type":"result","subtype":"success","is_error":false,"num_turns":5,"duration_ms":1200,"total_cost_usd":0.42,"result":"Cut 3 pauses."}"#,
        ];
        let kept: Vec<Option<String>> = lines.iter().map(|l| stream.line(l)).collect();
        let report = &stream.report;
        assert_eq!(report.model.as_deref(), Some("claude-test"));
        assert_eq!(report.session.as_deref(), Some("s1"));
        assert_eq!(report.connected, Some(true));
        assert_eq!(report.tool_calls, 2);
        assert_eq!(report.tool_errors, 1);
        assert_eq!(report.last_tool.as_deref(), Some("edit"));
        assert_eq!(
            report.last_message.as_deref(),
            Some("I will read the timeline.")
        );
        assert_eq!(report.turns, 5, "the result's count wins");
        assert_eq!(report.cost_usd, Some(0.42));
        assert_eq!(report.outcome.as_deref(), Some("success"));
        assert_eq!(report.summary.as_deref(), Some("Cut 3 pauses."));
        assert_eq!(
            kept[0].as_deref(),
            Some("started claude-test; FossCut's tools connected")
        );
        assert_eq!(kept[2].as_deref(), Some("tool: timeline_read"));
        assert_eq!(
            kept[3].as_deref(),
            Some("tool error: fosscut.revision.conflict")
        );
        assert_eq!(kept[5].as_deref(), Some("not json"));
        assert!(stream.result.is_some());

        // Turns are counted by answer before the result says how many.
        let mut counting = Events::default();
        counting.line(lines[1]);
        counting.line(lines[2]);
        counting.line(lines[4]);
        assert_eq!(counting.report.turns, 2);
        let mut failed = Events::default();
        failed.line(r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"fosscut","status":"failed"}]}"#);
        assert_eq!(failed.report.connected, Some(false));
    }

    #[test]
    fn long_lines_are_shortened_on_one_line() {
        assert_eq!(short("a\n  b"), "a b");
        let long = "x".repeat(1000);
        assert_eq!(short(&long).chars().count(), LINE);
        assert!(short(&long).ends_with('\u{2026}'));
    }

    #[test]
    fn agents_are_found_where_configured_or_in_their_usual_places() {
        let dir = std::env::temp_dir().join(format!("fosscut-agent-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let claude = dir.join("bin").join(program_names()[0]);
        std::fs::write(&claude, "").unwrap();
        let places = [dir.join("bin")];

        let configured = locate(Some(&claude), None, &[]).unwrap();
        assert_eq!(
            (configured.path.as_path(), configured.via.as_str()),
            (claude.as_path(), "preference")
        );
        let variable = locate(None, Some(&claude), &[]).unwrap();
        assert_eq!(variable.via, AGENT_VARIABLE);
        // A configured program that is missing is not swapped for another.
        let missing = locate(Some(&dir.join("nope")), None, &places).unwrap_err();
        assert_eq!(missing.code(), "fosscut.agent.not_found");
        assert!(
            missing.details()["tried"][0]
                .as_str()
                .unwrap()
                .contains("nope")
        );
        if !std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .any(|p| p.join(program_names()[0]).is_file())
        {
            let usual = locate(None, None, &places).unwrap();
            assert_eq!((usual.path, usual.via.as_str()), (claude.clone(), "path"));
            let none = locate(None, None, &[dir.join("empty")]).unwrap_err();
            assert_eq!(none.code(), "fosscut.agent.not_found");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usual_places_include_the_home_folder_and_package_managers() {
        let places = places(Some(Path::new("/Users/me")));
        assert!(places.contains(&PathBuf::from("/Users/me/.local/bin")));
        assert!(places.contains(&PathBuf::from("/Users/me/.claude/local")));
        if cfg!(unix) {
            assert!(places.contains(&PathBuf::from("/opt/homebrew/bin")));
        }
    }

    #[test]
    fn the_default_brief_names_the_skill_the_project_and_the_video() {
        let brief = default_brief(&BriefContext {
            project: "Launch".into(),
            bundle: PathBuf::from("/work/Launch.fosscut"),
            width: 1080,
            height: 1920,
            video: Some("raw.mp4".into()),
        });
        assert!(brief.contains("talking-head-reel"));
        assert!(brief.contains("/work/Launch.fosscut"));
        assert!(brief.contains("raw.mp4"));
        assert!(brief.contains("1080x1920"));
        assert!(brief.contains("Do not export"));
        assert!(!brief.contains('\u{2014}'), "no em dashes");
    }

    #[test]
    fn setup_lines_quote_paths_with_spaces() {
        let server = McpServer {
            command: PathBuf::from("/Applications/FossCut.app/Contents/MacOS/fosscut-desktop"),
            ..McpServer::default()
        };
        assert_eq!(
            server.setup_line(),
            "claude mcp add fosscut -- /Applications/FossCut.app/Contents/MacOS/fosscut-desktop mcp"
        );
        let spaced = McpServer {
            command: PathBuf::from("/Users/me/My Apps/fosscut"),
            ..McpServer::default()
        };
        assert_eq!(
            spaced.setup_line(),
            "claude mcp add fosscut -- '/Users/me/My Apps/fosscut' mcp"
        );
    }

    #[test]
    fn every_code_is_catalogued_with_a_hint() {
        let catalogue: Vec<&str> = error_catalogue().iter().map(|e| e.code).collect();
        for error in [
            AgentError::NotFound { tried: Vec::new() },
            AgentError::Confirmation {
                agent: String::new(),
                command: Vec::new(),
                notes: Vec::new(),
            },
            AgentError::Failed {
                message: String::new(),
                log: Vec::new(),
            },
            AgentError::LimitReached {
                limit: "turns".into(),
                turns: 0,
                cost_usd: None,
            },
        ] {
            assert!(catalogue.contains(&error.code()), "{}", error.code());
            assert!(error.hint().is_some());
            let engine = EngineError::from(error.clone());
            assert_eq!(engine.code(), error.code());
            assert_eq!(engine.details(), error.details());
        }
    }
}
