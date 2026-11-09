//! Where a section is as it plays, and how loud it is there (ADR 0056):
//! the clock of the progress bar and the speaker's bars, which follow the
//! loudness of the samples being played.

use std::time::{Duration, Instant};

use super::player::Pcm;

/// The speaker's bars.
pub const BARS: usize = 4;

/// The window of samples a level is of.
const WINDOW: Duration = Duration::from_millis(50);

/// Where a section is: from where it was started or sought to and since
/// when it has played on. The audio itself plays on a thread of its own;
/// this clock starts when the audio thread has handed the samples to the
/// device, so that it stays within the device's buffer of what is heard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clock {
    /// Where it was when it last started, paused or was sought.
    base: Duration,
    /// Since when it plays on; `None`: not started yet, or paused.
    since: Option<Instant>,
    paused: bool,
    /// How long the section is; `None`: not known yet.
    total: Option<Duration>,
}

impl Clock {
    /// The samples handed over at `now`: from the start (or where it was
    /// sought to before), paused if it was paused before.
    pub fn start(&mut self, total: Duration, now: Instant) {
        self.total = Some(total);
        self.base = self.base.min(total);
        self.since = (!self.paused).then_some(now);
    }

    pub fn at(&self, now: Instant) -> Duration {
        let on = self
            .since
            .map_or(Duration::ZERO, |s| now.saturating_duration_since(s));
        let at = self.base + on;
        self.total.map_or(at, |t| at.min(t))
    }

    pub fn total(&self) -> Option<Duration> {
        self.total
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn pause(&mut self, paused: bool, now: Instant) {
        if paused == self.paused {
            return;
        }
        if paused {
            self.base = self.at(now);
            self.since = None;
        } else if self.total.is_some() {
            self.since = Some(now);
        }
        self.paused = paused;
    }

    /// Sought to `to` at `now` (within the section).
    pub fn seek(&mut self, to: Duration, now: Instant) {
        self.base = self.total.map_or(to, |t| to.min(t));
        if self.since.is_some() {
            self.since = Some(now);
        }
    }
}

/// The root mean square of `samples` (all channels alike).
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// A root mean square as the height of a bar, 0 to 1: −48 dB (and below)
/// at 0, −12 dB (and above) at 1, as speech is heard.
fn height(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db + 48.0) / 36.0).clamp(0.0, 1.0)
}

/// How loud `pcm` is around `at`: the ~50 ms window about it in
/// [`BARS`] parts, each part's loudness a bar, 0 to 1.
pub fn levels(pcm: &Pcm, at: Duration) -> [f32; BARS] {
    let mut out = [0.0; BARS];
    let ch = usize::from(pcm.channels.max(1));
    let frames = pcm.samples.len() / ch;
    let rate = f64::from(pcm.rate.max(1));
    let window = (WINDOW.as_secs_f64() * rate) as usize;
    let mid = (at.as_secs_f64() * rate) as usize;
    let from = mid.saturating_sub(window / 2).min(frames);
    let to = (from + window).min(frames);
    let part = ((to - from) / BARS).max(1);
    for (i, bar) in out.iter_mut().enumerate() {
        let a = (from + i * part).min(to);
        let b = (a + part).min(to);
        *bar = height(rms(&pcm.samples[a * ch..b * ch]));
    }
    out
}

/// The bars' heights as shown: up fast (~30 ms), down slower (~120 ms),
/// so that they move with the voice and do not flicker.
pub fn smooth(shown: f32, target: f32, dt: f32) -> f32 {
    let tau = if target > shown { 0.03 } else { 0.12 };
    let k = 1.0 - (-dt.max(0.0) / tau).exp();
    shown + (target - shown) * k
}

/// The bars at rest: a still speaker, paused, idle or without animations.
pub const REST: [f32; BARS] = [0.0; BARS];

/// The speaker's bars as shown, from frame to frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Meter {
    pub shown: [f32; BARS],
    /// The time of the last frame (egui's `input.time`).
    last: Option<f64>,
}

impl Meter {
    /// At `now` (seconds), towards `target`; at rest at once without one.
    pub fn step(&mut self, target: Option<[f32; BARS]>, now: f64) -> [f32; BARS] {
        let Some(target) = target else {
            *self = Meter::default();
            return REST;
        };
        let dt = self.last.map_or(1.0, |l| (now - l).clamp(0.0, 0.1)) as f32;
        self.last = Some(now);
        for (s, t) in self.shown.iter_mut().zip(target) {
            *s = smooth(*s, t, dt);
        }
        self.shown
    }
}

/// `m:ss`, as the time beside the bar says it.
pub fn clock_text(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}
