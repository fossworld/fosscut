//! Agent eyes (ADR-0002, item 4): single frames and contact sheets of a timeline, rendered
//! by the same plan and renderer as an export, so what an agent checks is what it exports.

use std::collections::HashSet;
use std::path::Path;

use fosscut_media::DecoderOptions;
use fosscut_media::ffmpeg::FfmpegVideoDecoder;
use fosscut_model::{Id, Project, TimelineFormat};
use fosscut_render::color::TransferFunction;
use fosscut_render::plan::{LayerSource, RenderPlan, plan_frame};
use fosscut_render::{DecoderFrames, Image, Renderer};
use fosscut_time::{Flicks, TimeReport};
use serde::{Deserialize, Serialize};

use crate::EngineError;
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
/// timeline's own size when `None`), with the timeline's aspect ratio.
pub fn frame(
    project: &Project,
    bundle: &Path,
    timeline: &Id,
    time: Flicks,
    width: Option<u32>,
    renderer: &mut Renderer,
) -> Result<Still, EngineError> {
    let mut frames = Frames::new(project, bundle, timeline)?;
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
pub fn contact_sheet(
    project: &Project,
    bundle: &Path,
    timeline: &Id,
    count: u32,
    width: u32,
    renderer: &mut Renderer,
) -> Result<ContactSheet, EngineError> {
    if !(1..=MAX_TILES).contains(&count) {
        return Err(EngineError::InvalidArguments {
            message: format!("a contact sheet has 1 to {MAX_TILES} tiles, not {count}"),
        });
    }
    let mut frames = Frames::new(project, bundle, timeline)?;
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

/// Decoders for the media a timeline shows, each opened on first use and kept. Assets never
/// change once imported (ADR-0004), so decoders stay valid across edits.
#[derive(Default)]
pub(crate) struct Media {
    decoders: DecoderFrames,
    opened: HashSet<Id>,
}

impl Media {
    /// Opens the media of every frame layer of `plan` not opened yet.
    pub(crate) fn open_for(
        &mut self,
        project: &Project,
        bundle: &Path,
        plan: &RenderPlan,
    ) -> Result<(), EngineError> {
        for layer in &plan.layers {
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
        Ok(())
    }

    /// The decoders, as the renderer reads frames.
    pub(crate) fn frames(&mut self) -> &mut DecoderFrames {
        &mut self.decoders
    }
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
            media: Media::default(),
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
