use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{
    FORMAT_VERSION, Id, IdKind, Manifest, Media, ModelError, ProjectStore, READABLE_VERSIONS,
    Timeline, TimelineEntry, TimelineFormat, validate,
};

/// Path of the manifest inside a bundle.
pub const MANIFEST_PATH: &str = "project.json";
/// Path of the media table inside a bundle.
pub const MEDIA_PATH: &str = "media.json";
/// Directory of timeline documents inside a bundle.
pub const TIMELINES_DIR: &str = "timelines";
/// Path of the append-only operation log inside a bundle.
pub const OPS_LOG_PATH: &str = "history/ops.jsonl";

/// Identifies one stored document of a project.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DocumentKey {
    /// `project.json`.
    Manifest,
    /// `media.json`.
    Media,
    /// `timelines/<id>.json`.
    Timeline(Id),
}

impl DocumentKey {
    /// Path of the document inside the bundle.
    pub fn path(&self) -> String {
        match self {
            DocumentKey::Manifest => MANIFEST_PATH.to_string(),
            DocumentKey::Media => MEDIA_PATH.to_string(),
            DocumentKey::Timeline(id) => format!("{TIMELINES_DIR}/{id}.json"),
        }
    }

    /// Parses a bundle path back into a key.
    pub fn from_path(path: &str) -> Option<DocumentKey> {
        match path {
            MANIFEST_PATH => Some(DocumentKey::Manifest),
            MEDIA_PATH => Some(DocumentKey::Media),
            _ => path
                .strip_prefix("timelines/")
                .and_then(|rest| rest.strip_suffix(".json"))
                .and_then(|id| Id::parse(id).ok())
                .map(DocumentKey::Timeline),
        }
    }
}

impl fmt::Display for DocumentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path())
    }
}

impl Serialize for DocumentKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.path())
    }
}

impl<'de> Deserialize<'de> for DocumentKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<DocumentKey, D::Error> {
        let path = String::deserialize(deserializer)?;
        DocumentKey::from_path(&path)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown document path {path:?}")))
    }
}

/// A whole project in memory: manifest, media table and timelines.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// The manifest.
    pub manifest: Manifest,
    /// The media table.
    pub media: Media,
    /// Timelines by id.
    pub timelines: BTreeMap<Id, Timeline>,
}

impl Project {
    /// A new project with one empty primary timeline named "Timeline 1".
    pub fn new(id: Id, name: impl Into<String>, format: TimelineFormat) -> Project {
        let timeline_id = Id::new(IdKind::Timeline, 0);
        let timeline_name = "Timeline 1".to_string();
        let timeline = Timeline {
            id: timeline_id.clone(),
            name: timeline_name.clone(),
            format,
            tracks: Vec::new(),
            markers: Vec::new(),
        };
        Project {
            manifest: Manifest {
                format_version: FORMAT_VERSION.to_string(),
                id,
                name: name.into(),
                revision: 0,
                next_id: 1,
                primary_timeline: timeline_id.clone(),
                timelines: vec![TimelineEntry {
                    role: Default::default(),
                    id: timeline_id.clone(),
                    name: timeline_name,
                }],
                text_presets: Vec::new(),
            },
            media: Media::default(),
            timelines: BTreeMap::from([(timeline_id, timeline)]),
        }
    }

    /// Allocates a new id of `kind`. Ids are never reused within a project.
    pub fn allocate_id(&mut self, kind: IdKind) -> Id {
        let id = Id::new(kind, self.manifest.next_id);
        self.manifest.next_id += 1;
        id
    }

    /// The primary timeline.
    pub fn primary_timeline(&self) -> Option<&Timeline> {
        self.timelines.get(&self.manifest.primary_timeline)
    }

