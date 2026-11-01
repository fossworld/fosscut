//! Rendered preview frames kept on disk (ADR-0032, item 4), so showing a frame that was
//! rendered before skips rendering it, also after the app restarts.
//!
//! A frame is keyed by a [`FrameKey`]: a BLAKE3 hash of its render plan's encoding
//! ([`RenderPlan::cache_key_bytes`], which covers everything in the plan that affects
//! pixels), the output size and transfer function, what each asset the frame reads
//! contains and which copy of it was decoded, and [`VERSION`]. An edit that changes a frame
//! changes its key, and frames it does not touch keep theirs.
//!
//! Frames are kept in `frames/` of the cache directory as raw files, chosen over PNG so
//! reading one back costs no decoding: `frames/<first two hex digits>/<key in hex>.rgba`,
//! holding the 4 bytes `FCFR`, then the format version, width and height as little-endian
//! 32-bit numbers, then `width * height * 4` bytes of 8-bit RGBA, rows top to bottom. The
//! least recently used frames go first beyond a byte limit (`performance.render_cache_gb`).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use fosscut_model::{AssetKind, Id, ParamValue, Project};
use fosscut_render::Image;
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{Layer, LayerSource, RenderPlan};

use crate::cache::{temporary_path, touch};

/// The version of the frame files and of how keys are made from plans: bump it when either
/// changes. Changes to rendering itself bump [`fosscut_render::plan::CACHE_KEY_VERSION`].
pub const VERSION: u32 = 1;

/// The first bytes of a frame file.
const MAGIC: &[u8; 4] = b"FCFR";
/// Bytes before the pixels.
const HEADER: usize = 16;

/// What identifies one rendered frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameKey([u8; 32]);

impl FrameKey {
    /// The key of `plan` of `project` rendered at `size` with `transfer`. `describe` says,
    /// for each asset the frame reads (frame sources, LUTs and other asset parameters, and
    /// every font asset of the project, since text names fonts by family), what its file
    /// contains and which copy of it is decoded, for example its content identity and
    /// `proxy-half`. Asset ids belong to one project, so this is what tells two projects'
    /// `asset_0001` apart. `None` from `describe` means the content is unknown, and the
    /// frame gets no key and is not cached.
    pub fn new(
        project: &Project,
        plan: &RenderPlan,
        size: (u32, u32),
        transfer: TransferFunction,
        describe: &mut dyn FnMut(&Id) -> Option<String>,
    ) -> Option<FrameKey> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"fosscut.frame");
        hasher.update(&VERSION.to_le_bytes());
        let plan_bytes = plan.cache_key_bytes(size);
        hasher.update(&(plan_bytes.len() as u64).to_le_bytes());
        hasher.update(&plan_bytes);
        let transfer = format!("{transfer:?}");
        hasher.update(&(transfer.len() as u64).to_le_bytes());
        hasher.update(transfer.as_bytes());
        let mut assets = referenced_assets(plan);
        assets.extend(
            project
                .media
                .assets
                .iter()
                .filter(|a| a.kind == AssetKind::Font)
                .map(|a| a.id.clone()),
        );
        for asset in &assets {
            let id = asset.to_string();
            let description = describe(asset)?;
            for part in [id.as_str(), description.as_str()] {
                hasher.update(&(part.len() as u64).to_le_bytes());
                hasher.update(part.as_bytes());
            }
        }
        Some(FrameKey(*hasher.finalize().as_bytes()))
    }

    /// The key's 32 bytes.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key as 64 lowercase hex digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Every asset `plan` reads, at any depth: the frames layers show and mix in, and the
/// assets that effects and transitions take as parameters (LUTs).
pub fn referenced_assets(plan: &RenderPlan) -> BTreeSet<Id> {
    let mut assets = BTreeSet::new();
    for layer in &plan.layers {
        add_assets(layer, &mut assets);
    }
    assets
}

fn add_assets(layer: &Layer, assets: &mut BTreeSet<Id>) {
    let parameters = |values: &std::collections::BTreeMap<String, ParamValue>,
                      assets: &mut BTreeSet<Id>| {
        for value in values.values() {
            if let ParamValue::Asset(reference) = value {
                assets.insert(reference.asset.clone());
            }
        }
    };
    for effect in &layer.effects {
        parameters(&effect.values, assets);
    }
    if let Some((next, _)) = &layer.next_frame {
        assets.insert(next.asset.clone());
    }
    match &layer.source {
        LayerSource::Frame(source) => {
            assets.insert(source.asset.clone());
        }
        LayerSource::Transition(transition) => {
            parameters(&transition.params, assets);
            add_assets(&transition.outgoing, assets);
            add_assets(&transition.incoming, assets);
        }
        LayerSource::Nested(nested) => {
            for layer in &nested.layers {
                add_assets(layer, assets);
            }
        }
        LayerSource::Particles(particles) => {
            if let Some(sprite) = &particles.sprite {
                assets.insert(sprite.asset.clone());
            }
        }
        // A 3D object's model and printed pictures (ADR-0040).
        LayerSource::Model(object) => {
            assets.extend(object.content.model_asset().cloned());
            assets.extend(object.textures.iter().map(|s| s.asset.clone()));
        }
        LayerSource::Text(_)
        | LayerSource::Shape(_)
        | LayerSource::Caption(_)
        | LayerSource::Adjustment => {}
    }
}

