use fosscut_model::{DocumentKey, MANIFEST_PATH, OPS_LOG_PATH, Project, ProjectStore, validate};
use serde_json::{Value, json};

use crate::changes::changed_documents;
use crate::history::{DocumentChange, HistoryOutput, parse_log, stacks};
use crate::queries::QueryContext;
use crate::{
    ApiError, ChangeSummary, Context, EntryKind, LogEntry, REDO, Registry, Request, Response, UNDO,
    codes,
};

/// An open project: the single writer of its documents (ADR-0002).
///
/// Every write goes through [`Session::execute`], which applies the agent safety
/// contract of ADR-0006 and appends to the operation log before writing documents.
pub struct Session<S: ProjectStore> {
    store: S,
    project: Project,
    registry: Registry,
    log: Vec<LogEntry>,
}

impl<S: ProjectStore> Session<S> {
    /// Creates a new project in an empty store.
    pub fn create(mut store: S, project: Project) -> Result<Session<S>, ApiError> {
        if store.exists(MANIFEST_PATH) {
            return Err(ApiError::new(
                codes::REQUEST_INVALID,
                "a project already exists at this location",
            )
            .with_hint("Open the existing project or choose another location."));
        }
        let issues = validate(&project);
        if !issues.is_empty() {
            return Err(
                ApiError::new(codes::MODEL_INVALID, "the new project is invalid")
                    .with_details(serde_json::to_value(issues).unwrap_or_default()),
            );
        }
        project.save_all(&mut store)?;
        Ok(Session {
            store,
            project,
            registry: Registry::standard(),
            log: Vec::new(),
        })
    }

    /// Opens a project, completing a save that was interrupted after its log entry was written.
    pub fn open(store: S) -> Result<Session<S>, ApiError> {
        Session::load(store, true)
    }

    /// Opens a project to read it while another session may be writing it (ADR-0022): a
    /// save that was interrupted, or is still in progress, is completed in memory only, and
    /// nothing is written to the store.
    pub fn open_read_only(store: S) -> Result<Session<S>, ApiError> {
        Session::load(store, false)
    }

    fn load(mut store: S, repair: bool) -> Result<Session<S>, ApiError> {
        let mut completed = None;
        let log = if store.exists(OPS_LOG_PATH) {
            parse_log(&store.read(OPS_LOG_PATH)?)?
        } else {
            Vec::new()
        };
        if let Some(last) = log.last() {
            let manifest: Value =
                serde_json::from_slice(&store.read(MANIFEST_PATH)?).map_err(|e| {
                    ApiError::new("fosscut.project.malformed", format!("project.json: {e}"))
                })?;
            let on_disk = manifest
                .get("revision")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if last.revision_after > on_disk {
                let mut recovered =
                    Project::load(&store).or_else(|_| recover_from_entry(&store, last))?;
                for change in &last.documents {
                    recovered.set_document_value(&change.doc, change.after.clone())?;
                }
                if repair {
                    let keys: Vec<DocumentKey> =
                        last.documents.iter().map(|c| c.doc.clone()).collect();
                    recovered.save_documents(&mut store, &keys)?;
                } else {
                    completed = Some(recovered);
                }
            }
        }
        let project = match completed {
            Some(project) => project,
            None => Project::load(&store)?,
        };
        Ok(Session {
            store,
            project,
            registry: Registry::standard(),
            log,
        })
    }

    /// The current project state.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The current revision.
    pub fn revision(&self) -> u64 {
        self.project.manifest.revision
    }

    /// Registered commands and queries.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The operation log, oldest first.
    pub fn log(&self) -> &[LogEntry] {
        &self.log
    }

    /// The storage backend.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Runs a read-only query.
    pub fn query(&self, name: &str, args: Value) -> Result<Value, ApiError> {
        let descriptor = self
            .registry
            .query(name)
            .ok_or_else(|| self.unknown(codes::UNKNOWN_QUERY, name))?;
        (descriptor.run)(
            &QueryContext {
                project: &self.project,
                log: &self.log,
                registry: &self.registry,
            },
            args,
        )
    }

