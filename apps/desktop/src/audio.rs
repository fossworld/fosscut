//! Preview sound (ADR-0008, item 5): the timeline is mixed a little ahead into a queue
//! that the audio device drains. How much the device has played is the playback clock, so
//! the picture follows the sound. Short sounds of the app's own, such as the one that says
//! an export finished, play over it without moving the clock.

use std::collections::VecDeque;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use fosscut_engine::preview::Sound;
use fosscut_time::{FLICKS_PER_SECOND, Flicks};

use crate::preview::Scene;

/// How far ahead of the device the mixer keeps the queue filled.
const AHEAD: Duration = Duration::from_millis(250);
/// How much the mixer mixes at a time.
const BLOCK: Duration = Duration::from_millis(100);
/// How much sound one step of scrubbing or skimming plays.
const SCRUB: Duration = Duration::from_millis(80);

/// Interleaved samples waiting for the device, and how many frames it has played since
/// playback began. Only real samples count: when the queue runs dry the device plays
/// silence and the clock waits, so the picture never runs ahead of the sound. A one-shot
/// sound waits in an overlay of its own, mixed over whatever plays, which counts for
/// nothing and survives a reset.
pub struct Queue {
    samples: VecDeque<f32>,
    overlay: VecDeque<f32>,
    /// The sound of the frame scrubbed to last, replaced by the next scrub.
    scrubbed: VecDeque<f32>,
    channels: usize,
    played: u64,
}

impl Queue {
    /// An empty queue for `channels` interleaved channels.
    pub fn new(channels: u16) -> Queue {
        Queue {
            samples: VecDeque::new(),
            overlay: VecDeque::new(),
            scrubbed: VecDeque::new(),
            channels: usize::from(channels.max(1)),
            played: 0,
        }
    }

    /// Adds interleaved samples to play once over the queue, after any still waiting.
    pub fn overlay(&mut self, samples: &[f32]) {
        self.overlay.extend(samples);
    }

    /// Plays the sound of a scrubbed frame over the queue, instead of what is left of the
    /// last one, so fast scrubbing never falls behind.
    pub fn scrub(&mut self, samples: &[f32]) {
        self.scrubbed.clear();
        self.scrubbed.extend(samples);
    }

    /// Adds mixed samples at the end.
    pub fn push(&mut self, samples: &[f32]) {
        self.samples.extend(samples);
    }

    /// Fills the device's buffer from the front, with silence once the queue is empty.
    pub fn pull(&mut self, out: &mut [f32]) {
        let available = self.samples.len().min(out.len());
        let whole = available - available % self.channels;
        for (slot, sample) in out.iter_mut().zip(self.samples.drain(..whole)) {
            *slot = sample;
        }
        out[whole..].fill(0.0);
        self.played += (whole / self.channels) as u64;
        for extra in [&mut self.overlay, &mut self.scrubbed] {
            let over = extra.len().min(out.len());
            for (slot, sample) in out.iter_mut().zip(extra.drain(..over)) {
                *slot += sample;
            }
        }
    }

    /// Frames waiting.
    pub fn buffered(&self) -> u64 {
        (self.samples.len() / self.channels) as u64
    }

    /// Frames played since the queue was last reset.
    pub fn played(&self) -> u64 {
        self.played
    }

    /// Empties the queue and restarts the count.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.played = 0;
    }
}

/// Where playback is: it began at `from` and `played` frames at `rate` have been heard.
pub fn position(from: Flicks, played: u64, rate: u32) -> Flicks {
    let elapsed = u128::from(played) * FLICKS_PER_SECOND as u128 / u128::from(rate.max(1));
    from + Flicks::new(i64::try_from(elapsed).unwrap_or(i64::MAX))
}

/// What the mixer works on while sound plays.
struct Playing {
    scene: Arc<Scene>,
    from: Flicks,
    /// Mixed up to here.
    next: Flicks,
    /// Counts plays, so a span mixed for an earlier one is dropped.
    generation: u64,
}

struct Shared {
    queue: Queue,
    playing: Option<Playing>,
    /// A frame to play the sound of, while scrubbing or skimming.
    scrub: Option<(Arc<Scene>, Flicks)>,
    generation: u64,
}

