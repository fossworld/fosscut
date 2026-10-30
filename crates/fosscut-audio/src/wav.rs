//! Writing audio as WAVE files: 16-bit PCM in a RIFF container, the form every audio tool
//! and media framework reads.
//!
//! Samples are interleaved `f32` from -1 to 1. They are clamped to that range, scaled by
//! 32767 and rounded to the nearest integer, so full scale maps to ±32767 and a round trip
//! is exact to within half a step; NaN becomes silence. A trailing partial frame (when the
//! sample count is not a multiple of the channel count) is left out, so the file holds
//! whole frames only.

use std::io;
use std::path::Path;

/// Size of the RIFF header: the RIFF, `fmt ` and `data` chunk headers.
const HEADER_BYTES: usize = 44;

/// Writes interleaved f32 samples (-1 to 1, clamped) as a 16-bit PCM RIFF/WAVE file.
///
/// Writes exactly the bytes of [`wav_bytes`]. Fails with [`io::ErrorKind::InvalidInput`]
/// when `channels` or `sample_rate` is 0, or when the audio is too long for a RIFF file
/// (whose sizes are 32-bit), and with the error of the file system otherwise.
pub fn write_wav(path: &Path, samples: &[f32], channels: u16, sample_rate: u32) -> io::Result<()> {
    if channels == 0 || sample_rate == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a WAVE file needs at least one channel and a sample rate above 0",
        ));
    }
    let riff_size = whole_frames(samples.len(), channels)
        .checked_mul(2)
        .and_then(|data| data.checked_add(HEADER_BYTES - 8));
    if riff_size
        .and_then(|size| u32::try_from(size).ok())
        .is_none()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the audio is too long for a WAVE file (4 GiB at most)",
        ));
    }
    std::fs::write(path, wav_bytes(samples, channels, sample_rate))
}

/// The same as bytes.
///
/// The header holds the sizes of the RIFF and `data` chunks, format 1 (integer PCM), the
/// channel count, the sample rate, the byte rate (`sample_rate × channels × 2`), the block
/// alignment (`channels × 2`) and 16 bits per sample, followed by the samples as
/// little-endian `i16`. Sizes too large for 32 bits are written as `u32::MAX`; [`write_wav`]
/// refuses such audio instead.
pub fn wav_bytes(samples: &[f32], channels: u16, sample_rate: u32) -> Vec<u8> {
    let count = whole_frames(samples.len(), channels);
    let data = count.saturating_mul(2);
    let data_size = u32::try_from(data).unwrap_or(u32::MAX);
    let riff_size = u32::try_from(data.saturating_add(HEADER_BYTES - 8)).unwrap_or(u32::MAX);
    let block_align = channels.saturating_mul(2);
    let byte_rate = sample_rate.saturating_mul(u32::from(block_align));
    let mut bytes = Vec::with_capacity(HEADER_BYTES.saturating_add(data));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for &sample in &samples[..count] {
        bytes.extend_from_slice(&to_pcm(sample).to_le_bytes());
    }
    bytes
}

/// The number of samples in whole frames of `channels` channels.
fn whole_frames(samples: usize, channels: u16) -> usize {
    let channels = usize::from(channels);
    if channels == 0 {
        0
    } else {
        samples - samples % channels
    }
}

/// A sample as a 16-bit integer: clamped, scaled by 32767 and rounded; NaN is 0.
fn to_pcm(sample: f32) -> i16 {
    if sample.is_nan() {
        return 0;
    }
    // Within ±32767 after the clamp, so the conversion is exact.
    (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    /// The samples of a 16-bit file, back as floats.
    fn decode(bytes: &[u8]) -> Vec<f32> {
        bytes[HEADER_BYTES..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| f32::from(i16::from_le_bytes(pair)) / 32767.0)
            .collect()
    }

    /// A stereo sweep through the whole range, with values between the steps.
    fn ramp_samples(frames: usize) -> Vec<f32> {
        (0..frames * 2)
            .map(|i| {
                let x = i as f32 / (frames * 2 - 1) as f32;
                if i % 2 == 0 {
                    2.0 * x - 1.0
                } else {
                    (x * 7.3).sin() * 0.9
                }
            })
            .collect()
    }

    #[test]
    fn the_header_describes_16_bit_pcm() {
        let samples = ramp_samples(1000);
        let bytes = wav_bytes(&samples, 2, 48_000);
        assert_eq!(bytes.len(), 44 + 4000);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32_at(&bytes, 4), (bytes.len() - 8) as u32);
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u32_at(&bytes, 16), 16, "fmt chunk size");
        assert_eq!(u16_at(&bytes, 20), 1, "integer PCM");
        assert_eq!(u16_at(&bytes, 22), 2, "channels");
        assert_eq!(u32_at(&bytes, 24), 48_000, "sample rate");
        assert_eq!(u32_at(&bytes, 28), 48_000 * 4, "byte rate");
        assert_eq!(u16_at(&bytes, 32), 4, "block align");
        assert_eq!(u16_at(&bytes, 34), 16, "bits per sample");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32_at(&bytes, 40), 4000, "data size");

        let mono = wav_bytes(&samples[..441], 1, 44_100);
        assert_eq!(u16_at(&mono, 22), 1);
        assert_eq!(u32_at(&mono, 24), 44_100);
        assert_eq!(u32_at(&mono, 28), 88_200);
        assert_eq!(u16_at(&mono, 32), 2);
        assert_eq!(u32_at(&mono, 40), 882);
        assert_eq!(u32_at(&mono, 4), 36 + 882);
    }

    #[test]
    fn samples_round_trip_within_one_step() {
        let samples = ramp_samples(5000);
        let decoded = decode(&wav_bytes(&samples, 2, 48_000));
        assert_eq!(decoded.len(), samples.len());
        let worst = samples
            .iter()
            .zip(&decoded)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 0.5 / 32767.0 + 1e-7, "worst error {worst}");
        assert_eq!(
            decode(&wav_bytes(&[-1.0, 1.0, 0.0], 1, 48_000)),
            [-1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn samples_outside_full_scale_are_clamped_and_nan_is_silence() {
        let bytes = wav_bytes(&[1.5, -1.5, f32::NAN, 0.0, f32::INFINITY, -0.25], 2, 8000);
        let values: Vec<i16> = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| i16::from_le_bytes(pair))
            .collect();
        assert_eq!(values, [32767, -32767, 0, 0, 32767, -8192]);
    }

    #[test]
    fn a_trailing_partial_frame_is_left_out() {
        let bytes = wav_bytes(&[0.1, 0.2, 0.3, 0.4, 0.5], 2, 48_000);
        assert_eq!(u32_at(&bytes, 40), 8);
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(decode(&bytes).len(), 4);
    }

    #[test]
    fn write_wav_writes_the_same_bytes() {
        let samples = ramp_samples(2000);
        let path =
            std::env::temp_dir().join(format!("fosscut-wav-test-{}.wav", std::process::id()));
        write_wav(&path, &samples, 2, 44_100).unwrap();
        let written = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(written, wav_bytes(&samples, 2, 44_100));
    }

    #[test]
    fn write_wav_refuses_headers_no_reader_accepts() {
        let path =
            std::env::temp_dir().join(format!("fosscut-wav-refused-{}.wav", std::process::id()));
        for (channels, rate) in [(0, 48_000), (2, 0)] {
            let error = write_wav(&path, &[0.0; 4], channels, rate).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!path.exists());
    }
}