    /// Applies a write request as one transaction.
    pub fn execute(&mut self, request: Request) -> Result<Response, ApiError> {
        if request.commands.is_empty() {
            return Err(ApiError::new(
                codes::REQUEST_INVALID,
                "the request has no commands",
            ));
        }
        if !request.dry_run
            && let Some(id) = &request.request_id
            && let Some(entry) = self
                .log
                .iter()
                .rev()
                .find(|e| e.request_id.as_ref() == Some(id))
        {
            if entry.commands == request.commands {
                return Ok(Response {
                    committed: true,
                    dry_run: false,
                    replayed: true,
                    revision: entry.revision_after,
                    results: entry.results.clone(),
                    changes: entry.changes.clone(),
                });
            }
            return Err(ApiError::new(
                codes::REQUEST_REUSED,
                format!("request_id {id:?} was already used for different commands"),
            )
            .with_details(json!({ "seq": entry.seq })));
        }
        let current = self.revision();
        if let Some(expected) = request.expected_revision
            && expected != current
        {
            let since: Vec<Value> = self
                .log
                .iter()
                .filter(|e| e.revision_after > expected)
                .map(|e| json!({ "seq": e.seq, "kind": e.kind, "revision": e.revision_after, "actor": e.actor, "commands": e.commands.iter().map(|c| c.command.clone()).collect::<Vec<_>>() }))
                .collect();
            return Err(ApiError::new(
                codes::REVISION_CONFLICT,
                format!("the project is at revision {current}, not {expected}"),
            )
            .with_details(json!({ "expected_revision": expected, "current_revision": current, "since": since })));
        }
        if request
            .commands
            .iter()
            .any(|c| c.command == UNDO || c.command == REDO)
        {
            if request.commands.len() != 1 {
                return Err(ApiError::new(
                    codes::REQUEST_INVALID,
                    "history.undo and history.redo must be the only command in a request",
                ));
            }
            let kind = if request.commands[0].command == UNDO {
                EntryKind::Undo
            } else {
                EntryKind::Redo
            };
            return self.history(kind, &request);
        }

        let mut working = self.project.clone();
        let mut results = Vec::with_capacity(request.commands.len());
        {
            let mut ctx = Context::new(&mut working, request.allow_destructive);
            for (index, call) in request.commands.iter().enumerate() {
                let descriptor = self
                    .registry
                    .command(&call.command)
                    .ok_or_else(|| self.unknown(codes::UNKNOWN_COMMAND, &call.command))?;
                let exec = descriptor.exec.ok_or_else(|| {
                    ApiError::new(codes::INTERNAL, "command has no implementation")
                })?;
                let result = exec(&mut ctx, call.args.clone())
                    .map_err(|e| annotate(e, index, &call.command))?;
                results.push(result);
            }
        }
        let issues = validate(&working);
        if !issues.is_empty() {
            return Err(ApiError::new(
                codes::MODEL_INVALID,
                "the request would leave the project invalid",
            )
            .with_details(serde_json::to_value(issues).unwrap_or_default()));
        }
        let changes = ChangeSummary::between(&self.project, &working);
        if request.dry_run || changed_documents(&self.project, &working).is_empty() {
            return Ok(Response {
                committed: false,
                dry_run: request.dry_run,
                replayed: false,
                revision: current,
                results,
                changes,
            });
        }
        working.manifest.revision = current + 1;
        self.commit(EntryKind::Commit, working, &request, results, changes, None)
    }

    fn history(&mut self, kind: EntryKind, request: &Request) -> Result<Response, ApiError> {
        let (undo, redo) = stacks(&self.log);
        let target_seq =
            match kind {
                EntryKind::Undo => undo.last().copied().ok_or_else(|| {
                    ApiError::new(codes::NOTHING_TO_UNDO, "there is nothing to undo")
                })?,
                _ => redo.last().copied().ok_or_else(|| {
                    ApiError::new(codes::NOTHING_TO_REDO, "there is nothing to redo")
                })?,
            };
        let target = self
            .log
            .iter()
            .find(|e| e.seq == target_seq)
            .cloned()
            .ok_or_else(|| {
                ApiError::new(
                    codes::HISTORY_CORRUPT,
                    format!("log entry {target_seq} is missing"),
                )
            })?;
        let mut working = self.project.clone();
        for change in &target.documents {
            let (expected_now, restore) = if kind == EntryKind::Undo {
                (&change.after, &change.before)
            } else {
                (&change.before, &change.after)
            };
            let now = self
                .project
                .document_value(&change.doc)
                .unwrap_or(Value::Null);
            if !same_document(&change.doc, &now, expected_now) {
                return Err(ApiError::new(
                    codes::HISTORY_DIVERGED,
                    format!("{} changed outside the operation log", change.doc),
                ));
            }
            working.set_document_value(&change.doc, restore.clone())?;
        }
        let current = self.revision();
        working.manifest.revision = current + 1;
        working.manifest.next_id = working.manifest.next_id.max(self.project.manifest.next_id);
        let issues = validate(&working);
        if !issues.is_empty() {
            return Err(ApiError::new(
                codes::MODEL_INVALID,
                "history would leave the project invalid",
            )
            .with_details(serde_json::to_value(issues).unwrap_or_default()));
        }
        let changes = ChangeSummary::between(&self.project, &working);
        let output = serde_json::to_value(HistoryOutput {
            target_seq,
            changes: changes.clone(),
        })
        .unwrap_or_default();
        if request.dry_run {
            return Ok(Response {
                committed: false,
                dry_run: true,
                replayed: false,
                revision: current,
                results: vec![output],
                changes,
            });
        }
        self.commit(
            kind,
            working,
            request,
            vec![output],
            changes,
            Some(target_seq),
        )
    }

