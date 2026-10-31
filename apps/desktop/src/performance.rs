//! Speed and disk use as the preferences set them (ADR-0031, item 4; ADR-0032): hardware
//! acceleration, what the preview decodes and keeps (proxies, remuxed copies, rendered
//! frames), the cache policy, and the background jobs that make proxies and remuxed copies
//! of the open project's media. None of it changes what an export writes.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use fosscut_core::ApiError;
use fosscut_engine::cache::{CacheKind, CachePolicy, CacheUsage};
use fosscut_engine::hardware::{HardwareDecoding, HardwareEncoding};
use fosscut_engine::jobs::{JobState, Jobs};
use fosscut_engine::preferences::{DecodingChoice, Preferences, ProxyChoice, ProxyScale};
use fosscut_engine::preview::PreviewOptions;
use fosscut_engine::proxy::{ProxyPolicy, ProxySize};
use fosscut_model::{AssetKind, Id, Project};
use serde::Serialize;
use serde_json::{Value, json};

/// Sets the process's hardware decoding and encoding from `performance`.
pub fn apply_hardware(preferences: &Preferences) {
    fosscut_engine::hardware::set_hardware_decoding(hardware_decoding(preferences));
    fosscut_engine::hardware::set_hardware_encoding(hardware_encoding(preferences));
}

/// `performance.hardware_encoding` as the encoders take it.
pub fn hardware_encoding(preferences: &Preferences) -> HardwareEncoding {
    if preferences.performance.hardware_encoding {
        HardwareEncoding::Auto
    } else {
        HardwareEncoding::Off
    }
}

/// `performance.hardware_decoding` as the decoders take it.
pub fn hardware_decoding(preferences: &Preferences) -> HardwareDecoding {
    match preferences.performance.hardware_decoding {
        DecodingChoice::Auto => HardwareDecoding::Auto,
        DecodingChoice::Off => HardwareDecoding::Off,
    }
}

/// `performance.proxies` as the proxy jobs take it.
pub fn proxy_policy(preferences: &Preferences) -> ProxyPolicy {
    match preferences.performance.proxies {
        ProxyChoice::Off => ProxyPolicy::Off,
        ProxyChoice::Auto => ProxyPolicy::Auto,
        ProxyChoice::On => ProxyPolicy::On,
    }
}

/// `performance.proxy_size` as the proxy jobs take it.
pub fn proxy_size(preferences: &Preferences) -> ProxySize {
    match preferences.performance.proxy_size {
        ProxyScale::Half => ProxySize::Half,
        ProxyScale::Quarter => ProxySize::Quarter,
    }
}

/// What the preview decodes and keeps, from `performance`, with the caches in `cache`.
pub fn preview_options(preferences: &Preferences, cache: &Path) -> PreviewOptions {
    let performance = &preferences.performance;
    PreviewOptions {
        cache: Some(cache.to_path_buf()),
        proxies: (performance.proxies != ProxyChoice::Off).then(|| proxy_size(preferences)),
        remuxed: performance.auto_remux,
        hardware_decoding: hardware_decoding(preferences),
        render_cache_bytes: (performance.render_cache_gb > 0.0)
            .then(|| preferences.render_cache_bytes()),
    }
}

/// The cache policy of `cache` from `cache.max_age_days` and `cache.limit_gb`.
pub fn cache_policy(preferences: &Preferences, cache: &Path) -> CachePolicy {
    CachePolicy {
        cache: cache.to_path_buf(),
        max_age_days: preferences.cache.max_age_days,
        limit_bytes: preferences.cache_limit_bytes(),
    }
}

/// The caches' sizes, as `cache_info` answers.
pub fn cache_listing(cache: &Path) -> Vec<CacheUsage> {
    fosscut_engine::cache::cache_info(cache)
}