/// Rendered frames on disk, the least recently used dropped first beyond a byte limit.
/// Several caches (in several processes) may share a folder: each keeps its own count of
/// what is there, read when it opens, and treats files that vanished as misses. Failing to
/// read or write a frame only costs rendering it again.
pub struct RenderCache {
    dir: PathBuf,
    limit: u64,
    bytes: u64,
    tick: u64,
    entries: HashMap<FrameKey, (u64, u64)>,
}

impl RenderCache {
    /// The frames kept in `frames/` of the cache directory `cache`, keeping at most `limit`
    /// bytes of them. What is there already counts towards the limit, the least recently
    /// used first.
    pub fn open(cache: &Path, limit: u64) -> RenderCache {
        let dir = cache.join("frames");
        let mut found: Vec<(std::time::SystemTime, FrameKey, u64)> = Vec::new();
        for shard in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            for file in std::fs::read_dir(shard.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let path = file.path();
                let Some(key) = key_of(&path) else {
                    continue;
                };
                let Ok(metadata) = file.metadata() else {
                    continue;
                };
                let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
                let used = metadata.accessed().unwrap_or(modified).max(modified);
                found.push((used, key, metadata.len()));
            }
        }
        found.sort();
        let mut cache = RenderCache {
            dir,
            limit,
            bytes: 0,
            tick: 0,
            entries: HashMap::new(),
        };
        for (_, key, bytes) in found {
            cache.tick += 1;
            cache.bytes += bytes;
            cache.entries.insert(key, (bytes, cache.tick));
        }
        cache.evict();
        cache
    }

    /// The folder frames are kept in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Bytes of frames kept.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Number of frames kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no frame is kept.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The byte limit.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Changes the byte limit, dropping the least recently used frames beyond it.
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
        self.evict();
    }

    /// True when the frame of `key` is kept.
    pub fn contains(&self, key: &FrameKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Where the frame of `key` is kept.
    pub fn path(&self, key: &FrameKey) -> PathBuf {
        let hex = key.hex();
        self.dir.join(&hex[..2]).join(format!("{hex}.rgba"))
    }

    /// The frame of `key`, marking it used; `None` when it is not kept or its file is
    /// damaged (which removes it).
    pub fn get(&mut self, key: &FrameKey) -> Option<Image> {
        let path = self.path(key);
        let bytes = std::fs::read(&path).ok();
        let image = bytes.as_deref().and_then(decode);
        let Some(image) = image else {
            if bytes.is_some() {
                let _ = std::fs::remove_file(&path);
            }
            self.forget(key);
            return None;
        };
        touch(&path);
        self.tick += 1;
        let size = (HEADER + image.rgba.len()) as u64;
        match self.entries.get_mut(key) {
            Some(entry) => entry.1 = self.tick,
            // Written by another cache sharing the folder.
            None => {
                self.entries.insert(*key, (size, self.tick));
                self.bytes += size;
                self.evict_except(Some(key));
            }
        }
        Some(image)
    }

    /// Keeps `image` as the frame of `key`, dropping the least recently used frames beyond
    /// the limit. Returns whether it was kept: a frame larger than the limit, or one that
    /// cannot be written, is not.
    pub fn put(&mut self, key: &FrameKey, image: &Image) -> bool {
        let size = (HEADER + image.rgba.len()) as u64;
        if size > self.limit || image.rgba.len() != (image.width * image.height * 4) as usize {
            return false;
        }
        let path = self.path(key);
        let written = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = temporary_path(&path);
            let mut bytes = Vec::with_capacity(size as usize);
            bytes.extend_from_slice(MAGIC);
            bytes.extend_from_slice(&VERSION.to_le_bytes());
            bytes.extend_from_slice(&image.width.to_le_bytes());
            bytes.extend_from_slice(&image.height.to_le_bytes());
            bytes.extend_from_slice(&image.rgba);
            if let Err(error) =
                std::fs::write(&temporary, &bytes).and_then(|()| std::fs::rename(&temporary, &path))
            {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
            Ok(())
        })();
        if written.is_err() {
            return false;
        }
        self.forget(key);
        self.tick += 1;
        self.entries.insert(*key, (size, self.tick));
        self.bytes += size;
        self.evict_except(Some(key));
        true
    }

    fn forget(&mut self, key: &FrameKey) {
        if let Some((size, _)) = self.entries.remove(key) {
            self.bytes -= size;
        }
    }

    fn evict(&mut self) {
        self.evict_except(None);
    }

    /// Removes the least recently used frames until the limit holds, never `keep`.
    fn evict_except(&mut self, keep: Option<&FrameKey>) {
        while self.bytes > self.limit {
            let oldest = self
                .entries
                .iter()
                .filter(|(key, _)| Some(*key) != keep)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                break;
            };
            let _ = std::fs::remove_file(self.path(&oldest));
            self.forget(&oldest);
        }
    }
}