    /// Loads and validates a project from `store`.
    pub fn load(store: &dyn ProjectStore) -> Result<Project, ModelError> {
        let manifest_value: serde_json::Value = read_json(store, MANIFEST_PATH)?;
        let found = manifest_value
            .get("format_version")
            .and_then(|v| v.as_str())
            .unwrap_or("missing");
        if found != FORMAT_VERSION && !READABLE_VERSIONS.contains(&found) {
            return Err(ModelError::UnsupportedVersion {
                found: found.to_string(),
                supported: FORMAT_VERSION,
            });
        }
        let mut manifest: Manifest = from_value(MANIFEST_PATH, manifest_value)?;
        // Older versions are subsets of this one; they are saved as the current version.
        manifest.format_version = FORMAT_VERSION.to_string();
        let media: Media = if store.exists(MEDIA_PATH) {
            read_json(store, MEDIA_PATH)?
        } else {
            Media::default()
        };
        let mut timelines = BTreeMap::new();
        for entry in &manifest.timelines {
            let key = DocumentKey::Timeline(entry.id.clone());
            let timeline: Timeline = read_json(store, &key.path())?;
            timelines.insert(entry.id.clone(), timeline);
        }
        let project = Project {
            manifest,
            media,
            timelines,
        };
        let issues = validate(&project);
        if issues.is_empty() {
            Ok(project)
        } else {
            Err(ModelError::Invalid(issues))
        }
    }

    /// Every document of the project.
    pub fn document_keys(&self) -> Vec<DocumentKey> {
        let mut keys = vec![DocumentKey::Manifest, DocumentKey::Media];
        keys.extend(self.timelines.keys().cloned().map(DocumentKey::Timeline));
        keys
    }

    /// Writes every document.
    pub fn save_all(&self, store: &mut dyn ProjectStore) -> Result<(), ModelError> {
        self.save_documents(store, &self.document_keys())
    }

    /// Writes the given documents. The manifest is written last so that it never refers
    /// to a timeline document that is not yet on disk.
    pub fn save_documents(
        &self,
        store: &mut dyn ProjectStore,
        keys: &[DocumentKey],
    ) -> Result<(), ModelError> {
        let mut ordered: Vec<&DocumentKey> = keys
            .iter()
            .filter(|k| **k != DocumentKey::Manifest)
            .collect();
        if keys.contains(&DocumentKey::Manifest) {
            ordered.push(&DocumentKey::Manifest);
        }
        for key in ordered {
            match self.document_value(key) {
                Some(value) => store.write_atomic(&key.path(), &to_document_bytes(&value))?,
                // A removed timeline: its document goes too.
                None => store.remove(&key.path())?,
            }
        }
        Ok(())
    }

    /// The JSON value of one document, or `None` for an unknown timeline.
    pub fn document_value(&self, key: &DocumentKey) -> Option<serde_json::Value> {
        let value = match key {
            DocumentKey::Manifest => serde_json::to_value(&self.manifest),
            DocumentKey::Media => serde_json::to_value(&self.media),
            DocumentKey::Timeline(id) => serde_json::to_value(self.timelines.get(id)?),
        };
        value.ok()
    }

    /// Replaces one document from its JSON value; `null` removes a timeline document.
    pub fn set_document_value(
        &mut self,
        key: &DocumentKey,
        value: serde_json::Value,
    ) -> Result<(), ModelError> {
        let path = key.path();
        match key {
            DocumentKey::Manifest => self.manifest = from_value(&path, value)?,
            DocumentKey::Media => self.media = from_value(&path, value)?,
            DocumentKey::Timeline(id) => {
                if value.is_null() {
                    self.timelines.remove(id);
                } else {
                    self.timelines.insert(id.clone(), from_value(&path, value)?);
                }
            }
        }
        Ok(())
    }
}

/// Serializes a document the way FossCut stores it: pretty JSON with a trailing newline.
pub(crate) fn to_document_bytes(value: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

fn read_json<T: DeserializeOwned>(store: &dyn ProjectStore, path: &str) -> Result<T, ModelError> {
    let bytes = store.read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| ModelError::Json {
        path: path.to_string(),
        message: e.to_string(),
    })
}

fn from_value<T: DeserializeOwned>(path: &str, value: serde_json::Value) -> Result<T, ModelError> {
    serde_json::from_value(value).map_err(|e| ModelError::Json {
        path: path.to_string(),
        message: e.to_string(),
    })
}