/// Empties the caches named in `kinds` (every cache when empty) and answers `{freed_bytes,
/// caches}`, the caches' sizes after.
pub fn clear_caches(cache: &Path, kinds: &[String]) -> Result<Value, ApiError> {
    let kinds = kinds
        .iter()
        .map(|name| CacheKind::parse(name.trim()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(ApiError::from)?;
    let freed = fosscut_engine::cache::clear(cache, &kinds);
    Ok(json!({ "freed_bytes": freed, "caches": cache_listing(cache) }))
}

/// A copy of an asset's media the preview can use instead of the original.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaJob {
    /// A proxy of this size.
    Proxy {
        /// The asset.
        asset: Id,
        /// How much smaller than the original.
        size: ProxySize,
    },
    /// A copy of its streams in QuickTime, for seeking.
    Remux {
        /// The asset.
        asset: Id,
    },
}

impl MediaJob {
    /// The asset it copies.
    pub fn asset(&self) -> &Id {
        match self {
            MediaJob::Proxy { asset, .. } | MediaJob::Remux { asset } => asset,
        }
    }
}

/// The proxies and remuxed copies `project`'s media should have and has not yet: with
/// `performance.proxies` auto or on, a proxy of each video asset the policy wants; with
/// `performance.auto_remux`, a remuxed copy of each video asset in a container that seeks
/// badly. Assets whose files are missing get neither.
pub fn wanted(
    project: &Project,
    bundle: &Path,
    preferences: &Preferences,
    cache: &Path,
) -> Vec<MediaJob> {
    let policy = proxy_policy(preferences);
    let size = proxy_size(preferences);
    let mut wanted = Vec::new();
    for asset in &project.media.assets {
        let id = &asset.id;
        if asset.kind != AssetKind::Video {
            continue;
        }
        let Ok(file) = fosscut_engine::import::asset_file(project, bundle, id) else {
            continue;
        };
        if !file.is_file() {
            continue;
        }
        if fosscut_engine::proxy::wants_proxy(asset, policy)
            && matches!(
                fosscut_engine::proxy::existing_proxy(project, bundle, id, size, cache),
                Ok(None)
            )
        {
            wanted.push(MediaJob::Proxy {
                asset: id.clone(),
                size,
            });
        }
        if preferences.performance.auto_remux
            && fosscut_engine::remux::needs_remux(&file, asset.info.container.as_deref())
            && matches!(
                fosscut_engine::remux::existing_remux(project, bundle, id, cache),
                Ok(None)
            )
        {
            wanted.push(MediaJob::Remux { asset: id.clone() });
        }
    }
    wanted
}

/// How many of one kind of copy are made of how many asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Count {
    /// Finished, successfully or not.
    pub done: u32,
    /// Asked for since the queue was last empty.
    pub total: u32,
}

/// What the status line shows: the copies being made, and how many could not be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MediaWork {
    /// Proxies.
    pub proxies: Count,
    /// Remuxed copies.
    pub remux: Count,
    /// Copies that failed since the app started; they are not asked for again.
    pub failed: u32,
}

impl MediaWork {
    /// True while copies are being made.
    pub fn busy(&self) -> bool {
        self.proxies.done < self.proxies.total || self.remux.done < self.remux.total
    }
}

struct Queue {
    waiting: VecDeque<(PathBuf, Project, MediaJob)>,
    /// Asked for and not yet made, or failed: never asked for twice.
    known: HashSet<(PathBuf, MediaJob)>,
    work: MediaWork,
}

/// The proxies and remuxed copies the app makes, one at a time, in the background.
pub struct MediaJobs {
    queue: Mutex<Queue>,
    wake: Condvar,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Default for MediaJobs {
    fn default() -> MediaJobs {
        MediaJobs {
            queue: Mutex::new(Queue {
                waiting: VecDeque::new(),
                known: HashSet::new(),
                work: MediaWork::default(),
            }),
            wake: Condvar::new(),
        }
    }
}

impl MediaJobs {
    /// Asks for the copies in `wanted` of `project`'s media in `bundle` that are not asked
    /// for yet; true when any are new.
    pub fn ask(&self, bundle: &Path, project: &Project, wanted: Vec<MediaJob>) -> bool {
        let mut queue = guard(&self.queue);
        let mut added = false;
        for job in wanted {
            if !queue.known.insert((bundle.to_path_buf(), job.clone())) {
                continue;
            }
            match job {
                MediaJob::Proxy { .. } => queue.work.proxies.total += 1,
                MediaJob::Remux { .. } => queue.work.remux.total += 1,
            }
            queue
                .waiting
                .push_back((bundle.to_path_buf(), project.clone(), job));
            added = true;
        }
        drop(queue);
        if added {
            self.wake.notify_all();
        }
        added
    }