fn guard(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The default audio output, playing the preview's sound. Clones play through the same
/// device.
#[derive(Clone)]
pub struct Speaker {
    shared: Arc<Mutex<Shared>>,
    rate: u32,
    channels: u16,
}

impl Speaker {
    /// Opens the default output device in 32-bit float, or `None` when there is no device
    /// or it plays no float format; the preview then keeps time by the wall clock.
    pub fn open() -> Option<Speaker> {
        let (ready, answer) = channel();
        std::thread::Builder::new()
            .name("fosscut-sound".into())
            .spawn(move || {
                let Some((stream, shared, rate, channels)) = output() else {
                    let _ = ready.send(None);
                    return;
                };
                let _ = ready.send(Some((Arc::clone(&shared), rate, channels)));
                mixer(&shared, rate, channels);
                drop(stream);
            })
            .ok()?;
        let (shared, rate, channels) = answer.recv().ok().flatten()?;
        Some(Speaker {
            shared,
            rate,
            channels,
        })
    }

    /// Plays the library sound `name` once, over anything playing; false when the library
    /// has no such sound.
    pub fn play_sound(&self, name: &str) -> bool {
        let Some(stereo) = fosscut_audio::synth::render(name, self.rate) else {
            return false;
        };
        let samples = to_channels(&stereo, self.channels);
        guard(&self.shared).queue.overlay(&samples);
        true
    }

    /// Plays mono `samples` at `rate` once, over anything playing, as a preview of speech
    /// plays; false when there is nothing to play.
    pub fn play_mono(&self, samples: &[f32], rate: u32) -> bool {
        if samples.is_empty() || rate == 0 {
            return false;
        }
        let stereo = mono_to_stereo(samples, rate, self.rate);
        guard(&self.shared)
            .queue
            .overlay(&to_channels(&stereo, self.channels));
        true
    }

    /// Plays `scene` from `from`, dropping whatever was queued.
    pub fn play(&self, scene: Arc<Scene>, from: Flicks) {
        let mut shared = guard(&self.shared);
        shared.queue.reset();
        shared.generation += 1;
        shared.playing = Some(Playing {
            scene,
            from,
            next: from,
            generation: shared.generation,
        });
    }

    /// Mixes the rest from `scene`, after an edit while playing.
    pub fn update(&self, scene: Arc<Scene>) {
        if let Some(playing) = &mut guard(&self.shared).playing {
            playing.scene = scene;
        }
    }

    /// Plays a moment of `scene`'s sound at `at`, as scrubbing does; nothing while playing.
    pub fn scrub(&self, scene: Arc<Scene>, at: Flicks) {
        let mut shared = guard(&self.shared);
        if shared.playing.is_none() {
            shared.scrub = Some((scene, at));
        }
    }

    /// Stops the sound.
    pub fn stop(&self) {
        let mut shared = guard(&self.shared);
        shared.playing = None;
        shared.queue.reset();
    }

    /// The time being heard, while playing.
    pub fn position(&self) -> Option<Flicks> {
        let shared = guard(&self.shared);
        let playing = shared.playing.as_ref()?;
        Some(position(playing.from, shared.queue.played(), self.rate))
    }
}

/// Fades the last quarter of interleaved `samples` out, so a scrubbed moment ends without
/// a click.
pub fn fade_out(samples: &mut [f32], channels: usize) {
    let frames = samples.len() / channels.max(1);
    let fade = frames / 4;
    for (index, frame) in samples.chunks_exact_mut(channels.max(1)).enumerate() {
        let left = frames - index;
        if left <= fade {
            let gain = left as f32 / (fade + 1) as f32;
            frame.iter_mut().for_each(|sample| *sample *= gain);
        }
    }
}

/// Mono samples at `from` samples a second as interleaved stereo at `to`, resampled
/// linearly (enough for a preview of speech).
pub fn mono_to_stereo(mono: &[f32], from: u32, to: u32) -> Vec<f32> {
    if mono.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    let frames = (mono.len() as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    let mut out = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let at = frame as f64 * step;
        let index = at.floor() as usize;
        let next = (index + 1).min(mono.len() - 1);
        let fraction = (at - index as f64) as f32;
        let here = mono[index.min(mono.len() - 1)];
        let sample = here + (mono[next] - here) * fraction;
        out.extend_from_slice(&[sample, sample]);
    }
    out
}

/// Interleaved stereo as `channels` channels: one is the two mixed, more than two leave
/// the rest silent.
pub fn to_channels(stereo: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    let mut out = Vec::with_capacity(stereo.len() / 2 * channels);
    for [left, right] in stereo.as_chunks::<2>().0 {
        if channels == 1 {
            out.push((left + right) / 2.0);
        } else {
            out.extend_from_slice(&[*left, *right]);
            out.extend(std::iter::repeat_n(0.0, channels - 2));
        }
    }
    out
}

type Output = (cpal::Stream, Arc<Mutex<Shared>>, u32, u16);

/// The default device's output stream, draining the shared queue.
fn output() -> Option<Output> {
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return None;
    }
    let (rate, channels) = (config.sample_rate(), config.channels());
    let shared = Arc::new(Mutex::new(Shared {
        queue: Queue::new(channels),
        playing: None,
        scrub: None,
        generation: 0,
    }));
    let device_side = Arc::clone(&shared);
    let stream = device
        .build_output_stream::<f32, _, _>(
            config.config(),
            move |out: &mut [f32], _| guard(&device_side).queue.pull(out),
            |error| eprintln!("fosscut-desktop: sound: {error}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some((stream, shared, rate, channels))
}

/// Keeps the queue about `AHEAD` ahead of the device while sound plays. Spans that fail to
/// mix (missing media) play as silence, so the clock keeps going.
fn mixer(shared: &Mutex<Shared>, rate: u32, channels: u16) {
    let ahead = (AHEAD.as_secs_f64() * f64::from(rate)) as u64;
    let block = Flicks::from_millis(BLOCK.as_millis() as i64);
    let mut sound = Sound::new();
    loop {
        let scrub = guard(shared).scrub.take();
        if let Some((scene, at)) = scrub {
            let span = Flicks::from_millis(SCRUB.as_millis() as i64);
            if let Ok(buffer) = sound.mix(
                &scene.project,
                &scene.bundle,
                &scene.timeline,
                at,
                span,
                rate,
                channels,
            ) {
                let mut samples = buffer.samples;
                fade_out(&mut samples, usize::from(channels.max(1)));
                guard(shared).queue.scrub(&samples);
            }
            continue;
        }
        let work = {
            let shared = guard(shared);
            match &shared.playing {
                Some(playing) if shared.queue.buffered() < ahead => {
                    Some((Arc::clone(&playing.scene), playing.next, playing.generation))
                }
                _ => None,
            }
        };
        let Some((scene, start, generation)) = work else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        let samples = sound
            .mix(
                &scene.project,
                &scene.bundle,
                &scene.timeline,
                start,
                block,
                rate,
                channels,
            )
            .map(|buffer| buffer.samples)
            .unwrap_or_else(|_| {
                let frames = (BLOCK.as_secs_f64() * f64::from(rate)) as usize;
                vec![0.0; frames * usize::from(channels)]
            });
        let mut shared = guard(shared);
        if let Some(playing) = &mut shared.playing
            && playing.generation == generation
        {
            playing.next = start + block;
            shared.queue.push(&samples);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_counts_only_real_frames_played() {
        let mut queue = Queue::new(2);
        queue.push(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        assert_eq!(queue.buffered(), 3);
        let mut out = [9.0; 4];
        queue.pull(&mut out);
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!((queue.played(), queue.buffered()), (2, 1));
        let mut out = [9.0; 6];
        queue.pull(&mut out);
        assert_eq!(
            out,
            [0.5, 0.6, 0.0, 0.0, 0.0, 0.0],
            "silence once it runs dry"
        );
        assert_eq!(queue.played(), 3, "silence does not move the clock");
        queue.push(&[1.0]);
        let mut out = [9.0; 2];
        queue.pull(&mut out);
        assert_eq!(out, [0.0, 0.0], "a partial frame waits for the rest");
        queue.reset();
        assert_eq!((queue.played(), queue.buffered()), (0, 0));
        assert_eq!(Queue::new(0).channels, 1);
    }

    #[test]
    fn a_one_shot_sound_plays_over_the_queue_without_moving_the_clock() {
        let mut queue = Queue::new(2);
        queue.push(&[0.25, 0.25]);
        queue.overlay(&[0.5, 0.5, 0.25, 0.25, 0.125, 0.125]);
        let mut out = [9.0; 4];
        queue.pull(&mut out);
        assert_eq!(
            out,
            [0.75, 0.75, 0.25, 0.25],
            "mixed over the queue and over silence"
        );
        assert_eq!(queue.played(), 1, "only the queue's frame counts");
        queue.reset();
        let mut out = [9.0; 4];
        queue.pull(&mut out);
        assert_eq!(
            out,
            [0.125, 0.125, 0.0, 0.0],
            "a reset leaves the sound playing"
        );
        assert_eq!(queue.played(), 0);
    }

    #[test]
    fn a_scrubbed_moment_replaces_the_last_and_fades_out() {
        let mut queue = Queue::new(1);
        queue.scrub(&[0.5, 0.5, 0.5]);
        let mut out = [9.0; 1];
        queue.pull(&mut out);
        assert_eq!(out, [0.5]);
        queue.scrub(&[0.25, 0.25]);
        let mut out = [9.0; 3];
        queue.pull(&mut out);
        assert_eq!(
            out,
            [0.25, 0.25, 0.0],
            "the rest of the last moment is dropped"
        );
        assert_eq!(queue.played(), 0, "scrubbing does not move the clock");

        let mut samples = vec![1.0; 16];
        fade_out(&mut samples, 2);
        assert_eq!(
            &samples[..12],
            &[1.0; 12],
            "the first three quarters are untouched"
        );
        assert_eq!(
            &samples[12..],
            &[2.0 / 3.0, 2.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]
        );
    }

    #[test]
    fn stereo_becomes_the_devices_channels() {
        let stereo = [0.25, 0.75, -0.5, 0.0];
        assert_eq!(to_channels(&stereo, 2), stereo);
        assert_eq!(to_channels(&stereo, 1), [0.5, -0.25]);
        assert_eq!(to_channels(&stereo, 0), [0.5, -0.25], "no channels is one");
        assert_eq!(
            to_channels(&stereo, 4),
            [0.25, 0.75, 0.0, 0.0, -0.5, 0.0, 0.0, 0.0]
        );
        let ding = fosscut_audio::synth::render("ding", 8_000).unwrap();
        assert!(
            !ding.is_empty() && ding.iter().any(|s| s.abs() > 0.01),
            "the export sound is audible"
        );
    }

    #[test]
    fn mono_speech_is_resampled_to_the_devices_rate_in_stereo() {
        assert_eq!(mono_to_stereo(&[0.5, -0.5], 1, 1), [0.5, 0.5, -0.5, -0.5]);
        assert_eq!(
            mono_to_stereo(&[0.0, 1.0], 1, 2),
            [0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0],
            "twice the rate, in between linearly"
        );
        assert_eq!(
            mono_to_stereo(&[0.0, 0.5, 1.0, 0.5], 2, 1),
            [0.0, 0.0, 1.0, 1.0]
        );
        let second = vec![0.25; 24_000];
        assert_eq!(
            mono_to_stereo(&second, 24_000, 48_000).len(),
            96_000,
            "a second"
        );
        assert!(mono_to_stereo(&[], 24_000, 48_000).is_empty());
        assert!(mono_to_stereo(&[0.5], 0, 48_000).is_empty());
    }

    #[test]
    fn the_position_is_where_playback_began_plus_what_was_heard() {
        let from = Flicks::from_millis(2_000);
        assert_eq!(position(from, 0, 48_000), from);
        assert_eq!(position(from, 24_000, 48_000), Flicks::from_millis(2_500));
        assert_eq!(
            position(Flicks::ZERO, 44_100, 44_100),
            Flicks::from_millis(1_000)
        );
        assert_eq!(
            position(Flicks::ZERO, 1, 0),
            Flicks::new(FLICKS_PER_SECOND),
            "a zero rate is one"
        );
    }
}
