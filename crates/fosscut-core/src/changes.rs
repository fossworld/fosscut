use std::collections::BTreeMap;

use fosscut_model::{DocumentKey, Project};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Objects added, removed or modified by a transaction, by id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeSummary {
    /// Ids of new objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    /// Ids of removed objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Ids of changed objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modified: Vec<String>,
}

impl ChangeSummary {
    /// True when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    /// Compares two project states object by object (assets, timelines, tracks, clips).
    pub fn between(before: &Project, after: &Project) -> ChangeSummary {
        let old = objects(before);
        let new = objects(after);
        let mut summary = ChangeSummary::default();
        for (id, value) in &new {
            match old.get(id) {
                None => summary.added.push(id.clone()),
                Some(previous) if previous != value => summary.modified.push(id.clone()),
                Some(_) => {}
            }
        }
        summary.removed = old
            .keys()
            .filter(|id| !new.contains_key(*id))
            .cloned()
            .collect();
        summary
    }
}

/// Every addressable object as JSON, with child collections stripped so that a change to
/// a clip is reported for the clip only.
fn objects(project: &Project) -> BTreeMap<String, serde_json::Value> {
    let mut map = BTreeMap::new();
    for asset in &project.media.assets {
        map.insert(
            asset.id.to_string(),
            serde_json::to_value(asset).unwrap_or_default(),
        );
    }
    for timeline in project.timelines.values() {
        let mut header = serde_json::to_value(timeline).unwrap_or_default();
        if let Some(object) = header.as_object_mut() {
            object.remove("tracks");
        }
        map.insert(timeline.id.to_string(), header);
        for (index, track) in timeline.tracks.iter().enumerate() {
            let mut value = serde_json::to_value(track).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.remove("clips");
                object.insert("index".into(), index.into());
            }
            map.insert(track.id.to_string(), value);
            for clip in &track.clips {
                let mut value = serde_json::to_value(clip).unwrap_or_default();
                if let Some(object) = value.as_object_mut() {
                    object.insert("track".into(), track.id.to_string().into());
                }
                map.insert(clip.id.to_string(), value);
            }
        }
    }
    map
}

/// Documents whose stored JSON differs between two project states.
pub(crate) fn changed_documents(before: &Project, after: &Project) -> Vec<DocumentKey> {
    let mut keys = before.document_keys();
    for key in after.document_keys() {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.into_iter()
        .filter(|key| before.document_value(key) != after.document_value(key))
        .collect()
}
