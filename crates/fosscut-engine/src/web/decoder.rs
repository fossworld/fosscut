//! Frames of a web composition (ADR-0039): its page loaded once in a headless browser and
//! shown at each frame's time, captured as straight-alpha R'G'B'A frames the renderer
//! treats like any decoded video frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fosscut_media::{
    Acceleration, ColorInfo, MediaError, PixelFormat, Plane, VideoDecoder, VideoFrame,
    VideoStreamInfo,
};
use fosscut_media::{Matrix, Primaries, Range, Transfer};
use fosscut_time::Flicks;
use serde_json::{Value, json};

use super::browser::{Browser, Found};
use super::cdp::{ORIGIN, decode_base64};
use super::{Composition, WebError};

/// The script that gives the page FossCut's clock.
const CLOCK: &str = include_str!("clock.js");

/// How long a page may take to load.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// A web composition's page in its own headless browser.
pub struct WebDecoder {
    browser: Browser,
    session: String,
    url: String,
    composition: Composition,
    info: VideoStreamInfo,
    last: Option<VideoFrame>,
    /// The frame `next_frame` gives next.
    next: i64,
}

impl WebDecoder {
    /// Starts `browser` and loads the composition whose page is `entry` (its folder holds
    /// the files it loads), at the size and rate of `composition`.
    pub fn open(
        entry: &Path,
        composition: Composition,
        browser: &Found,
    ) -> Result<WebDecoder, WebError> {
        let folder = entry
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "index.html".into());
        let mut browser = Browser::start(browser)?;
        let failed = |e: super::cdp::CdpError| WebError::BrowserFailed { message: e.0 };
        let target = browser
            .cdp
            .call(None, "Target.createTarget", json!({ "url": "about:blank" }))
            .map_err(failed)?;
        let target = target["targetId"].as_str().unwrap_or_default().to_string();
        let attached = browser
            .cdp
            .call(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
            )
            .map_err(failed)?;
        let session = attached["sessionId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        browser.cdp.serve(folder);
        let page = Some(session.as_str());
        let setup = [
            ("Page.enable", json!({})),
            (
                "Fetch.enable",
                json!({ "patterns": [{ "urlPattern": "*", "requestStage": "Request" }] }),
            ),
            (
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": CLOCK }),
            ),
            (
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": composition.width,
                    "height": composition.height,
                    "deviceScaleFactor": 1,
                    "mobile": false,
                }),
            ),
            (
                "Emulation.setDefaultBackgroundColorOverride",
                json!({ "color": { "r": 0, "g": 0, "b": 0, "a": 0 } }),
            ),
        ];
        for (method, params) in setup {
            browser.cdp.call(page, method, params).map_err(failed)?;
        }
        let rate = composition.rate;
        let frames = composition.frames();
        let info = VideoStreamInfo {
            index: 0,
            codec: "html".into(),
            width: composition.width,
            height: composition.height,
            rate: Some(rate),
            time_base: (
                i32::try_from(rate.den()).unwrap_or(1),
                i32::try_from(rate.num()).unwrap_or(30),
            ),
            duration: Some(composition.duration),
            frame_count: u64::try_from(frames).ok(),
            rotation: 0,
            color: color(),
            source_pixel_format: "rgba".into(),
        };
        let mut decoder = WebDecoder {
            browser,
            session,
            url: format!("{ORIGIN}/{}", percent_encode(&name)),
            composition,
            info,
            last: None,
            next: 0,
        };
        decoder.load()?;
        Ok(decoder)
    }

    /// Loads (or reloads) the page and waits for its fonts and images.
    fn load(&mut self) -> Result<(), WebError> {
        let page = Some(self.session.as_str());
        let failed = |e: super::cdp::CdpError| WebError::PageFailed { message: e.0 };
        self.browser.cdp.clear_events();
        let navigated = self
            .browser
            .cdp
            .call(page, "Page.navigate", json!({ "url": self.url }))
            .map_err(failed)?;
        if let Some(problem) = navigated.get("errorText").and_then(Value::as_str) {
            return Err(WebError::PageFailed {
                message: format!("the page did not load: {problem}"),
            });
        }
        self.browser
            .cdp
            .wait_event(&self.session, "Page.loadEventFired", LOAD_TIMEOUT)
            .map_err(failed)?;
        self.evaluate("window.__fosscut.ready()")?;
        Ok(())
    }

    /// Runs `expression` in the page, awaiting a promise it returns, and gives its value.
    fn evaluate(&mut self, expression: &str) -> Result<Value, WebError> {
        let result = self
            .browser
            .cdp
            .call(
                Some(self.session.as_str()),
                "Runtime.evaluate",
                json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
            )
            .map_err(|e| WebError::PageFailed { message: e.0 })?;
        if let Some(details) = result.get("exceptionDetails") {
            let text = details["exception"]["description"]
                .as_str()
                .or_else(|| details["text"].as_str())
                .unwrap_or("an exception");
            return Err(WebError::PageFailed {
                message: format!("the page threw: {text}"),
            });
        }
        Ok(result["result"]["value"].clone())
    }

    /// Shows frame `index` and captures it.
    fn capture(&mut self, index: i64) -> Result<VideoFrame, WebError> {
        let rate = self.composition.rate;
        let show = format!(
            "window.__fosscut.show({index}, {}, {})",
            rate.num(),
            rate.den()
        );
        if self.evaluate(&show)? == json!("reload") {
            self.load()?;
            self.evaluate(&show)?;
        }
        let (width, height) = (self.composition.width, self.composition.height);
        let shot = self
            .browser
            .cdp
            .call(
                Some(self.session.as_str()),
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "clip": { "x": 0, "y": 0, "width": width, "height": height, "scale": 1 },
                    "captureBeyondViewport": false,
                    "fromSurface": true,
                    "optimizeForSpeed": true,
                }),
            )
            .map_err(|e| WebError::PageFailed { message: e.0 })?;
        let png = decode_base64(shot["data"].as_str().unwrap_or_default())
            .map_err(|e| WebError::PageFailed { message: e.0 })?;
        let rgba = rgba_from_png(&png, width, height)?;
        let start = rate.frames_to_flicks(index);
        Ok(VideoFrame {
            pts: start,
            duration: rate.frames_to_flicks(index + 1) - start,
            width,
            height,
            format: PixelFormat::Rgba,
            planes: vec![Plane {
                stride: width as usize * 4,
                rows: height as usize,
                data: Arc::from(rgba),
            }],
            color: color(),
        })
    }

    fn frame(&mut self, index: i64) -> Result<VideoFrame, MediaError> {
        if let Some(last) = &self.last
            && self.composition.rate.frame_at(last.pts) == index
        {
            return Ok(last.clone());
        }
        let frame = self.capture(index).map_err(|e| MediaError::DecodeFailed {
            message: e.to_string(),
        })?;
        self.last = Some(frame.clone());
        self.next = index + 1;
        Ok(frame)
    }
}

