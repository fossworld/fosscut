//! Agent eyes (ADR-0002, item 4): single frames and contact sheets of a timeline, rendered
//! by the same plan and renderer as an export, so what an agent checks is what it exports.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use fosscut_media::DecoderOptions;
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_model::{Id, Project, TimelineFormat};
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{LayerSource, RenderPlan, plan_frame};
use fosscut_render::{DecoderFrames, Image, Renderer};
use fosscut_time::{Flicks, TimeReport};
use serde::{Deserialize, Serialize};

use crate::EngineError;
use crate::analysis::subject::{Subjects, plan_requirements, timeline_requirements};
use crate::export::resolve;

/// Widest frame or tile rendered, in pixels.
pub const MAX_WIDTH: u32 = 4096;
/// Most tiles on one contact sheet.
pub const MAX_TILES: u32 = 100;
/// Pixels between contact sheet tiles.
const GAP: u32 = 4;
/// Contact sheet background, dark gray.
const BACKGROUND: [u8; 4] = [24, 24, 24, 255];

/// One rendered frame of a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Still {
    /// The frame shown: the frame containing the requested time.
    pub at: TimeReport,
    /// The frame as 8-bit RGBA.
    pub image: Image,
}

/// Where one frame sits on a contact sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    /// The frame shown in the tile.
    pub at: TimeReport,
    /// Left edge in the sheet, in pixels.
    pub x: u32,
    /// Top edge in the sheet, in pixels.
    pub y: u32,
    /// Tile width in pixels.
    pub width: u32,
    /// Tile height in pixels.
    pub height: u32,
}

/// Frames spread evenly over a timeline, tiled left to right and top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactSheet {
    /// The sheet as 8-bit RGBA.
    pub image: Image,
    /// Tiles per row.
    pub columns: u32,
    /// Every tile, in time order.
    pub tiles: Vec<Tile>,
}

/// Renders the frame of `timeline` that contains `time`, `width` pixels wide (the
/// timeline's own size when `None`), with the timeline's aspect ratio. Analyses that clips'
/// effects read come from the cache in [`crate::cache_dir`].
pub fn frame(
    project: &Project,
    bundle: &Path,
    timeline: &Id,
    time: Flicks,
    width: Option<u32>,
    renderer: &mut Renderer,
) -> Result<Still, EngineError> {
    frame_in(
        project,
        bundle,
        timeline,
        time,
        width,
        &crate::cache_dir(),
        renderer,
    )
}

/// [`frame`] with the analyses that `key.subject` and the body effects read taken from the
/// cache directory `cache`. A clip whose analysis is missing fails with
/// `fosscut.analysis.missing` before anything renders (ADR-0033).
pub fn frame_in(
    project: &Project,
    bundle: &Path,
    timeline: &Id,
    time: Flicks,
    width: Option<u32>,
    cache: &Path,
    renderer: &mut Renderer,
) -> Result<Still, EngineError> {
    let mut frames = Frames::new(project, bundle, timeline, cache)?;
    if time < Flicks::ZERO || time >= frames.duration {
        return Err(EngineError::TimeOutOfRange {
            time,
            duration: frames.duration,
        });
    }
    let size = frames.size(width.unwrap_or(frames.format.width))?;
    frames.render(time, size, renderer)
}

/// Renders `count` frames spread evenly over `timeline`, each `width` pixels wide, and tiles
/// them into one image. Tile `i` shows the middle of the `i`-th of `count` equal parts.
/// Analyses that clips' effects read come from the cache in [`crate::cache_dir`].
pub fn contact_sheet(
    project: &Project,
    bundle: &Path,
    timeline: &Id,
    count: u32,
    width: u32,
    renderer: &mut Renderer,
) -> Result<ContactSheet, EngineError> {
    contact_sheet_in(
        project,
        bundle,
        timeline,
        count,
        width,
        &crate::cache_dir(),
        renderer,
    )
}

