//! Recording from FFmpeg's input devices (libavdevice): `avfoundation` on macOS, `dshow`
//! and `gdigrab` on Windows, `v4l2`, `pulse`, `alsa` and `x11grab` on Linux, and `lavfi`
//! everywhere.
//!
//! Each input is read and decoded on its own thread, which converts pictures to RGBA at
//! the output size and sound to the output's sample rate and channels. The calling thread
//! places them on the output's timeline (repeating or dropping pictures to keep the
//! constant frame rate, filling gaps in the sound with silence) and encodes them with
//! [`FfmpegWriter`], so recordings get the same codecs, settings checks and probing as
//! exports.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use fosscut_time::{FLICKS_PER_SECOND, Flicks, Rate};
use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
use rsmpeg::avformat::{AVFormatContextInput, AVInputFormat, AVInputFormatRef};
use rsmpeg::avutil::{AVFrame, err2str};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;
use rsmpeg::swscale::SwsContext;

use super::audio::{normalize_layout, resample, resampler, samples_at};
use super::{FfmpegWriter, color_info, decode_failed, probe_output};
use crate::{
    AudioBuffer, AudioSettings, CaptureInput, Device, DeviceKind, Matrix, MediaError, OutputReport,
    OutputSettings, Range, Recording, RgbaFrame, VideoSettings, timebase,
};

/// How long a reader waits before asking a device that had nothing ready again.
const POLL: Duration = Duration::from_millis(5);

/// Gaps in a device's sound longer than this (a tenth of a second) become silence; shorter
/// timestamp jitter is ignored, so sound stays continuous.
const AUDIO_GAP_DIVISOR: i64 = 10;

/// Makes FFmpeg's input devices findable by name and keeps FFmpeg quiet. Called by every
/// entry point of this module.
fn register() {
    static ONCE: Once = Once::new();
    // SAFETY: `avdevice_register_all` only fills FFmpeg's static device tables; the `Once`
    // keeps two threads from doing it at the same time.
    ONCE.call_once(|| unsafe { ffi::avdevice_register_all() });
    super::quiet();
}