    /// What is being made.
    pub fn status(&self) -> MediaWork {
        guard(&self.queue).work
    }

    /// The next copy to make, waiting for one.
    fn next(&self) -> (PathBuf, Project, MediaJob) {
        let mut queue = guard(&self.queue);
        loop {
            if let Some(next) = queue.waiting.pop_front() {
                return next;
            }
            queue = self
                .wake
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Counts a copy finished; once nothing waits, the counts start again from zero.
    fn finished(&self, bundle: PathBuf, job: MediaJob, succeeded: bool) -> MediaWork {
        let mut queue = guard(&self.queue);
        match job {
            MediaJob::Proxy { .. } => queue.work.proxies.done += 1,
            MediaJob::Remux { .. } => queue.work.remux.done += 1,
        }
        if succeeded {
            // Made: it exists now, so it is not wanted again; asking again is harmless.
            queue.known.remove(&(bundle, job));
        } else {
            queue.work.failed += 1;
        }
        if queue.waiting.is_empty() && !queue.work.busy() {
            queue.work.proxies = Count::default();
            queue.work.remux = Count::default();
        }
        queue.work
    }

    /// Makes the copies asked for on a thread of its own, one at a time, as jobs of
    /// `jobs` in `cache`. `changed` hears the status after each one, with whether it
    /// succeeded (the preview then opens its media again to use it).
    pub fn run(
        self: &Arc<Self>,
        jobs: Arc<Jobs>,
        cache: PathBuf,
        changed: impl Fn(MediaWork, bool) + Send + 'static,
    ) -> std::io::Result<()> {
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("fosscut-media-jobs".into())
            .spawn(move || {
                loop {
                    let (bundle, project, job) = this.next();
                    changed(this.status(), false);
                    let started = match &job {
                        MediaJob::Proxy { asset, size } => jobs.start_proxy(
                            project,
                            bundle.clone(),
                            asset.clone(),
                            *size,
                            cache.clone(),
                        ),
                        MediaJob::Remux { asset } => {
                            jobs.start_remux(project, bundle.clone(), asset.clone(), cache.clone())
                        }
                    };
                    let succeeded = started.is_ok_and(|status| {
                        let mut status = status;
                        while status.state == JobState::Running {
                            match jobs.status(&status.job, Duration::from_secs(60)) {
                                Ok(next) => status = next,
                                Err(_) => break,
                            }
                        }
                        status.state == JobState::Succeeded
                    });
                    changed(this.finished(bundle, job, succeeded), succeeded);
                }
            })
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fosscut-desktop-perf-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_preview_options_follow_the_performance_preferences() {
        let cache = PathBuf::from("/cache");
        let defaults = preview_options(&Preferences::default(), &cache);
        assert_eq!(
            defaults,
            PreviewOptions {
                cache: Some(cache.clone()),
                proxies: None,
                remuxed: true,
                hardware_decoding: HardwareDecoding::Auto,
                render_cache_bytes: Some(10_000_000_000),
            }
        );
        let mut preferences = Preferences::default();
        for (key, value) in [
            ("performance.proxies", "auto"),
            ("performance.proxy_size", "quarter"),
            ("performance.auto_remux", "false"),
            ("performance.hardware_decoding", "off"),
            ("performance.render_cache_gb", "0"),
        ] {
            preferences.set(key, value).unwrap();
        }
        let chosen = preview_options(&preferences, &cache);
        assert_eq!(chosen.proxies, Some(ProxySize::Quarter));
        assert!(!chosen.remuxed);
        assert_eq!(chosen.hardware_decoding, HardwareDecoding::Off);
        assert_eq!(
            chosen.render_cache_bytes, None,
            "0 turns the frame cache off"
        );
        preferences.set("performance.proxies", "on").unwrap();
        preferences.set("performance.proxy_size", "half").unwrap();
        assert_eq!(
            preview_options(&preferences, &cache).proxies,
            Some(ProxySize::Half)
        );
        assert_eq!(proxy_policy(&preferences), ProxyPolicy::On);

        preferences.set("cache.max_age_days", "7").unwrap();
        preferences.set("cache.limit_gb", "1.5").unwrap();
        assert_eq!(
            cache_policy(&preferences, &cache),
            CachePolicy {
                cache,
                max_age_days: 7,
                limit_bytes: 1_500_000_000
            }
        );
    }

    #[test]
    fn hardware_acceleration_follows_the_preferences() {
        // The choices only: setting them changes the whole process, other tests included.
        let defaults = Preferences::default();
        assert_eq!(hardware_decoding(&defaults), HardwareDecoding::Auto);
        assert_eq!(hardware_encoding(&defaults), HardwareEncoding::Auto);
        let mut preferences = Preferences::default();
        preferences
            .set("performance.hardware_decoding", "off")
            .unwrap();
        preferences
            .set("performance.hardware_encoding", "false")
            .unwrap();
        assert_eq!(hardware_decoding(&preferences), HardwareDecoding::Off);
        assert_eq!(hardware_encoding(&preferences), HardwareEncoding::Off);
    }

    #[test]
    fn caches_are_listed_and_cleared_by_kind() {
        let cache = folder("caches");
        for (path, bytes) in [
            ("analysis/a/loudness.json", 10),
            ("proxies/b/half@1.mov", 300),
            ("remux/c@1.mov", 200),
            ("frames/d.png", 40),
            ("frames/e.png", 60),
        ] {
            let file = cache.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, vec![0u8; bytes]).unwrap();
        }
        let listed = cache_listing(&cache);
        let sizes: Vec<(CacheKind, u64, u64)> = listed
            .iter()
            .map(|c| (c.kind, c.bytes, c.entries))
            .collect();
        assert_eq!(
            sizes,
            [
                (CacheKind::Analysis, 10, 1),
                (CacheKind::Proxies, 300, 1),
                (CacheKind::Remux, 200, 1),
                (CacheKind::Frames, 100, 2),
            ]
        );
        let cleared = clear_caches(&cache, &["frames".into(), " remux ".into()]).unwrap();
        assert_eq!(cleared["freed_bytes"], 300);
        assert_eq!(cleared["caches"][3]["bytes"], 0);
        assert_eq!(cleared["caches"][1]["bytes"], 300, "proxies stay");
        let refused = clear_caches(&cache, &["thumbnails".into()]).unwrap_err();
        assert_eq!(refused.code, fosscut_core::codes::INVALID_ARGUMENTS);
        assert!(refused.message.contains("there is no cache"), "{refused:?}");
        let all = clear_caches(&cache, &[]).unwrap();
        assert_eq!(all["freed_bytes"], 310, "an empty list clears every cache");
        let _ = std::fs::remove_dir_all(&cache);
    }

    /// A project with video assets whose files exist (but hold no media), and an image.
    fn project_with(
        work: &Path,
        files: &[(&str, u32)],
    ) -> (crate::workspace::Workspace, PathBuf, Vec<String>) {
        use crate::workspace::{NewProject, Workspace};
        let runtime = PathBuf::from(if cfg!(unix) { "/tmp" } else { "." })
            .join(format!("fcd-perf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let workspace = Workspace::new(runtime.clone(), None);
        workspace
            .create(&NewProject {
                path: work.join("Media.fosscut"),
                name: None,
                width: 1920,
                height: 1080,
                rate: "30".into(),
            })
            .unwrap();
        let mut ids = Vec::new();
        for (n, (name, width)) in files.iter().enumerate() {
            let file = work.join(name);
            std::fs::write(&file, format!("media {n}")).unwrap();
            let added = workspace
                .execute(fosscut_core::Request::single(
                    "media.add",
                    json!({
                        "name": name,
                        "kind": "video",
                        "path": { "absolute": file },
                        "identity": { "size": 7 + n, "sample_blake3": format!("{n:x}").repeat(64)[..64].to_string() },
                        "info": {
                            "duration": fosscut_time::FLICKS_PER_SECOND,
                            "container": if name.ends_with(".ts") { "mpegts" } else { "mov,mp4,m4a,3gp,3g2,mj2" },
                            "video": { "width": width, "height": width * 9 / 16, "rate": "30", "codec": "h264" }
                        }
                    }),
                ))
                .unwrap();
            ids.push(added.results[0]["asset"].as_str().unwrap().to_string());
        }
        (workspace, runtime, ids)
    }

    #[test]
    fn proxies_and_remuxed_copies_are_asked_for_as_the_preferences_say() {
        let work = folder("wanted");
        let cache = work.join("cache");
        let (workspace, runtime, ids) = project_with(
            &work,
            &[
                ("small.mp4", 1280),
                ("big.mov", 3840),
                ("broadcast.ts", 1920),
            ],
        );
        let (bundle, project, _) = workspace.snapshot().unwrap();
        let id = |n: usize| Id::parse(&ids[n]).unwrap();
        let mut preferences = Preferences::default();
        assert_eq!(
            wanted(&project, &bundle, &preferences, &cache),
            [MediaJob::Remux { asset: id(2) }],
            "proxies are off by default; remuxing is on"
        );
        preferences.set("performance.proxies", "auto").unwrap();
        preferences.set("performance.auto_remux", "false").unwrap();
        assert_eq!(
            wanted(&project, &bundle, &preferences, &cache),
            [MediaJob::Proxy {
                asset: id(1),
                size: ProxySize::Half
            }],
            "auto: only media larger than 1920 pixels"
        );
        preferences.set("performance.proxies", "on").unwrap();
        preferences
            .set("performance.proxy_size", "quarter")
            .unwrap();
        let every = wanted(&project, &bundle, &preferences, &cache);
        assert_eq!(every.len(), 3, "on: every video asset: {every:?}");
        assert!(every.iter().all(|job| matches!(
            job,
            MediaJob::Proxy {
                size: ProxySize::Quarter,
                ..
            }
        )));

        // A copy that exists is not asked for again, and a missing file gets none.
        let made = fosscut_engine::proxy::existing_proxy(
            &project,
            &bundle,
            &id(0),
            ProxySize::Quarter,
            &cache,
        );
        assert_eq!(made.unwrap(), None);
        let identity = project.media.get(&id(0)).unwrap().identity.clone().unwrap();
        let path = fosscut_engine::proxy::proxy_path(&cache, &identity, ProxySize::Quarter);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"proxy").unwrap();
        std::fs::remove_file(work.join("big.mov")).unwrap();
        assert_eq!(
            wanted(&project, &bundle, &preferences, &cache),
            [MediaJob::Proxy {
                asset: id(2),
                size: ProxySize::Quarter
            }]
        );
        workspace.close();
        let _ = std::fs::remove_dir_all(&runtime);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn copies_are_asked_for_once_and_counted_for_the_status_line() {
        let jobs = MediaJobs::default();
        let format: fosscut_model::TimelineFormat =
            serde_json::from_value(json!({ "width": 1920, "height": 1080, "rate": "30" })).unwrap();
        let project = Project::new(Id::parse("project_0001").unwrap(), "Counted", format);
        let bundle = PathBuf::from("/work/Counted.fosscut");
        let a = Id::parse("asset_0001").unwrap();
        let b = Id::parse("asset_0002").unwrap();
        let proxy = MediaJob::Proxy {
            asset: a.clone(),
            size: ProxySize::Half,
        };
        let remux = MediaJob::Remux { asset: b };
        assert!(jobs.ask(&bundle, &project, vec![proxy.clone(), remux.clone()]));
        assert!(
            !jobs.ask(&bundle, &project, vec![proxy.clone()]),
            "asked for already"
        );
        let work = jobs.status();
        assert_eq!(
            (work.proxies, work.remux),
            (Count { done: 0, total: 1 }, Count { done: 0, total: 1 })
        );
        assert!(work.busy());
        assert_eq!(jobs.next().2, proxy);
        let after = jobs.finished(bundle.clone(), proxy.clone(), true);
        assert_eq!(after.proxies, Count { done: 1, total: 1 });
        assert!(
            jobs.ask(&bundle, &project, vec![proxy.clone()]),
            "made, so asking again is new"
        );
        assert_eq!(jobs.next().2, remux);
        let failed = jobs.finished(bundle.clone(), remux.clone(), false);
        assert_eq!(failed.failed, 1);
        assert!(
            !jobs.ask(&bundle, &project, vec![remux]),
            "a failed copy is not asked for again"
        );
        assert_eq!(jobs.next().2, proxy);
        let done = jobs.finished(bundle, proxy, true);
        assert_eq!(
            done.proxies,
            Count::default(),
            "the counts start again once all are made"
        );
        assert!(!done.busy());
        assert_eq!(done.failed, 1);
    }
}
