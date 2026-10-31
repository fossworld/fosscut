//! Hardware acceleration for the whole process (ADR-0032, item 6; ADR-0031, item 4).
//!
//! Hosts set these from the user's `performance.hardware_decoding` and
//! `performance.hardware_encoding` preferences when they start and when the preferences
//! change. Decoding applies to exports and to the source decoding of proxy jobs; the
//! preview takes its own choice in [`crate::preview::PreviewOptions`]. Encoding applies to
//! every writer the engine creates, and to which codecs presets and capabilities offer.
//! Neither changes what is written: with hardware encoding off, a codec that only hardware
//! encodes here becomes unavailable with `fosscut.export.codec_unavailable` (or
//! `fosscut.media.encoder_unavailable` from a writer), and is never replaced.

use std::sync::atomic::{AtomicU8, Ordering};

pub use fosscut_media::{HardwareDecoding, HardwareEncoding};

static DECODING: AtomicU8 = AtomicU8::new(0);

/// Sets the hardware decoding choice for exports and proxy jobs. [`HardwareDecoding::Auto`]
/// until set.
pub fn set_hardware_decoding(choice: HardwareDecoding) {
    let value = match choice {
        HardwareDecoding::Auto => 0,
        HardwareDecoding::Off => 1,
        HardwareDecoding::Require => 2,
    };
    DECODING.store(value, Ordering::Relaxed);
}

/// The hardware decoding choice for exports and proxy jobs.
pub fn hardware_decoding() -> HardwareDecoding {
    match DECODING.load(Ordering::Relaxed) {
        1 => HardwareDecoding::Off,
        2 => HardwareDecoding::Require,
        _ => HardwareDecoding::Auto,
    }
}

/// Sets whether hardware encoders may be used, for every writer and encoder query of the
/// process ([`fosscut_media::ffmpeg::set_hardware_encoding`]). [`HardwareEncoding::Auto`]
/// until set.
pub fn set_hardware_encoding(choice: HardwareEncoding) {
    fosscut_media::ffmpeg::set_hardware_encoding(choice);
}

/// Whether hardware encoders may be used.
pub fn hardware_encoding() -> HardwareEncoding {
    fosscut_media::ffmpeg::hardware_encoding()
}

/// Decoder options with the process's hardware decoding choice.
pub(crate) fn decoder_options() -> fosscut_media::DecoderOptions {
    fosscut_media::DecoderOptions {
        hardware: hardware_decoding(),
    }
}