/// The capture input formats (FFmpeg input devices) of this FFmpeg build, for example
/// `avfoundation` and `lavfi`.
pub fn input_formats() -> Vec<String> {
    register();
    type Next = unsafe extern "C" fn(*const ffi::AVInputFormat) -> *const ffi::AVInputFormat;
    let iterators: [Next; 2] = [
        ffi::av_input_video_device_next,
        ffi::av_input_audio_device_next,
    ];
    let mut names: Vec<String> = Vec::new();
    for next in iterators {
        let mut cursor: *const ffi::AVInputFormat = ptr::null();
        loop {
            // SAFETY: the iterator takes null or a format it returned before, and returns
            // the next registered device format (static) or null.
            cursor = unsafe { next(cursor) };
            if cursor.is_null() {
                break;
            }
            // SAFETY: `cursor` is a registered format; its name is null or a static
            // NUL-terminated string.
            let name = unsafe { text((*cursor).name) };
            for name in name.split(',').filter(|n| !n.is_empty()) {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

/// The capture devices on this machine: cameras, screens and microphones of the input
/// formats this FFmpeg build has (`avfoundation`; `dshow` and `gdigrab`; `v4l2`, `pulse`,
/// or `alsa` when PulseAudio lists nothing, and `x11grab` for `$DISPLAY`). Empty when there
/// are none or a format cannot list them; finding no devices is not an error.
pub fn list_devices() -> Result<Vec<Device>, MediaError> {
    register();
    let mut devices = Vec::new();
    #[cfg(target_os = "macos")]
    if let Some(format) = AVInputFormat::find(c"avfoundation") {
        devices.extend(avfoundation::devices(&format));
    }
    if let Some(format) = AVInputFormat::find(c"dshow") {
        for listed in listed_sources(&format) {
            // DirectShow inputs name the media type: `video=<device>` or `audio=<device>`.
            let kinds = [
                (DeviceKind::Camera, "video", listed.video),
                (DeviceKind::Microphone, "audio", listed.audio),
            ];
            for (kind, prefix, _) in kinds.into_iter().filter(|(_, _, has)| *has) {
                devices.push(Device {
                    kind,
                    name: listed.shown().to_string(),
                    format: "dshow".into(),
                    input: format!("{prefix}={}", listed.name),
                });
            }
        }
    }
    if AVInputFormat::find(c"gdigrab").is_some() {
        devices.push(Device {
            kind: DeviceKind::Screen,
            name: "Desktop".into(),
            format: "gdigrab".into(),
            input: "desktop".into(),
        });
    }
    if let Some(format) = AVInputFormat::find(c"v4l2") {
        devices.extend(named_devices(&format, "v4l2", DeviceKind::Camera));
    }
    let mut sound = Vec::new();
    if let Some(format) = AVInputFormat::find(c"pulse") {
        sound = named_devices(&format, "pulse", DeviceKind::Microphone);
    }
    if sound.is_empty()
        && let Some(format) = AVInputFormat::find(c"alsa")
    {
        sound = named_devices(&format, "alsa", DeviceKind::Microphone);
    }
    devices.extend(sound);
    if AVInputFormat::find(c"x11grab").is_some()
        && let Some(display) = std::env::var("DISPLAY").ok().filter(|d| !d.is_empty())
    {
        devices.push(Device {
            kind: DeviceKind::Screen,
            name: format!("Screen {display}"),
            format: "x11grab".into(),
            input: display,
        });
    }
    Ok(devices)
}

/// A device as `avdevice_list_input_sources` reports it.
struct Listed {
    name: String,
    description: String,
    video: bool,
    audio: bool,
}

impl Listed {
    /// The description when there is one, otherwise the name.
    fn shown(&self) -> &str {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }
}

/// Devices of a format whose input is the listed device name, of `kind` unless the listing
/// says otherwise.
fn named_devices(format: &AVInputFormat, name: &str, kind: DeviceKind) -> Vec<Device> {
    listed_sources(format)
        .into_iter()
        .map(|listed| Device {
            kind: match (listed.video, listed.audio) {
                (true, false) => DeviceKind::Camera,
                (false, true) => DeviceKind::Microphone,
                _ => kind,
            },
            name: listed.shown().to_string(),
            format: name.to_string(),
            input: listed.name,
        })
        .collect()
}

/// What `avdevice_list_input_sources` finds for a format; empty when the format cannot
/// list its devices.
fn listed_sources(format: &AVInputFormat) -> Vec<Listed> {
    let mut list: *mut ffi::AVDeviceInfoList = ptr::null_mut();
    // SAFETY: `format` is a registered input format; FFmpeg allocates `list`, which is
    // freed below, and takes no options here.
    let count = unsafe {
        ffi::avdevice_list_input_sources(format.as_ptr(), ptr::null(), ptr::null_mut(), &mut list)
    };
    let mut found = Vec::new();
    if count >= 0 && !list.is_null() {
        // SAFETY: on success `list` points to a list of `nb_devices` entries that stays
        // valid until it is freed.
        let (entries, count) = unsafe { ((*list).devices, (*list).nb_devices) };
        for index in 0..usize::try_from(count).unwrap_or(0) {
            // SAFETY: `index < nb_devices`, so the entry pointer may be read.
            let info = unsafe { *entries.add(index) };
            if info.is_null() {
                continue;
            }
            // SAFETY: a listed entry holds null or NUL-terminated strings, and
            // `media_types` holds `nb_media_types` entries or is null.
            let (name, description, types) = unsafe {
                let info = &*info;
                let types: &[ffi::AVMediaType] = match usize::try_from(info.nb_media_types) {
                    Ok(n) if n > 0 && !info.media_types.is_null() => {
                        std::slice::from_raw_parts(info.media_types, n)
                    }
                    _ => &[],
                };
                (
                    text(info.device_name),
                    text(info.device_description),
                    types.to_vec(),
                )
            };
            if name.is_empty() {
                continue;
            }
            found.push(Listed {
                name,
                description,
                video: types.contains(&ffi::AVMEDIA_TYPE_VIDEO),
                audio: types.contains(&ffi::AVMEDIA_TYPE_AUDIO),
            });
        }
    }
    // SAFETY: frees what `avdevice_list_input_sources` allocated; null is accepted.
    unsafe { ffi::avdevice_free_list_devices(&mut list) };
    found
}

/// A C string as an owned string; empty for null.
///
/// # Safety
/// `pointer` is null or points to a NUL-terminated string that outlives the call.
unsafe fn text(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: the caller guarantees a NUL-terminated string.
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

/// AVFoundation cannot list devices through `avdevice_list_input_sources`; asked to with
/// its `list_devices` option, it logs them and fails to open. This collects those log
/// lines on the listing thread only, passing every other message to FFmpeg's own logger.
#[cfg(target_os = "macos")]
mod avfoundation {
    use std::cell::RefCell;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::{Mutex, PoisonError};

    use rsmpeg::avformat::{AVFormatContextInput, AVInputFormat};
    use rsmpeg::avutil::AVDictionary;
    use rsmpeg::ffi;

    use crate::Device;

    thread_local! {
        static LINES: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    /// One listing at a time, since FFmpeg's log callback is global.
    static LISTING: Mutex<()> = Mutex::new(());

    /// The cameras, screens and microphones AVFoundation reports.
    pub(super) fn devices(format: &AVInputFormat) -> Vec<Device> {
        let _listing = LISTING.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = LINES.try_with(|lines| *lines.borrow_mut() = Some(Vec::new()));
        // SAFETY: `collect` may be called from any thread at any time (see there).
        unsafe { ffi::av_log_set_callback(Some(collect)) };
        let mut options = Some(AVDictionary::new(c"list_devices", c"true", 0));
        // Opening fails on purpose once the devices are logged.
        let _ = AVFormatContextInput::builder()
            .url(c"")
            .format(format)
            .options(&mut options)
            .open();
        // SAFETY: puts FFmpeg's own logger back.
        unsafe { ffi::av_log_set_callback(Some(ffi::av_log_default_callback)) };
        let lines = LINES
            .try_with(|lines| lines.borrow_mut().take())
            .ok()
            .flatten()
            .unwrap_or_default();
        super::parse_avfoundation(&lines)
    }

    /// FFmpeg log callback: collects lines while this thread lists devices, otherwise
    /// logs as FFmpeg would. It never panics, since it is called from C.
    unsafe extern "C" fn collect(
        context: *mut c_void,
        level: c_int,
        format: *const c_char,
        arguments: ffi::va_list,
    ) {
        let collecting = LINES
            .try_with(|lines| lines.try_borrow().is_ok_and(|lines| lines.is_some()))
            .unwrap_or(false);
        if !collecting {
            // SAFETY: passes FFmpeg's own message and arguments on unchanged.
            unsafe { ffi::av_log_default_callback(context, level, format, arguments) };
            return;
        }
        let mut line = [0 as c_char; 1024];
        let mut print_prefix: c_int = 0;
        // SAFETY: formats FFmpeg's own message and arguments into `line`, at most
        // `line.len()` bytes including the terminating NUL; a zero `print_prefix` leaves
        // out the `[context @ address]` prefix.
        unsafe {
            ffi::av_log_format_line2(
                context,
                level,
                format,
                arguments,
                line.as_mut_ptr(),
                line.len() as c_int,
                &mut print_prefix,
            );
        }
        // SAFETY: `av_log_format_line2` NUL-terminates the line inside the buffer.
        let text = unsafe { CStr::from_ptr(line.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let _ = LINES.try_with(|lines| {
            if let Ok(mut lines) = lines.try_borrow_mut()
                && let Some(lines) = lines.as_mut()
            {
                lines.push(text);
            }
        });
    }
}

/// Devices from AVFoundation's listing: a header for video devices (cameras, then screens
/// named `Capture screen N`), a header for audio devices, and each device as
/// `[index] name`, possibly after a `[context @ address]` prefix.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_avfoundation(lines: &[String]) -> Vec<Device> {
    let mut devices = Vec::new();
    let mut audio = None;
    for line in lines {
        let mut line = line.trim();
        if line.starts_with('[')
            && let Some((context, rest)) = line.split_once("] ")
            && context.contains(" @ ")
        {
            line = rest.trim();
        }
        if line.ends_with("video devices:") {
            audio = Some(false);
            continue;
        }
        if line.ends_with("audio devices:") {
            audio = Some(true);
            continue;
        }
        let Some(audio) = audio else { continue };
        let Some((index, name)) = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once("] "))
        else {
            continue;
        };
        let (Ok(index), name) = (index.parse::<u32>(), name.trim()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let (kind, input) = if audio {
            (DeviceKind::Microphone, format!(":{index}"))
        } else if name.starts_with("Capture screen") {
            (DeviceKind::Screen, format!("{index}:"))
        } else {
            (DeviceKind::Camera, format!("{index}:"))
        };
        devices.push(Device {
            kind,
            name: name.to_string(),
            format: "avfoundation".into(),
            input,
        });
    }
    devices
}

/// What the reader threads and FFmpeg's blocking calls watch to end a recording: the
/// caller's `stop`, and `done` once the recording is complete or has failed.
struct Control<'a> {
    stop: &'a AtomicBool,
    done: AtomicBool,
}

impl Control<'_> {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn ended(&self) -> bool {
        self.stopped() || self.done.load(Ordering::Relaxed)
    }
}

/// FFmpeg interrupt callback: non-zero ends the blocking calls of an input.
unsafe extern "C" fn interrupted(opaque: *mut c_void) -> c_int {
    // SAFETY: `opaque` is the `Control` given to `open_device`, which outlives every
    // input opened with it (inputs are closed before `record` returns).
    let control = unsafe { &*opaque.cast::<Control<'_>>() };
    c_int::from(control.ended())
}

/// Sets `done` when dropped, so reader threads end however the calling thread leaves.
struct Finish<'a>(&'a AtomicBool);

impl Drop for Finish<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// `input (format)`, for messages.
fn describe(source: &CaptureInput) -> String {
    format!("{} ({})", source.input, source.format)
}

fn input_format(name: &str) -> Result<AVInputFormatRef<'static>, MediaError> {
    let unavailable = || MediaError::InputFormatUnavailable {
        format: name.to_string(),
        available: input_formats(),
    };
    let c_name = CString::new(name).map_err(|_| unavailable())?;
    if name.is_empty() {
        return Err(unavailable());
    }
    AVInputFormat::find(&c_name).ok_or_else(unavailable)
}

/// Opens a device input that does not block (FFmpeg's command-line tool opens devices
/// the same way) and whose blocking calls end when `control` does.
fn open_device(
    source: &CaptureInput,
    control: &Control<'_>,
) -> Result<AVFormatContextInput, MediaError> {
    let format = input_format(&source.format)?;
    let unavailable = |message: String| MediaError::DeviceUnavailable {
        format: source.format.clone(),
        input: source.input.clone(),
        message,
    };
    let failed = |code: c_int| {
        if control.stopped() {
            return stopped_early(source);
        }
        let mut message = err2str(code).unwrap_or_else(|| format!("FFmpeg error {code}"));
        if source.format == "avfoundation" {
            message.push_str(
                "; check that the device exists and that this app may use the camera, \
                 microphone or screen (System Settings, Privacy & Security)",
            );
        }
        unavailable(message)
    };
    let url = CString::new(source.input.as_str())
        .map_err(|_| unavailable("the input contains a NUL byte".into()))?;
    let mut options: *mut ffi::AVDictionary = ptr::null_mut();
    for (key, value) in &source.options {
        let (Ok(key_c), Ok(value_c)) = (CString::new(key.as_str()), CString::new(value.as_str()))
        else {
            // SAFETY: frees the dictionary built so far, which nothing else refers to.
            unsafe { ffi::av_dict_free(&mut options) };
            return Err(MediaError::Unsupported {
                message: format!("the input option {key:?} contains a NUL byte"),
            });
        };
        // SAFETY: `av_dict_set` copies both strings into the dictionary `options` owns.
        unsafe { ffi::av_dict_set(&mut options, key_c.as_ptr(), value_c.as_ptr(), 0) };
    }
    // SAFETY: allocates a context that `avformat_open_input` takes over below.
    let mut context = unsafe { ffi::avformat_alloc_context() };
    if context.is_null() {
        // SAFETY: as above.
        unsafe { ffi::av_dict_free(&mut options) };
        return Err(unavailable("out of memory".into()));
    }
    // SAFETY: the context was just allocated and nothing else refers to it. The interrupt
    // callback reads `control`, which outlives the input (see `interrupted`).
    unsafe {
        (*context).flags |= ffi::AVFMT_FLAG_NONBLOCK as c_int;
        // Devices report their frame rate; counting frames to guess it would only delay
        // the start of the recording.
        (*context).fps_probe_size = 0;
        (*context).interrupt_callback = ffi::AVIOInterruptCB {
            callback: Some(interrupted),
            opaque: ptr::from_ref(control).cast_mut().cast::<c_void>(),
        };
    }
    // SAFETY: every pointer is valid. On failure `avformat_open_input` frees the context
    // and sets it to null; either way it replaces `options` with the options it did not
    // use, which are read and freed below.
    let code = unsafe {
        ffi::avformat_open_input(&mut context, url.as_ptr(), format.as_ptr(), &mut options)
    };
    let unused = dictionary_keys(options);
    // SAFETY: frees the leftover dictionary, which nothing else refers to; null is fine.
    unsafe { ffi::av_dict_free(&mut options) };
    if code < 0 {
        return Err(failed(code));
    }
    let Some(context) = NonNull::new(context) else {
        return Err(unavailable("FFmpeg returned no input".into()));
    };
    // SAFETY: `avformat_open_input` succeeded, so `context` is an open input, which the
    // wrapper closes when it is dropped.
    let mut input = unsafe { AVFormatContextInput::from_raw(context) };
    if let Some(key) = unused.first() {
        return Err(MediaError::Unsupported {
            message: format!("the {} input has no option {key}", source.format),
        });
    }
    // SAFETY: the input is open; no per-stream options are passed.
    let code = unsafe { ffi::avformat_find_stream_info(input.as_mut_ptr(), ptr::null_mut()) };
    if code < 0 {
        return Err(failed(code));
    }
    Ok(input)
}

/// The keys of an FFmpeg dictionary (null for an empty one).
fn dictionary_keys(dictionary: *const ffi::AVDictionary) -> Vec<String> {
    let mut keys = Vec::new();
    let mut entry: *const ffi::AVDictionaryEntry = ptr::null();
    loop {
        // SAFETY: `av_dict_iterate` takes null or a dictionary and null or its previous
        // entry, and returns an entry the dictionary owns or null.
        entry = unsafe { ffi::av_dict_iterate(dictionary, entry) };
        if entry.is_null() {
            return keys;
        }
        // SAFETY: dictionary entries have NUL-terminated keys.
        keys.push(unsafe { text((*entry).key) });
    }
}

fn stopped_early(source: &CaptureInput) -> MediaError {
    MediaError::NothingRecorded {
        message: format!(
            "the recording was stopped before {} delivered any media",
            describe(source)
        ),
    }
}

/// Opens the decoder of stream `index`.
fn open_decoder(
    input: &AVFormatContextInput,
    index: usize,
    codec: &AVCodec,
) -> Result<AVCodecContext, MediaError> {
    let stream = &input.streams()[index];
    let mut decoder = AVCodecContext::new(codec);
    decoder
        .apply_codecpar(&stream.codecpar())
        .map_err(|e| decode_failed(&e))?;
    decoder.set_pkt_timebase(stream.time_base);
    decoder
        .open(None)
        .map_err(|e| MediaError::DecoderUnavailable {
            codec: format!("{} ({e})", codec.name().to_string_lossy()),
        })?;
    Ok(decoder)
}

/// The best stream of `kind` and its decoder, if the input has one.
fn best_stream(
    input: &AVFormatContextInput,
    kind: ffi::AVMediaType,
) -> Result<Option<(usize, AVCodecContext)>, MediaError> {
    let found = input
        .find_best_stream(kind)
        .map_err(|e| MediaError::DecoderUnavailable {
            codec: format!("the device's stream ({e})"),
        })?;
    match found {
        Some((index, codec)) => Ok(Some((index, open_decoder(input, index, &codec)?))),
        None => Ok(None),
    }
}

/// The time of a decoded frame in flicks, from its stream's time base.
fn frame_time(frame: &AVFrame, (num, den): (i32, i32)) -> Option<Flicks> {
    let ticks = [frame.best_effort_timestamp, frame.pts]
        .into_iter()
        .find(|t| *t != ffi::AV_NOPTS_VALUE)?;
    Some(timebase::to_flicks(ticks, num, den))
}

/// Media from one input, timed from that input's first decoded frame.
enum Captured {
    /// The input's first media arrived at this moment.
    Started { input: usize, at: Instant },
    /// A picture at the output size.
    Video {
        input: usize,
        time: Flicks,
        frame: RgbaFrame,
    },
    /// Interleaved samples at the output's rate and channels.
    Audio {
        input: usize,
        time: Flicks,
        samples: Vec<f32>,
    },
    /// Reading the input failed.
    Failed(MediaError),
}

/// Converts decoded pictures of any format and size to RGBA at the output size, scaled to
/// fit without distortion and centred on black.
struct Fitter {
    width: u32,
    height: u32,
    scaler: Option<(Shape, SwsContext)>,
}

/// What a scaler was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shape {
    width: i32,
    height: i32,
    format: i32,
    fitted: (u32, u32),
    colors: Option<(c_int, bool)>,
}

impl Fitter {
    fn convert(&mut self, frame: &AVFrame) -> Result<RgbaFrame, MediaError> {
        if frame.width <= 0 || frame.height <= 0 {
            return Err(MediaError::DecodeFailed {
                message: "the device delivered an empty picture".into(),
            });
        }
        let aspect = frame.sample_aspect_ratio;
        let (display_width, display_height) = if aspect.num > 0 && aspect.den > 0 {
            (
                frame.width as u64 * aspect.num as u64,
                frame.height as u64 * aspect.den as u64,
            )
        } else {
            (frame.width as u64, frame.height as u64)
        };
        let fitted = fit(display_width, display_height, self.width, self.height);
        let shape = Shape {
            width: frame.width,
            height: frame.height,
            format: frame.format,
            fitted,
            colors: source_colors(frame),
        };
        let scaler = match &mut self.scaler {
            Some((made_for, scaler)) if *made_for == shape => scaler,
            slot => {
                let scaler = SwsContext::get_context(
                    frame.width,
                    frame.height,
                    frame.format,
                    fitted.0 as i32,
                    fitted.1 as i32,
                    ffi::AV_PIX_FMT_RGBA,
                    ffi::SWS_BICUBIC,
                    None,
                    None,
                    None,
                )
                .ok_or_else(|| MediaError::DecodeFailed {
                    message: format!(
                        "cannot convert {} pictures to RGBA",
                        super::pixel_format_name(frame.format)
                    ),
                })?;
                if let Some((table, full_range)) = shape.colors {
                    set_source_colors(&scaler, table, full_range);
                }
                &mut slot.insert((shape, scaler)).1
            }
        };
        let mut scaled = AVFrame::new();
        scaled.set_format(ffi::AV_PIX_FMT_RGBA);
        scaled.set_width(fitted.0 as i32);
        scaled.set_height(fitted.1 as i32);
        scaled.alloc_buffer().map_err(|e| decode_failed(&e))?;
        scaler
            .scale_frame(frame, 0, frame.height, &mut scaled)
            .map_err(|e| decode_failed(&e))?;

        let (width, height) = (self.width as usize, self.height as usize);
        let (fitted_width, fitted_height) = (fitted.0 as usize, fitted.1 as usize);
        let (left, top) = ((width - fitted_width) / 2, (height - fitted_height) / 2);
        let mut rgba = if (fitted_width, fitted_height) == (width, height) {
            vec![0; width * height * 4]
        } else {
            [0, 0, 0, 255].repeat(width * height)
        };
        let row_bytes = fitted_width * 4;
        for_each_row(&scaled, row_bytes, fitted_height, |y, row| {
            let start = ((top + y) * width + left) * 4;
            rgba[start..start + row_bytes].copy_from_slice(row);
        })?;
        Ok(RgbaFrame {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

/// Calls `each` with the first `row_bytes` bytes of each of the first `rows` rows of a
/// packed frame's only plane.
fn for_each_row(
    frame: &AVFrame,
    row_bytes: usize,
    rows: usize,
    mut each: impl FnMut(usize, &[u8]),
) -> Result<(), MediaError> {
    let source = frame.data[0];
    let stride = usize::try_from(frame.linesize[0]).unwrap_or(0);
    if source.is_null() || stride < row_bytes || frame.height < rows as i32 {
        return Err(MediaError::DecodeFailed {
            message: "a converted picture has an unexpected layout".into(),
        });
    }
    for y in 0..rows {
        // SAFETY: the frame was allocated for its format and size, so its plane holds at
        // least `height >= rows` rows of `linesize >= row_bytes` bytes, and it stays alive
        // during the call.
        let row = unsafe { std::slice::from_raw_parts(source.add(y * stride), row_bytes) };
        each(y, row);
    }
    Ok(())
}

/// The largest size with the aspect ratio of a `width` x `height` picture that fits in
/// `into_width` x `into_height`, rounded to whole pixels and at least 1 x 1.
fn fit(width: u64, height: u64, into_width: u32, into_height: u32) -> (u32, u32) {
    let (width, height) = (width.max(1), height.max(1));
    let (into_w, into_h) = (u64::from(into_width), u64::from(into_height));
    if width * into_h >= into_w * height {
        let fitted = (2 * into_w * height + width) / (2 * width);
        (into_width, fitted.clamp(1, into_h.max(1)) as u32)
    } else {
        let fitted = (2 * into_h * width + height) / (2 * height);
        (fitted.clamp(1, into_w.max(1)) as u32, into_height)
    }
}

/// The swscale coefficient table and whether the range is full for a Y'CbCr picture, or
/// `None` for RGB pictures, which need neither.
fn source_colors(frame: &AVFrame) -> Option<(c_int, bool)> {
    // SAFETY: `av_pix_fmt_desc_get` returns null or a pointer to a static descriptor.
    let descriptor = unsafe { ffi::av_pix_fmt_desc_get(frame.format) };
    // SAFETY: checked non-null; descriptors are static and immutable.
    let rgb = !descriptor.is_null()
        && unsafe { (*descriptor).flags } & u64::from(ffi::AV_PIX_FMT_FLAG_RGB) != 0;
    if rgb {
        return None;
    }
    let color = color_info(
        frame.colorspace,
        frame.color_range,
        frame.color_primaries,
        frame.color_trc,
    )
    .resolved(frame.height.max(0) as u32);
    let jpeg = [
        ffi::AV_PIX_FMT_YUVJ420P,
        ffi::AV_PIX_FMT_YUVJ422P,
        ffi::AV_PIX_FMT_YUVJ444P,
        ffi::AV_PIX_FMT_YUVJ440P,
        ffi::AV_PIX_FMT_YUVJ411P,
    ]
    .contains(&frame.format);
    let table = match color.matrix {
        Matrix::Bt709 => ffi::SWS_CS_ITU709,
        Matrix::Bt2020Ncl => ffi::SWS_CS_BT2020,
        _ => ffi::SWS_CS_ITU601,
    };
    Some((table as c_int, jpeg || color.range == Range::Full))
}

/// Makes swscale decode Y'CbCr with the source's coefficients and range into full-range
/// RGB.
fn set_source_colors(scaler: &SwsContext, table: c_int, full_range: bool) {
    // SAFETY: `sws_getCoefficients` returns pointers to static tables, and the context is
    // valid and owned by the caller for the duration of the call.
    unsafe {
        let source = ffi::sws_getCoefficients(table);
        let target = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709 as c_int);
        ffi::sws_setColorspaceDetails(
            scaler.as_ptr() as *mut _,
            source,
            c_int::from(full_range),
            target,
            1,
            0,
            1 << 16,
            1 << 16,
        );
    }
}

/// The video stream an input contributes.
struct VideoReader {
    index: usize,
    decoder: AVCodecContext,
    time_base: (i32, i32),
    frame_duration: Flicks,
    /// When the next frame is expected, for frames without timestamps.
    next: Flicks,
    fitter: Fitter,
}

/// The audio stream an input contributes.
struct AudioReader {
    index: usize,
    decoder: AVCodecContext,
    time_base: (i32, i32),
    sample_rate: u32,
    channels: u16,
    /// The resampler and the source format, rate and channel count it was made for.
    resampler: Option<((i32, i32, i32), SwrContext)>,
    /// When the next samples are expected, for frames without timestamps.
    next: Flicks,
}

/// Reads and decodes one input on its own thread.
struct Reader {
    number: usize,
    source: CaptureInput,
    input: AVFormatContextInput,
    video: Option<VideoReader>,
    audio: Option<AudioReader>,
    /// The time of the input's first decoded frame; media are timed from it.
    origin: Option<Flicks>,
}

impl Reader {
    fn run(mut self, control: &Control<'_>, sender: &Sender<Captured>) {
        if let Err(error) = self.read(control, sender) {
            let _ = sender.send(Captured::Failed(error));
        }
    }

    fn read(&mut self, control: &Control<'_>, sender: &Sender<Captured>) -> Result<(), MediaError> {
        let again = ffi::AVERROR(ffi::EAGAIN);
        while !control.ended() {
            let packet = match self.input.read_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(error) if error.raw_error() == Some(again) => {
                    std::thread::sleep(POLL);
                    continue;
                }
                Err(_) if control.ended() => break,
                Err(error) => {
                    return Err(MediaError::DeviceUnavailable {
                        format: self.source.format.clone(),
                        input: self.source.input.clone(),
                        message: error.to_string(),
                    });
                }
            };
            if !self.decode(Some(&packet), sender)? {
                return Ok(());
            }
        }
        self.decode(None, sender).map(|_| ())
    }

    /// Decodes a packet (or flushes every decoder with `None`) and sends what comes out.
    /// False when nobody receives any more.
    fn decode(
        &mut self,
        packet: Option<&AVPacket>,
        sender: &Sender<Captured>,
    ) -> Result<bool, MediaError> {
        let stream = packet.map(|p| p.stream_index as usize);
        let mut frames = Vec::new();
        if let Some(video) = &mut self.video
            && stream.is_none_or(|s| s == video.index)
        {
            decode_packet(&mut video.decoder, packet, &mut frames)?;
            for frame in frames.drain(..) {
                let time = frame_time(&frame, video.time_base).unwrap_or(video.next);
                video.next = time + video.frame_duration;
                let time = since_origin(&mut self.origin, self.number, time, sender);
                let frame = video.fitter.convert(&frame)?;
                let message = Captured::Video {
                    input: self.number,
                    time,
                    frame,
                };
                if sender.send(message).is_err() {
                    return Ok(false);
                }
            }
        }
        if let Some(audio) = &mut self.audio
            && stream.is_none_or(|s| s == audio.index)
        {
            decode_packet(&mut audio.decoder, packet, &mut frames)?;
            for mut frame in frames.drain(..) {
                normalize_layout(&mut frame);
                let time = frame_time(&frame, audio.time_base).unwrap_or(audio.next);
                let rate = i64::from(frame.sample_rate.max(1));
                audio.next =
                    time + Flicks::new(i64::from(frame.nb_samples) * FLICKS_PER_SECOND / rate);
                let shape = (frame.format, frame.sample_rate, frame.ch_layout.nb_channels);
                let swr = match &mut audio.resampler {
                    Some((made_for, swr)) if *made_for == shape => swr,
                    slot => {
                        let swr = resampler(&frame, audio.sample_rate, audio.channels)?;
                        &mut slot.insert((shape, swr)).1
                    }
                };
                let samples = resample(swr, Some(&frame), audio.sample_rate, audio.channels)?;
                let time = since_origin(&mut self.origin, self.number, time, sender);
                if !samples.is_empty() {
                    let message = Captured::Audio {
                        input: self.number,
                        time,
                        samples,
                    };
                    if sender.send(message).is_err() {
                        return Ok(false);
                    }
                }
            }
            if packet.is_none()
                && let Some((_, swr)) = &audio.resampler
                && let Some(origin) = self.origin
            {
                // What the resampler still holds follows the last samples.
                let samples = resample(swr, None, audio.sample_rate, audio.channels)?;
                if !samples.is_empty() {
                    let message = Captured::Audio {
                        input: self.number,
                        time: audio.next - origin,
                        samples,
                    };
                    let _ = sender.send(message);
                }
            }
        }
        Ok(true)
    }
}

/// `time` from the input's first decoded frame, which sets the origin (and reports the
/// moment it arrived) when it is the first.
fn since_origin(
    origin: &mut Option<Flicks>,
    input: usize,
    time: Flicks,
    sender: &Sender<Captured>,
) -> Flicks {
    let start = *origin.get_or_insert_with(|| {
        let _ = sender.send(Captured::Started {
            input,
            at: Instant::now(),
        });
        time
    });
    time - start
}

/// Sends a packet (or the flush) to a decoder and collects every frame it returns.
/// Packets the decoder rejects as invalid (a damaged picture from a camera) are skipped.
fn decode_packet(
    decoder: &mut AVCodecContext,
    packet: Option<&AVPacket>,
    frames: &mut Vec<AVFrame>,
) -> Result<(), MediaError> {
    for _ in 0..2 {
        match decoder.send_packet(packet) {
            Ok(()) | Err(RsmpegError::DecoderFlushedError) => break,
            Err(RsmpegError::DecoderFullError) => receive_frames(decoder, frames)?,
            Err(RsmpegError::SendPacketError(code)) if code == ffi::AVERROR_INVALIDDATA => {
                return Ok(());
            }
            Err(error) => return Err(decode_failed(&error)),
        }
    }
    receive_frames(decoder, frames)
}

fn receive_frames(
    decoder: &mut AVCodecContext,
    frames: &mut Vec<AVFrame>,
) -> Result<(), MediaError> {
    loop {
        match decoder.receive_frame() {
            Ok(frame) => frames.push(frame),
            Err(RsmpegError::DecoderDrainError) | Err(RsmpegError::DecoderFlushedError) => {
                return Ok(());
            }
            Err(error) => return Err(decode_failed(&error)),
        }
    }
}

/// The time from `from` to `to` in flicks, negative when `to` is earlier.
fn flicks_between(from: Instant, to: Instant) -> Flicks {
    let (duration, sign) = match to.checked_duration_since(from) {
        Some(duration) => (duration, 1),
        None => (from.duration_since(to), -1),
    };
    let flicks = duration.as_nanos() * FLICKS_PER_SECOND as u128 / 1_000_000_000;
    Flicks::new(sign * i64::try_from(flicks).unwrap_or(i64::MAX))
}

/// The number of whole frames that cover `time` (rounding up).
fn frames_covering(rate: Rate, time: Flicks) -> i64 {
    let frames = rate.frame_at(time);
    if rate.frames_to_flicks(frames) < time {
        frames + 1
    } else {
        frames
    }
}

/// Pictures timed to the output's constant frame rate: each picture fills the frames from
/// its own (rounded) time until the next picture's, so pictures are repeated when the
/// device is slower than the output and dropped when it is faster.
struct VideoTrack {
    rate: Rate,
    /// The number of frames to write, when the recording has a duration.
    end: Option<i64>,
    /// The next output frame to write.
    next: i64,
    /// The latest picture and its frame, not written yet.
    held: Option<(i64, RgbaFrame)>,
}

impl VideoTrack {
    fn push(
        &mut self,
        writer: &mut FfmpegWriter,
        time: Flicks,
        frame: RgbaFrame,
    ) -> Result<(), MediaError> {
        if self.complete() {
            return Ok(());
        }
        let slot = self.rate.nearest_frame(time).max(0);
        if let Some((_, held)) = self.held.take() {
            let until = self.end.map_or(slot, |end| slot.min(end));
            while self.next < until {
                writer.push_rgba(&held)?;
                self.next += 1;
            }
        }
        self.held = Some((slot, frame));
        Ok(())
    }

    fn complete(&self) -> bool {
        self.end.is_some_and(|end| self.next >= end)
    }

    /// Frames recorded so far, including the held picture.
    fn extent(&self) -> i64 {
        match &self.held {
            Some((slot, _)) => self.next.max(slot + 1),
            None => self.next,
        }
    }

    /// Writes the held picture until frame `end`.
    fn finish(&mut self, writer: &mut FfmpegWriter, end: i64) -> Result<(), MediaError> {
        if let Some((_, held)) = self.held.take() {
            while self.next < end {
                writer.push_rgba(&held)?;
                self.next += 1;
            }
        }
        Ok(())
    }
}

/// Sound written continuously from where its first samples belong; gaps longer than a
/// tenth of a second (lost buffers) become silence.
struct AudioTrack {
    sample_rate: u32,
    channels: u16,
    /// The number of sample frames to write, when the recording has a duration.
    target: Option<i64>,
    written: i64,
    started: bool,
}

impl AudioTrack {
    fn push(
        &mut self,
        writer: &mut FfmpegWriter,
        time: Flicks,
        mut samples: Vec<f32>,
    ) -> Result<(), MediaError> {
        if self.complete() {
            return Ok(());
        }
        let channels = usize::from(self.channels);
        let position = samples_at(time, self.sample_rate);
        if !self.started {
            if position < 0 {
                let early = usize::try_from(-position).unwrap_or(usize::MAX);
                samples.drain(..early.saturating_mul(channels).min(samples.len()));
                if samples.is_empty() {
                    return Ok(());
                }
            } else {
                self.silence(writer, position)?;
            }
            self.started = true;
        } else if position - self.written > i64::from(self.sample_rate) / AUDIO_GAP_DIVISOR {
            self.silence(writer, position - self.written)?;
        }
        self.append(writer, samples)
    }

    fn complete(&self) -> bool {
        self.target.is_some_and(|target| self.written >= target)
    }

    fn append(
        &mut self,
        writer: &mut FfmpegWriter,
        mut samples: Vec<f32>,
    ) -> Result<(), MediaError> {
        let channels = usize::from(self.channels);
        if let Some(target) = self.target {
            let room = usize::try_from(target - self.written).unwrap_or(0);
            samples.truncate(room.saturating_mul(channels));
        }
        let frames = samples.len() / channels;
        if frames == 0 {
            return Ok(());
        }
        samples.truncate(frames * channels);
        writer.push_audio(&AudioBuffer {
            sample_rate: self.sample_rate,
            channels: self.channels,
            samples,
        })?;
        self.written += frames as i64;
        Ok(())
    }

    /// Writes `frames` sample frames of silence, a second at a time.
    fn silence(&mut self, writer: &mut FfmpegWriter, frames: i64) -> Result<(), MediaError> {
        let mut left = frames.max(0);
        while left > 0 && !self.complete() {
            let chunk = left.min(i64::from(self.sample_rate));
            let count = usize::try_from(chunk).unwrap_or(0) * usize::from(self.channels);
            self.append(writer, vec![0.0; count])?;
            left -= chunk;
        }
        Ok(())
    }

    fn duration(&self) -> Flicks {
        Flicks::new(self.written * FLICKS_PER_SECOND / i64::from(self.sample_rate.max(1)))
    }
}

/// Places captured media on the output's timeline and writes it.
struct Session {
    writer: FfmpegWriter,
    video: Option<VideoTrack>,
    audio: Option<AudioTrack>,
    duration: Option<Flicks>,
    /// When the first input's first media arrived: the start of the recording.
    zero: Option<Instant>,
    /// When each input's first media arrived, from `zero`.
    offsets: [Flicks; 2],
}

impl Session {
    fn new(writer: FfmpegWriter, settings: &OutputSettings, duration: Option<Flicks>) -> Session {
        Session {
            writer,
            video: settings.video.as_ref().map(|video| VideoTrack {
                rate: video.rate,
                end: duration.map(|d| video.rate.nearest_frame(d).max(1)),
                next: 0,
                held: None,
            }),
            audio: settings.audio.as_ref().map(|audio| AudioTrack {
                sample_rate: audio.sample_rate,
                channels: audio.channels,
                target: duration.map(|d| samples_at(d, audio.sample_rate).max(1)),
                written: 0,
                started: false,
            }),
            duration,
            zero: None,
            offsets: [Flicks::ZERO; 2],
        }
    }

    fn offset(&self, input: usize) -> Flicks {
        self.offsets.get(input).copied().unwrap_or(Flicks::ZERO)
    }

    fn handle(&mut self, message: Captured) -> Result<(), MediaError> {
        match message {
            Captured::Started { input, at } => {
                let zero = *self.zero.get_or_insert(at);
                if let Some(offset) = self.offsets.get_mut(input) {
                    *offset = flicks_between(zero, at);
                }
            }
            Captured::Video { input, time, frame } => {
                let time = self.offset(input) + time;
                if let Some(video) = &mut self.video {
                    video.push(&mut self.writer, time, frame)?;
                }
            }
            Captured::Audio {
                input,
                time,
                samples,
            } => {
                let time = self.offset(input) + time;
                if let Some(audio) = &mut self.audio {
                    audio.push(&mut self.writer, time, samples)?;
                }
            }
            Captured::Failed(error) => return Err(error),
        }
        Ok(())
    }

    /// The time recorded so far: the longest of the streams.
    fn recorded(&self) -> Flicks {
        let video = self
            .video
            .as_ref()
            .map_or(Flicks::ZERO, |v| v.rate.frames_to_flicks(v.extent()));
        let audio = self
            .audio
            .as_ref()
            .map_or(Flicks::ZERO, AudioTrack::duration);
        video.max(audio)
    }

    /// Whether every stream has reached the recording's duration.
    fn complete(&self) -> bool {
        self.duration.is_some()
            && self.video.as_ref().is_none_or(VideoTrack::complete)
            && self.audio.as_ref().is_none_or(AudioTrack::complete)
    }

    /// Ends every stream at the duration, or where the longest one ended, and closes the
    /// file.
    fn finish(mut self, sources: &[CaptureInput]) -> Result<(), MediaError> {
        let shown = sources
            .iter()
            .map(describe)
            .collect::<Vec<_>>()
            .join(" and ");
        if self.video.as_ref().is_some_and(|v| v.extent() == 0) {
            return Err(MediaError::NothingRecorded {
                message: format!("{shown} delivered no pictures"),
            });
        }
        if self.audio.as_ref().is_some_and(|a| a.written == 0) {
            return Err(MediaError::NothingRecorded {
                message: format!("{shown} delivered no sound"),
            });
        }
        let recorded = self.recorded();
        let end = self.duration.map_or(recorded, |d| d.min(recorded));
        let mut video_end = None;
        if let Some(video) = &mut self.video {
            let frames = frames_covering(video.rate, end).min(video.end.unwrap_or(i64::MAX));
            video.finish(&mut self.writer, frames)?;
            video_end = Some(video.rate.frames_to_flicks(frames));
        }
        if let Some(audio) = &mut self.audio {
            let natural = samples_at(video_end.unwrap_or(end), audio.sample_rate);
            let wanted = audio.target.map_or(natural, |t| t.min(natural));
            audio.silence(&mut self.writer, wanted - audio.written)?;
        }
        self.writer.finish()
    }
}

/// The temporary file a recording is written to: `.name.partial` next to the output.
fn partial_path(output: &Path) -> Result<PathBuf, MediaError> {
    let name = output.file_name().ok_or_else(|| MediaError::WriteFailed {
        path: output.display().to_string(),
        message: "the output has no file name".into(),
    })?;
    Ok(output.with_file_name(format!(".{}.partial", name.to_string_lossy())))
}

/// Records `recording.input` (and its microphone, if any) into `recording.output` until
/// the duration is reached, `stop` is set or the input ends, calling `progress` with the
/// time recorded so far. The file is written under a temporary name next to the output and
/// moved into place only when complete, then probed; a recording that fails leaves no file
/// behind.
pub fn record(
    recording: &Recording,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(Flicks),
) -> Result<OutputReport, MediaError> {
    register();
    if recording.duration.is_some_and(|d| !d.is_positive()) {
        return Err(MediaError::Unsupported {
            message: "a recording's duration must be positive".into(),
        });
    }
    let temporary = partial_path(&recording.output)?;
    let writer = match FfmpegWriter::create(&temporary, &recording.settings) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    let control = Control {
        stop,
        done: AtomicBool::new(false),
    };
    if let Err(error) = capture(recording, &control, writer, progress) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    // `rename` replaces an existing file in one step on every platform.
    if let Err(error) = std::fs::rename(&temporary, &recording.output) {
        let _ = std::fs::remove_file(&temporary);
        return Err(MediaError::WriteFailed {
            path: recording.output.display().to_string(),
            message: error.to_string(),
        });
    }
    probe_output(&recording.output)
}

/// The video stream of `input`, decoded and fitted to the output size.
fn video_reader(
    input: &AVFormatContextInput,
    source: &CaptureInput,
    output: &VideoSettings,
) -> Result<VideoReader, MediaError> {
    let (index, decoder) =
        best_stream(input, ffi::AVMEDIA_TYPE_VIDEO)?.ok_or_else(|| MediaError::NoVideoStream {
            path: describe(source),
        })?;
    let stream = &input.streams()[index];
    let rate = timebase::rate(stream.avg_frame_rate.num, stream.avg_frame_rate.den)
        .or_else(|| timebase::rate(stream.r_frame_rate.num, stream.r_frame_rate.den))
        .unwrap_or(output.rate);
    Ok(VideoReader {
        index,
        decoder,
        time_base: (stream.time_base.num, stream.time_base.den),
        frame_duration: rate.frame_duration(),
        next: Flicks::ZERO,
        fitter: Fitter {
            width: output.width,
            height: output.height,
            scaler: None,
        },
    })
}

/// The audio stream of `input`, decoded and resampled to the output's format.
fn audio_reader(
    input: &AVFormatContextInput,
    source: &CaptureInput,
    output: &AudioSettings,
) -> Result<AudioReader, MediaError> {
    let (index, decoder) =
        best_stream(input, ffi::AVMEDIA_TYPE_AUDIO)?.ok_or_else(|| MediaError::NoAudioStream {
            path: describe(source),
        })?;
    let stream = &input.streams()[index];
    Ok(AudioReader {
        index,
        decoder,
        time_base: (stream.time_base.num, stream.time_base.den),
        sample_rate: output.sample_rate,
        channels: output.channels,
        resampler: None,
        next: Flicks::ZERO,
    })
}

/// Opens the inputs, reads them on their own threads and writes what they capture.
fn capture(
    recording: &Recording,
    control: &Control<'_>,
    writer: FfmpegWriter,
    progress: &mut dyn FnMut(Flicks),
) -> Result<(), MediaError> {
    let settings = &recording.settings;
    let main = recording.capture_input();
    let first = open_device(&main, control)?;
    let second = match &recording.microphone {
        Some(microphone) => Some((microphone.clone(), open_device(microphone, control)?)),
        None => None,
    };
    let video = match &settings.video {
        Some(output) => Some(video_reader(&first, &main, output)?),
        None => None,
    };
    let audio = settings.audio.as_ref();
    let mut readers = Vec::new();
    let mut sources = vec![main.clone()];
    match second {
        Some((microphone, input)) => {
            let audio = audio
                .map(|output| audio_reader(&input, &microphone, output))
                .transpose()?;
            readers.push(Reader {
                number: 0,
                source: main,
                input: first,
                video,
                audio: None,
                origin: None,
            });
            sources.push(microphone.clone());
            readers.push(Reader {
                number: 1,
                source: microphone,
                input,
                video: None,
                audio,
                origin: None,
            });
        }
        None => {
            let audio = audio
                .map(|output| audio_reader(&first, &main, output))
                .transpose()?;
            readers.push(Reader {
                number: 0,
                source: main,
                input: first,
                video,
                audio,
                origin: None,
            });
        }
    }
    readers.retain(|r| r.video.is_some() || r.audio.is_some());

    let mut session = Session::new(writer, settings, recording.duration);
    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        // However this closure ends, the readers stop and the scope can join them.
        let _finish = Finish(&control.done);
        for reader in readers {
            let sender = sender.clone();
            scope.spawn(move || reader.run(control, &sender));
        }
        drop(sender);
        let mut reported = Flicks::ZERO;
        for message in receiver.iter() {
            session.handle(message)?;
            let recorded = session.recorded();
            if recorded > reported {
                reported = recorded;
                progress(recorded);
            }
            if session.complete() {
                break;
            }
        }
        Ok::<_, MediaError>(())
    })?;
    if control.stopped() && session.recorded() == Flicks::ZERO {
        return Err(stopped_early(&sources[0]));
    }
    session.finish(&sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_inside_the_output_without_distortion() {
        // The same aspect ratio fills the output.
        assert_eq!(fit(320, 240, 320, 240), (320, 240));
        assert_eq!(fit(1920, 1080, 1280, 720), (1280, 720));
        // 4:3 in 16:9 is as tall as the output, with bars at the sides.
        assert_eq!(fit(320, 240, 640, 360), (480, 360));
        // 16:9 in 9:16 is as wide as the output (607.5 rows round up).
        assert_eq!(fit(1920, 1080, 1080, 1920), (1080, 608));
        // Degenerate sizes still give at least one pixel; an empty size counts as square.
        assert_eq!(fit(10_000, 1, 64, 64), (64, 1));
        assert_eq!(fit(0, 0, 64, 36), (36, 36));
    }

    #[test]
    fn avfoundation_listings_become_devices() {
        let lines: Vec<String> = [
            "[AVFoundation indev @ 0x11e8048d0] AVFoundation video devices:\n",
            "[AVFoundation indev @ 0x11e8048d0] [0] FaceTime HD Camera\n",
            "[1] Capture screen 0\n",
            "unrelated line\n",
            "AVFoundation audio devices:\n",
            "[0] MacBook Pro Microphone\n",
            "[x] not a device\n",
        ]
        .map(String::from)
        .to_vec();
        let devices = parse_avfoundation(&lines);
        let summary: Vec<_> = devices
            .iter()
            .map(|d| (d.kind, d.name.as_str(), d.format.as_str(), d.input.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    DeviceKind::Camera,
                    "FaceTime HD Camera",
                    "avfoundation",
                    "0:"
                ),
                (DeviceKind::Screen, "Capture screen 0", "avfoundation", "1:"),
                (
                    DeviceKind::Microphone,
                    "MacBook Pro Microphone",
                    "avfoundation",
                    ":0"
                ),
            ]
        );
        assert!(parse_avfoundation(&["[0] before any header".to_string()]).is_empty());
    }

    #[test]
    fn frame_counts_round_up_to_cover_a_time() {
        let rate = Rate::FPS_30;
        assert_eq!(frames_covering(rate, Flicks::ZERO), 0);
        assert_eq!(frames_covering(rate, rate.frames_to_flicks(15)), 15);
        assert_eq!(
            frames_covering(rate, rate.frames_to_flicks(15) + Flicks::new(1)),
            16
        );
        assert_eq!(
            frames_covering(Rate::NTSC_30, Flicks::from_millis(1001)),
            30
        );
    }

    #[test]
    fn instants_convert_to_signed_flicks() {
        let start = Instant::now();
        let later = start + Duration::from_millis(250);
        assert_eq!(flicks_between(start, later), Flicks::from_millis(250));
        assert_eq!(flicks_between(later, start), Flicks::from_millis(-250));
        assert_eq!(flicks_between(start, start), Flicks::ZERO);
    }

    #[test]
    fn partial_files_sit_next_to_the_output() {
        assert_eq!(
            partial_path(Path::new("/tmp/takes/voice.wav")).unwrap(),
            PathBuf::from("/tmp/takes/.voice.wav.partial")
        );
        assert!(matches!(
            partial_path(Path::new("/")),
            Err(MediaError::WriteFailed { .. })
        ));
    }

    #[test]
    fn unknown_input_formats_name_the_available_ones() {
        let error = input_format("no-such-device").err().unwrap();
        assert_eq!(error.code(), "fosscut.media.input_format_unavailable");
        let MediaError::InputFormatUnavailable { format, available } = error else {
            unreachable!()
        };
        assert_eq!(format, "no-such-device");
        assert_eq!(available, input_formats());
        assert!(input_format("").is_err());
    }
}