/// How web frames are tagged: R'G'B' in sRGB, as CSS colours are, so a colour written in a
/// page matches the same colour in a FossCut text or shape.
fn color() -> ColorInfo {
    ColorInfo {
        matrix: Matrix::Bt709,
        range: Range::Full,
        primaries: Primaries::Bt709,
        transfer: Transfer::Srgb,
    }
}

/// Decodes a screenshot into `width * height * 4` straight-alpha R'G'B'A bytes.
fn rgba_from_png(png: &[u8], width: u32, height: u32) -> Result<Vec<u8>, WebError> {
    let failed = |message: String| WebError::PageFailed { message };
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| failed(format!("unreadable screenshot: {e}")))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| failed("the screenshot is too large".into()))?;
    let mut buffer = vec![0u8; size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|e| failed(format!("unreadable screenshot: {e}")))?;
    if (info.width, info.height) != (width, height) {
        return Err(failed(format!(
            "the screenshot is {}x{}, not {width}x{height}",
            info.width, info.height
        )));
    }
    let pixels = buffer[..info.buffer_size()].to_vec();
    Ok(match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[v, a]| [*v, *v, *v, *a])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => {
            return Err(failed("the screenshot has an unexpected palette".into()));
        }
    })
}

/// `name` with characters that would break a URL path escaped.
fn percent_encode(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl VideoDecoder for WebDecoder {
    fn info(&self) -> &VideoStreamInfo {
        &self.info
    }

    fn acceleration(&self) -> Acceleration {
        Acceleration::Software
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, MediaError> {
        if self.next >= self.composition.frames() {
            return Ok(None);
        }
        self.frame(self.next).map(Some)
    }

    /// The frame shown at `time`; times past the end show the last frame, as a video's last
    /// frame holds.
    fn frame_at(&mut self, time: Flicks) -> Result<VideoFrame, MediaError> {
        let last = (self.composition.frames() - 1).max(0);
        let index = self.composition.rate.frame_at(time).clamp(0, last);
        self.frame(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_become_straight_rgba() {
        let mut png_bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            rgba_from_png(&png_bytes, 2, 1).unwrap(),
            vec![255, 0, 0, 255, 0, 0, 255, 255]
        );
        assert!(rgba_from_png(&png_bytes, 3, 1).is_err());
        assert!(rgba_from_png(b"not a png", 2, 1).is_err());
    }

    #[test]
    fn page_names_are_escaped() {
        assert_eq!(percent_encode("index.html"), "index.html");
        assert_eq!(percent_encode("my ad #1.html"), "my%20ad%20%231.html");
    }
}