/// The key a frame file is named by.
fn key_of(path: &Path) -> Option<FrameKey> {
    let name = path.file_name()?.to_str()?;
    let hex = name.strip_suffix(".rgba")?;
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(FrameKey(key))
}

/// The image in a frame file's bytes, if they are one of this version.
fn decode(bytes: &[u8]) -> Option<Image> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if bytes.get(..4)? != MAGIC || word(4)? != VERSION {
        return None;
    }
    let (width, height) = (word(8)?, word(12)?);
    let rgba = bytes.get(HEADER..)?;
    (rgba.len() == (width as usize) * (height as usize) * 4).then(|| Image {
        width,
        height,
        rgba: rgba.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fosscut-frames-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn key(n: u8) -> FrameKey {
        FrameKey([n; 32])
    }

    fn image(shade: u8) -> Image {
        Image {
            width: 4,
            height: 2,
            rgba: vec![shade; 32],
        }
    }

    /// Bytes a 4x2 frame takes.
    const FRAME: u64 = (HEADER + 32) as u64;

    #[test]
    fn frames_round_trip_and_survive_reopening() {
        let cache = folder("round-trip");
        let mut frames = RenderCache::open(&cache, 10_000);
        assert!(frames.is_empty());
        assert_eq!(frames.get(&key(1)), None);
        assert!(frames.put(&key(1), &image(9)));
        assert_eq!(frames.get(&key(1)), Some(image(9)));
        assert_eq!((frames.len(), frames.bytes()), (1, FRAME));
        let path = frames.path(&key(1));
        assert!(path.starts_with(cache.join("frames").join("01")));
        assert_eq!(key_of(&path), Some(key(1)));

        let mut again = RenderCache::open(&cache, 10_000);
        assert_eq!(again.bytes(), FRAME);
        assert_eq!(again.get(&key(1)), Some(image(9)), "kept on disk");

        // A damaged file is a miss, and goes.
        std::fs::write(&path, b"FCFR").unwrap();
        assert_eq!(again.get(&key(1)), None);
        assert!(!path.exists());
        assert_eq!(again.bytes(), 0);
        std::fs::remove_dir_all(&cache).unwrap();
    }

    #[test]
    fn the_least_recently_used_frames_go_beyond_the_limit() {
        let cache = folder("eviction");
        let mut frames = RenderCache::open(&cache, 3 * FRAME);
        for n in 1..=3 {
            assert!(frames.put(&key(n), &image(n)));
        }
        // Using frame 1 makes frame 2 the least recently used.
        assert!(frames.get(&key(1)).is_some());
        assert!(frames.put(&key(4), &image(4)));
        assert_eq!(frames.bytes(), 3 * FRAME);
        assert!(!frames.contains(&key(2)) && !frames.path(&key(2)).exists());
        for n in [1, 3, 4] {
            assert!(frames.contains(&key(n)), "frame {n}");
        }
        // Replacing a frame does not count it twice.
        assert!(frames.put(&key(4), &image(5)));
        assert_eq!(frames.bytes(), 3 * FRAME);
        assert_eq!(frames.get(&key(4)), Some(image(5)));

        frames.set_limit(FRAME);
        assert_eq!((frames.len(), frames.bytes()), (1, FRAME));
        assert!(frames.contains(&key(4)), "the most recently used stays");
        assert!(
            !frames.put(
                &key(6),
                &Image {
                    width: 8,
                    height: 8,
                    rgba: vec![0; 256],
                }
            ),
            "a frame larger than the limit is not kept"
        );
        // Opening with a smaller limit evicts what is there.
        assert!(RenderCache::open(&cache, 0).is_empty());
        assert!(!frames.path(&key(4)).exists());
        std::fs::remove_dir_all(&cache).unwrap();
    }
}