/// [`contact_sheet`] with analyses from the cache directory `cache`. Every analysis the
/// timeline's effects need is checked before the first tile renders.
pub fn contact_sheet_in(
    project: &Project,
    bundle: &Path,
    timeline: &Id,
    count: u32,
    width: u32,
    cache: &Path,
    renderer: &mut Renderer,
) -> Result<ContactSheet, EngineError> {
    if !(1..=MAX_TILES).contains(&count) {
        return Err(EngineError::InvalidArguments {
            message: format!("a contact sheet has 1 to {MAX_TILES} tiles, not {count}"),
        });
    }
    let mut frames = Frames::new(project, bundle, timeline, cache)?;
    frames.media.require_subjects(
        project,
        bundle,
        &timeline_requirements(project, timeline, None),
    )?;
    let (tile_width, tile_height) = frames.size(width)?;
    let columns = sheet_columns(count, tile_width, tile_height);
    let rows = count.div_ceil(columns);
    let sheet_width = columns * tile_width + (columns - 1) * GAP;
    let sheet_height = rows * tile_height + (rows - 1) * GAP;
    let mut rgba = BACKGROUND.repeat((sheet_width * sheet_height) as usize);
    let mut tiles = Vec::with_capacity(count as usize);
    let duration = i128::from(frames.duration.get());
    for index in 0..count {
        let middle = duration * i128::from(2 * index + 1) / i128::from(2 * count);
        // Below the duration, which fits in i64.
        let time = Flicks::new(i64::try_from(middle).unwrap_or(i64::MAX));
        let still = frames.render(time, (tile_width, tile_height), renderer)?;
        let (x, y) = (
            (index % columns) * (tile_width + GAP),
            (index / columns) * (tile_height + GAP),
        );
        let row_bytes = (tile_width * 4) as usize;
        for (row, line) in still.image.rgba.chunks(row_bytes).enumerate() {
            let start = (((y as usize + row) * sheet_width as usize) + x as usize) * 4;
            rgba[start..start + row_bytes].copy_from_slice(line);
        }
        tiles.push(Tile {
            at: still.at,
            x,
            y,
            width: tile_width,
            height: tile_height,
        });
    }
    Ok(ContactSheet {
        image: Image {
            width: sheet_width,
            height: sheet_height,
            rgba,
        },
        columns,
        tiles,
    })
}

/// Encodes `image` as an 8-bit RGBA PNG.
pub fn png(image: &Image) -> Result<Vec<u8>, EngineError> {
    let failed = |e: png::EncodingError| EngineError::Failed {
        code: "fosscut.internal".into(),
        message: format!("cannot encode PNG: {e}"),
    };
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(&image.rgba).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(bytes)
}

/// Columns that make the sheet about as wide as it is tall.
fn sheet_columns(count: u32, tile_width: u32, tile_height: u32) -> u32 {
    let columns = (f64::from(count) * f64::from(tile_height) / f64::from(tile_width))
        .sqrt()
        .ceil() as u32;
    columns.clamp(1, count)
}

/// Decoders for the media a timeline shows, each opened on first use and kept, with the
/// cached analyses subject effects read. Assets never change once imported (ADR-0004), so
/// decoders stay valid across edits.
#[derive(Default)]
pub(crate) struct Media {
    decoders: DecoderFrames,
    opened: HashSet<Id>,
    /// The cache directory analyses are read from; [`crate::cache_dir`] when `None`.
    cache: Option<PathBuf>,
    subjects: Subjects,
}

impl Media {
    /// Opens the media of every frame layer of `plan` not opened yet, and loads the LUTs
    /// its effects use.
    pub(crate) fn open_for(
        &mut self,
        project: &Project,
        bundle: &Path,
        plan: &RenderPlan,
    ) -> Result<(), EngineError> {
        // Transitions and compound clips hold layers of their own.
        let layers = plan.layers.iter().flat_map(|layer| layer.flattened());
        for layer in layers {
            for effect in &layer.effects {
                if let Some(asset) = effect.asset("lut")
                    && !self.decoders.has_lut(asset)
                {
                    let lut = load_lut(project, bundle, asset)?;
                    self.decoders.insert_lut(asset.clone(), lut);
                }
            }
            if let LayerSource::Text(text) = &layer.source {
                for family in text.content.fonts() {
                    if !self.decoders.has_font(family)
                        && let Some((families, data)) = load_font(project, bundle, family)?
                    {
                        self.decoders.insert_font(&families, data);
                    }
                }
            }
            if let LayerSource::Frame(source) = &layer.source
                && !self.opened.contains(&source.asset)
            {
                let path = resolve(project, bundle, &source.asset)?;
                let decoder = FfmpegVideoDecoder::open(&path, DecoderOptions::default())?;
                self.decoders
                    .insert(source.asset.clone(), Box::new(decoder));
                self.opened.insert(source.asset.clone());
            }
        }
        self.require_subjects(project, bundle, &plan_requirements(project, plan))
    }

    /// Loads the cached analyses `requirements` name, failing with
    /// `fosscut.analysis.missing` for the first that is not cached.
    pub(crate) fn require_subjects(
        &mut self,
        project: &Project,
        bundle: &Path,
        requirements: &[crate::analysis::subject::Requirement],
    ) -> Result<(), EngineError> {
        if requirements.is_empty() {
            return Ok(());
        }
        let cache = self.cache.clone().unwrap_or_else(crate::cache_dir);
        self.subjects
            .require(project, bundle, &cache, requirements, &mut self.decoders)
    }

    /// The decoders, as the renderer reads frames.
    pub(crate) fn frames(&mut self) -> &mut DecoderFrames {
        &mut self.decoders
    }
}

/// A font file's families and its data.
pub(crate) type FontFile = (Vec<String>, std::sync::Arc<[u8]>);

