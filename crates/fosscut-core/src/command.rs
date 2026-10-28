use std::collections::BTreeMap;

use fosscut_model::Project;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{ApiError, codes, commands, queries};

/// A typed editing command (ADR-0002).
///
/// Implementations validate their input and change `ctx.project`. Revisions, dry runs,
/// idempotency, atomic batches, undo and logging are provided by [`crate::Session`],
/// so commands describe only their effect.
pub trait Command: DeserializeOwned + JsonSchema {
    /// Command name, `<area>.<verb>`.
    const NAME: &'static str;
    /// One-sentence description for agents and documentation.
    const SUMMARY: &'static str;
    /// Result returned to the caller.
    type Output: Serialize + JsonSchema;

    /// Applies the command to the working copy of the project.
    fn apply(self, ctx: &mut Context<'_>) -> Result<Self::Output, ApiError>;
}

/// What a command can see and change.
pub struct Context<'a> {
    /// The working copy; changes are committed only if every command in the request succeeds.
    pub project: &'a mut Project,
    allow_destructive: bool,
}

impl<'a> Context<'a> {
    pub(crate) fn new(project: &'a mut Project, allow_destructive: bool) -> Context<'a> {
        Context {
            project,
            allow_destructive,
        }
    }

    /// Fails with `fosscut.guard.destructive` unless the request allows destructive edits.
    pub fn require_destructive(
        &self,
        what: &str,
        details: serde_json::Value,
    ) -> Result<(), ApiError> {
        if self.allow_destructive {
            Ok(())
        } else {
            Err(ApiError::new(
                codes::GUARD_DESTRUCTIVE,
                format!("{what} is destructive and needs allow_destructive"),
            )
            .with_details(details))
        }
    }
}

type Exec = fn(&mut Context<'_>, serde_json::Value) -> Result<serde_json::Value, ApiError>;
type QueryFn =
    fn(&queries::QueryContext<'_>, serde_json::Value) -> Result<serde_json::Value, ApiError>;

/// A registered command with its schemas.
#[derive(Clone)]
pub struct CommandDescriptor {
    /// Command name.
    pub name: &'static str,
    /// One-sentence description.
    pub summary: &'static str,
    /// JSON Schema of the arguments.
    pub input_schema: schemars::Schema,
    /// JSON Schema of the result.
    pub output_schema: schemars::Schema,
    pub(crate) exec: Option<Exec>,
}

/// A registered query with its schemas.
#[derive(Clone)]
pub struct QueryDescriptor {
    /// Query name.
    pub name: &'static str,
    /// One-sentence description.
    pub summary: &'static str,
    /// JSON Schema of the arguments.
    pub input_schema: schemars::Schema,
    pub(crate) run: QueryFn,
}

/// All commands and queries known to a session.
#[derive(Clone)]
pub struct Registry {
    commands: BTreeMap<&'static str, CommandDescriptor>,
    queries: BTreeMap<&'static str, QueryDescriptor>,
}

fn exec<C: Command>(
    ctx: &mut Context<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    let args = if args.is_null() {
        serde_json::Value::Object(Default::default())
    } else {
        args
    };
    let command: C = serde_json::from_value(args).map_err(|e| {
        ApiError::new(codes::INVALID_ARGUMENTS, format!("{}: {e}", C::NAME))
            .with_details(serde_json::json!({ "command": C::NAME }))
    })?;
    let output = command.apply(ctx)?;
    serde_json::to_value(output).map_err(|e| ApiError::new(codes::INTERNAL, e.to_string()))
}

impl Registry {
    /// A registry with every built-in command and query.
    pub fn standard() -> Registry {
        let mut registry = Registry {
            commands: BTreeMap::new(),
            queries: BTreeMap::new(),
        };
        commands::register(&mut registry);
        registry.register_history();
        queries::register(&mut registry);
        registry
    }

    /// Registers command `C`.
    pub fn register<C: Command>(&mut self) {
        self.commands.insert(
            C::NAME,
            CommandDescriptor {
                name: C::NAME,
                summary: C::SUMMARY,
                input_schema: schemars::schema_for!(C),
                output_schema: schemars::schema_for!(C::Output),
                exec: Some(exec::<C>),
            },
        );
    }

    pub(crate) fn register_query<A: JsonSchema>(
        &mut self,
        name: &'static str,
        summary: &'static str,
        run: QueryFn,
    ) {
        self.queries.insert(
            name,
            QueryDescriptor {
                name,
                summary,
                input_schema: schemars::schema_for!(A),
                run,
            },
        );
    }

    fn register_history(&mut self) {
        let empty = schemars::json_schema!({ "type": "object", "additionalProperties": false });
        for (name, summary) in [
            (
                crate::UNDO,
                "Undo the most recent committed transaction. Must be the only command in its request.",
            ),
            (
                crate::REDO,
                "Redo the most recently undone transaction. Must be the only command in its request.",
            ),
        ] {
            self.commands.insert(
                name,
                CommandDescriptor {
                    name,
                    summary,
                    input_schema: empty.clone(),
                    output_schema: schemars::schema_for!(crate::history::HistoryOutput),
                    exec: None,
                },
            );
        }
    }

    /// Looks up a command.
    pub fn command(&self, name: &str) -> Option<&CommandDescriptor> {
        self.commands.get(name)
    }

    /// Looks up a query.
    pub fn query(&self, name: &str) -> Option<&QueryDescriptor> {
        self.queries.get(name)
    }

    /// All commands, sorted by name.
    pub fn commands(&self) -> impl Iterator<Item = &CommandDescriptor> {
        self.commands.values()
    }

    /// All queries, sorted by name.
    pub fn queries(&self) -> impl Iterator<Item = &QueryDescriptor> {
        self.queries.values()
    }

    /// Names of commands and queries similar to `name`, for error hints.
    pub(crate) fn suggestions(&self, name: &str) -> Vec<&'static str> {
        let area = name.split('.').next().unwrap_or(name);
        self.commands
            .keys()
            .chain(self.queries.keys())
            .copied()
            .filter(|candidate| candidate.starts_with(area) || candidate.contains(name))
            .collect()
    }
}