    fn commit(
        &mut self,
        kind: EntryKind,
        working: Project,
        request: &Request,
        results: Vec<Value>,
        changes: ChangeSummary,
        target_seq: Option<u64>,
    ) -> Result<Response, ApiError> {
        let keys = changed_documents(&self.project, &working);
        let documents = keys
            .iter()
            .map(|key| DocumentChange {
                doc: key.clone(),
                before: self.project.document_value(key).unwrap_or(Value::Null),
                after: working.document_value(key).unwrap_or(Value::Null),
            })
            .collect();
        let entry = LogEntry {
            seq: self.log.last().map_or(1, |e| e.seq + 1),
            kind,
            revision_before: self.revision(),
            revision_after: working.manifest.revision,
            actor: request.actor.clone(),
            reason: request.reason.clone(),
            request_id: request.request_id.clone(),
            commands: request.commands.clone(),
            target_seq,
            at_ms: request.at_ms,
            results: results.clone(),
            changes: changes.clone(),
            documents,
        };
        let mut line = serde_json::to_vec(&entry)
            .map_err(|e| ApiError::new(codes::INTERNAL, e.to_string()))?;
        line.push(b'\n');
        self.store.append(OPS_LOG_PATH, &line)?;
        working.save_documents(&mut self.store, &keys)?;
        self.project = working;
        self.log.push(entry);
        Ok(Response {
            committed: true,
            dry_run: false,
            replayed: false,
            revision: self.revision(),
            results,
            changes,
        })
    }

    fn unknown(&self, code: &str, name: &str) -> ApiError {
        let suggestions = self.registry.suggestions(name);
        let error = ApiError::new(code, format!("{name:?} is not a known command or query"));
        if suggestions.is_empty() {
            error
        } else {
            error.with_hint(format!(
                "Did you mean one of: {}? See capabilities.",
                suggestions.join(", ")
            ))
        }
    }
}

/// Rebuilds a project from the last log entry when the documents on disk are mid-save.
fn recover_from_entry(store: &dyn ProjectStore, entry: &LogEntry) -> Result<Project, ApiError> {
    let mut manifest: Option<Value> = None;
    let mut media: Option<Value> = None;
    for change in &entry.documents {
        match change.doc {
            DocumentKey::Manifest => manifest = Some(change.after.clone()),
            DocumentKey::Media => media = Some(change.after.clone()),
            DocumentKey::Timeline(_) => {}
        }
    }
    let read = |path: &str| -> Result<Value, ApiError> {
        serde_json::from_slice(&store.read(path)?)
            .map_err(|e| ApiError::new("fosscut.project.malformed", format!("{path}: {e}")))
    };
    let manifest = match manifest {
        Some(value) => value,
        None => read(MANIFEST_PATH)?,
    };
    let manifest: fosscut_model::Manifest = serde_json::from_value(manifest)
        .map_err(|e| ApiError::new("fosscut.project.malformed", format!("project.json: {e}")))?;
    let media = match media {
        Some(value) => value,
        None if store.exists(fosscut_model::MEDIA_PATH) => read(fosscut_model::MEDIA_PATH)?,
        None => json!({ "assets": [] }),
    };
    let mut project = Project {
        manifest,
        media: serde_json::from_value(media)
            .map_err(|e| ApiError::new("fosscut.project.malformed", format!("media.json: {e}")))?,
        timelines: Default::default(),
    };
    for timeline in project.manifest.timelines.clone() {
        let key = DocumentKey::Timeline(timeline.id.clone());
        let value = match entry.documents.iter().find(|c| c.doc == key) {
            Some(change) => change.after.clone(),
            None => read(&key.path())?,
        };
        project.set_document_value(&key, value)?;
    }
    Ok(project)
}

/// Document equality for history checks; the manifest's revision and id counter are
/// expected to move on and are ignored.
fn same_document(key: &DocumentKey, a: &Value, b: &Value) -> bool {
    if *key != DocumentKey::Manifest {
        return a == b;
    }
    let strip = |value: &Value| {
        let mut value = value.clone();
        if let Some(object) = value.as_object_mut() {
            object.remove("revision");
            object.remove("next_id");
        }
        value
    };
    strip(a) == strip(b)
}

/// Adds the failing command's position to an error.
fn annotate(mut error: ApiError, index: usize, command: &str) -> ApiError {
    match &mut error.details {
        Value::Object(map) => {
            map.insert("command_index".into(), index.into());
            map.insert("command".into(), command.into());
        }
        Value::Null => error.details = json!({ "command_index": index, "command": command }),
        other => {
            error.details =
                json!({ "command_index": index, "command": command, "details": other.clone() })
        }
    }
    error
}