/// The families and data of the font asset that has `family`; `None` for the bundled font
/// or a family no font asset has (validation refuses those before they get here).
pub(crate) fn load_font(
    project: &Project,
    bundle: &Path,
    family: &str,
) -> Result<Option<FontFile>, EngineError> {
    let Some(asset) = project.media.assets.iter().find(|a| {
        a.info
            .font
            .as_ref()
            .is_some_and(|f| f.families.iter().any(|name| name == family))
    }) else {
        return Ok(None);
    };
    let path = resolve(project, bundle, &asset.id)?;
    let data = std::fs::read(&path).map_err(|e| EngineError::InvalidArguments {
        message: format!("cannot read the font {}: {e}", path.display()),
    })?;
    let families = asset
        .info
        .font
        .as_ref()
        .map(|f| f.families.clone())
        .unwrap_or_default();
    Ok(Some((families, data.into())))
}

/// Reads and parses the `.cube` file of LUT asset `asset`.
pub(crate) fn load_lut(
    project: &Project,
    bundle: &Path,
    asset: &Id,
) -> Result<std::sync::Arc<fosscut_render::lut::Lut3d>, EngineError> {
    let path = resolve(project, bundle, asset)?;
    let text = std::fs::read_to_string(&path).map_err(|e| EngineError::InvalidArguments {
        message: format!("cannot read the LUT {}: {e}", path.display()),
    })?;
    let lut = fosscut_render::lut::Lut3d::parse_cube(&text)?;
    Ok(std::sync::Arc::new(lut))
}

/// Every LUT asset the effects of `timeline` use.
pub(crate) fn timeline_luts(project: &Project, timeline: &Id) -> Vec<Id> {
    let mut luts: Vec<Id> = project
        .timelines
        .get(timeline)
        .into_iter()
        .flat_map(|t| &t.tracks)
        .flat_map(|t| &t.clips)
        .flat_map(|c| &c.effects)
        .filter_map(|e| match e.params.get("lut") {
            Some(fosscut_model::ParamValue::Asset(reference)) => Some(reference.asset.clone()),
            _ => None,
        })
        .collect();
    luts.sort();
    luts.dedup();
    luts
}

/// Frames of one timeline, with each clip's media opened on first use.
struct Frames<'a> {
    project: &'a Project,
    bundle: &'a Path,
    timeline: &'a Id,
    format: TimelineFormat,
    duration: Flicks,
    media: Media,
}

impl<'a> Frames<'a> {
    fn new(
        project: &'a Project,
        bundle: &'a Path,
        timeline: &'a Id,
        cache: &Path,
    ) -> Result<Frames<'a>, EngineError> {
        let doc = project
            .timelines
            .get(timeline)
            .ok_or_else(|| EngineError::TimelineNotFound {
                timeline: timeline.to_string(),
            })?;
        let duration = doc.duration();
        if !duration.is_positive() {
            return Err(EngineError::EmptyTimeline);
        }
        Ok(Frames {
            project,
            bundle,
            timeline,
            format: doc.format.clone(),
            duration,
            media: Media {
                cache: Some(cache.to_path_buf()),
                ..Media::default()
            },
        })
    }

    /// The size of a frame `width` pixels wide, keeping the timeline's aspect ratio.
    fn size(&self, width: u32) -> Result<(u32, u32), EngineError> {
        if !(1..=MAX_WIDTH).contains(&width) {
            return Err(EngineError::InvalidArguments {
                message: format!("width must be 1 to {MAX_WIDTH} pixels, not {width}"),
            });
        }
        let height = (u64::from(width) * u64::from(self.format.height)
            + u64::from(self.format.width) / 2)
            / u64::from(self.format.width);
        Ok((width, height.clamp(1, u64::from(MAX_WIDTH)) as u32))
    }

    /// Renders the frame containing `time` at `size`.
    fn render(
        &mut self,
        time: Flicks,
        size: (u32, u32),
        renderer: &mut Renderer,
    ) -> Result<Still, EngineError> {
        let rate = self.format.rate;
        let start = rate.frames_to_flicks(rate.frame_at(time));
        let plan = plan_frame(self.project, self.timeline, start)?;
        self.media.open_for(self.project, self.bundle, &plan)?;
        let image =
            renderer.render_image(&plan, self.media.frames(), size, TransferFunction::Bt709)?;
        Ok(Still {
            at: TimeReport::new(start, rate),
            image,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_are_about_square() {
        assert_eq!(sheet_columns(12, 320, 180), 3, "landscape tiles");
        assert_eq!(sheet_columns(12, 180, 320), 5, "portrait tiles");
        assert_eq!(sheet_columns(1, 320, 180), 1);
        assert_eq!(
            sheet_columns(2, 10, 1000),
            2,
            "never more columns than tiles"
        );
    }

    #[test]
    fn png_round_trips() {
        let image = Image {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 128],
        };
        let bytes = png(&image).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(decoded, image.rgba);
    }
}
