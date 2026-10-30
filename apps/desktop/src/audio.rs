//! Preview sound (ADR-0008, item 5): the timeline is mixed a little ahead into a queue
//! that the audio device drains. How much the device has played is the playback clock, so
//! the picture follows the sound.

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

/// Interleaved samples waiting for the device, and how many frames it has played since
/// playback began. Only real samples count: when the queue runs dry the device plays
/// silence and the clock waits, so the picture never runs ahead of the sound.
pub struct Queue {
    samples: VecDeque<f32>,
    channels: usize,
    played: u64,
}

impl Queue {
    /// An empty queue for `channels` interleaved channels.
    pub fn new(channels: u16) -> Queue {
        Queue {
            samples: VecDeque::new(),
            channels: usize::from(channels.max(1)),
            played: 0,
        }
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
    generation: u64,
}

fn guard(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The default audio output, playing the preview's sound.
pub struct Speaker {
    shared: Arc<Mutex<Shared>>,
    rate: u32,
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
                let _ = ready.send(Some((Arc::clone(&shared), rate)));
                mixer(&shared, rate, channels);
                drop(stream);
            })
            .ok()?;
        let (shared, rate) = answer.recv().ok().flatten()?;
        Some(Speaker { shared, rate })
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
