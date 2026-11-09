//! A section's audio played: by rodio on a thread of its own, which opens
//! the audio device on the first play, so the window never waits for it.
//! The WAV is read here (16-bit or float PCM), a downloaded bundle's AAC
//! by symphonia. Without the feature `playback` nothing plays: the voice
//! says so.

use std::path::Path;
#[cfg(feature = "playback")]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "playback")]
use std::sync::Mutex;
#[cfg(feature = "playback")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "playback")]
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
#[cfg(feature = "playback")]
use std::time::Instant;

#[cfg(feature = "playback")]
use super::meter::Clock;
#[cfg(feature = "playback")]
use opencad_ui::i18n::trf;

/// What plays a WAV: the speakers, or a test's.
pub trait Play: Send {
    /// `wav` from its start; what played before stops.
    fn play(&mut self, wav: &Path);
    fn stop(&mut self);
    fn playing(&self) -> bool;
    /// Why the last play failed.
    fn error(&self) -> Option<String>;
    /// Where the section is and how long it is, once it plays.
    fn position(&self) -> Option<(Duration, Duration)> {
        None
    }
    /// From `to` on (while it plays or is paused).
    fn seek(&mut self, _to: Duration) {}
    fn pause(&mut self, _paused: bool) {}
    fn paused(&self) -> bool {
        false
    }
    /// The samples being played, for how loud they are.
    fn pcm(&self) -> Option<Arc<Pcm>> {
        None
    }
}

/// A WAV's samples: channels, rate and the samples interleaved.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<f32>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// A RIFF WAV of 16-bit integer or 32-bit float PCM.
///
/// # Errors
///
/// It is no such WAV.
pub fn read_wav(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a WAV".into());
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32_at(bytes, at + 4).ok_or("a short chunk")? as usize;
        let body = at + 8;
        let end = (body + len).min(bytes.len());
        match id {
            b"fmt " => {
                let tag = u16_at(bytes, body).ok_or("a short fmt")?;
                let channels = u16_at(bytes, body + 2).ok_or("a short fmt")?;
                let rate = u32_at(bytes, body + 4).ok_or("a short fmt")?;
                let bits = u16_at(bytes, body + 14).ok_or("a short fmt")?;
                // WAVE_FORMAT_EXTENSIBLE: the sub-format's first two bytes.
                let tag = if tag == 0xFFFE {
                    u16_at(bytes, body + 24).ok_or("a short fmt")?
                } else {
                    tag
                };
                format = Some((tag, channels, rate, bits));
            }
            b"data" => {
                let (tag, channels, rate, bits) = format.ok_or("data before fmt")?;
                if channels == 0 || rate == 0 {
                    return Err("no channels".into());
                }
                let data = &bytes[body..end];
                let samples = match (tag, bits) {
                    (1, 16) => data
                        .chunks_exact(2)
                        .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
                        .collect(),
                    (3, 32) => data
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect(),
                    _ => return Err(format!("format {tag} of {bits} bits")),
                };
                return Ok(Pcm {
                    channels,
                    rate,
                    samples,
                });
            }
            _ => {}
        }
        at = body + len + (len & 1);
    }
    Err("no data".into())
}

#[cfg(feature = "playback")]
/// AAC in an MP4 (a downloaded bundle's `audio.m4a`), decoded whole by
/// symphonia.
///
/// # Errors
///
/// It is no such file, or a packet does not decode.
pub fn read_m4a(bytes: Vec<u8>) -> Result<Pcm, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let stream = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let options = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &options, &MetadataOptions::default())
        .map_err(|e| e.to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio track")?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut spec = None;
    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet is skipped, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let s = *decoded.spec();
        spec.get_or_insert(s);
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, s);
        buf.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buf.samples());
    }
    let spec = spec.ok_or("no audio")?;
    Ok(Pcm {
        channels: u16::try_from(spec.channels.count()).unwrap_or(1).max(1),
        rate: spec.rate,
        samples,
    })
}

/// A section's audio: a WAV, or AAC in an MP4 (`.m4a`).
///
/// # Errors
///
/// It cannot be read or is neither.
pub fn read_audio(path: &Path) -> Result<Pcm, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if path.extension().is_some_and(|e| e == "m4a") {
        #[cfg(feature = "playback")]
        return read_m4a(bytes).map_err(|e| format!("{}: {e}", path.display()));
        #[cfg(not(feature = "playback"))]
        return Err(format!(
            "{}: {}",
            path.display(),
            opencad_ui::i18n::tr(Msg::NoPlayback)
        ));
    }
    read_wav(&bytes)
}

#[cfg(feature = "playback")]
enum Cmd {
    /// The WAV, as the play of that number.
    Play(PathBuf, u64),
    Stop,
    /// To where [`Tape::seek`] says (the last asked of a drag only).
    Seek,
    Pause(bool),
}

#[cfg(feature = "playback")]
/// What the window and the audio thread share of the section playing.
#[derive(Default)]
struct Tape {
    /// The number of the last play asked; the audio thread fills in only
    /// that one's.
    play: u64,
    clock: Clock,
    pcm: Option<Arc<Pcm>>,
    /// Where to seek to, not done yet.
    seek: Option<Duration>,
}

#[cfg(feature = "playback")]
/// The speakers, by rodio.
pub struct Speakers {
    tx: Sender<Cmd>,
    playing: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    tape: Arc<Mutex<Tape>>,
}

#[cfg(feature = "playback")]
impl std::fmt::Debug for Speakers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speakers")
            .field("playing", &self.playing())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "playback")]
impl Speakers {
    /// The speakers of the app `app` (its name: the audio thread's and its
    /// log lines').
    pub fn new(app: &str) -> Speakers {
        let (tx, rx) = channel();
        let playing = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let tape = Arc::new(Mutex::new(Tape::default()));
        let (p, e, t) = (Arc::clone(&playing), Arc::clone(&error), Arc::clone(&tape));
        let log = app.to_lowercase();
        let thread_log = log.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("{log}-audio"))
            .spawn(move || audio(&thread_log, &rx, &p, &e, &t));
        if let Err(e) = spawned {
            eprintln!("{log}: audio: {e}");
        }
        Speakers {
            tx,
            playing,
            error,
            tape,
        }
    }

    fn tape<R>(&self, f: impl FnOnce(&mut Tape) -> R) -> Option<R> {
        self.tape.lock().ok().map(|mut t| f(&mut t))
    }
}

#[cfg(feature = "playback")]
impl Play for Speakers {
    fn play(&mut self, wav: &Path) {
        self.playing.store(true, Ordering::SeqCst);
        if let Ok(mut e) = self.error.lock() {
            *e = None;
        }
        let play = self.tape(|t| {
            let play = t.play + 1;
            *t = Tape {
                play,
                ..Tape::default()
            };
            play
        });
        let _ = self
            .tx
            .send(Cmd::Play(wav.to_path_buf(), play.unwrap_or(0)));
    }

    fn stop(&mut self) {
        self.playing.store(false, Ordering::SeqCst);
        self.tape(|t| {
            t.clock = Clock::default();
            t.pcm = None;
            t.seek = None;
        });
        let _ = self.tx.send(Cmd::Stop);
    }

    fn playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    fn position(&self) -> Option<(Duration, Duration)> {
        let now = Instant::now();
        self.tape(|t| Some((t.clock.at(now), t.clock.total()?)))
            .flatten()
            .filter(|_| self.playing())
    }

    fn seek(&mut self, to: Duration) {
        if !self.playing() {
            return;
        }
        let now = Instant::now();
        self.tape(|t| {
            t.clock.seek(to, now);
            t.seek = Some(t.clock.at(now));
        });
        let _ = self.tx.send(Cmd::Seek);
    }

    fn pause(&mut self, paused: bool) {
        if !self.playing() {
            return;
        }
        let now = Instant::now();
        self.tape(|t| t.clock.pause(paused, now));
        let _ = self.tx.send(Cmd::Pause(paused));
    }

    fn paused(&self) -> bool {
        self.playing() && self.tape(|t| t.clock.paused()).unwrap_or(false)
    }

    fn pcm(&self) -> Option<Arc<Pcm>> {
        self.tape(|t| t.pcm.clone()).flatten()
    }
}

/// The audio thread: the output opened on the first play and kept while
/// the lesson's voice is in use (the thread ends with its `Speakers`, which
/// go when the voice is switched off or the lesson closes); opened again
/// only when its stream reported the device gone. A stream opened and
/// closed for each section raced the one before on some Macs.
#[cfg(feature = "playback")]
fn audio(
    log: &str,
    rx: &Receiver<Cmd>,
    playing: &AtomicBool,
    error: &Mutex<Option<String>>,
    tape: &Mutex<Tape>,
) {
    use rodio::{Player, buffer::SamplesBuffer};
    let mut out: Option<Output> = None;
    let mut player: Option<Player> = None;
    let fail = |e: String| {
        playing.store(false, Ordering::SeqCst);
        if let Ok(mut slot) = error.lock() {
            *slot = Some(e);
        }
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Cmd::Play(path, play)) => {
                if let Some(p) = player.take() {
                    p.stop();
                }
                let pcm = read_audio(&path);
                let pcm = match pcm {
                    Ok(p) => p,
                    Err(e) => {
                        fail(e);
                        continue;
                    }
                };
                // The device gone (headphones taken off): its stream
                // closed before another opens.
                if out.as_ref().is_some_and(Output::lost) {
                    drop(out.take());
                }
                if out.is_none() {
                    match open_output(log, &mut |line| eprintln!("{log}: audio: {line}")) {
                        Ok(o) => out = Some(o),
                        Err(e) => {
                            fail(trf(Msg::NoOutput, &[&e]));
                            continue;
                        }
                    }
                }
                let (Some(o), Some(ch), Some(rate)) = (
                    out.as_ref(),
                    std::num::NonZero::new(pcm.channels),
                    std::num::NonZero::new(pcm.rate),
                ) else {
                    continue;
                };
                let total = Duration::from_secs_f64(
                    (pcm.samples.len() / usize::from(pcm.channels)) as f64 / f64::from(pcm.rate),
                );
                let Ok(mut t) = tape.lock() else {
                    continue;
                };
                // Another play asked meanwhile: this one is not heard.
                if t.play != play {
                    continue;
                }
                let p = Player::connect_new(o.sink.mixer());
                if t.clock.paused() {
                    p.pause();
                }
                p.append(SamplesBuffer::new(ch, rate, pcm.samples.clone()));
                t.clock.start(total, Instant::now());
                t.pcm = Some(Arc::new(pcm));
                let to = t.seek.take();
                drop(t);
                if let Some(to) = to {
                    let _ = p.try_seek(to);
                }
                player = Some(p);
            }
            Ok(Cmd::Stop) => {
                if let Some(p) = player.take() {
                    p.stop();
                }
            }
            Ok(Cmd::Seek) => {
                let to = tape.lock().ok().and_then(|mut t| t.seek.take());
                if let (Some(p), Some(to)) = (&player, to) {
                    // Waits for the device to take the order (a buffer at
                    // most), here and not in the window.
                    let _ = p.try_seek(to);
                }
            }
            Ok(Cmd::Pause(paused)) => {
                if let Some(p) = &player {
                    if paused {
                        p.pause();
                    } else {
                        p.play();
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if player.as_ref().is_some_and(Player::empty) {
            player = None;
            playing.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(feature = "playback")]
/// An open output and whether its stream reported the device gone.
pub struct Output {
    pub sink: rodio::MixerDeviceSink,
    lost: Arc<AtomicBool>,
}

#[cfg(feature = "playback")]
impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("lost", &self.lost())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "playback")]
impl Output {
    fn lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }
}

/// Frames of about 200 ms at `rate` within what the device takes (rodio's
/// own aims at 50 ms, which runs dry while the app builds a model);
/// `None`: the device does not say, its own size is taken.
#[cfg(feature = "playback")]
pub fn buffer_frames(rate: u32, supported: &rodio::cpal::SupportedBufferSize) -> Option<u32> {
    match *supported {
        rodio::cpal::SupportedBufferSize::Range { min, max } if min <= max && max > 0 => {
            Some((rate / 5).clamp(min.max(1), max))
        }
        _ => None,
    }
}

#[cfg(feature = "playback")]
/// An error and its sources, as one line.
fn chain(e: &dyn std::error::Error) -> String {
    let mut line = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        line.push_str(": ");
        line.push_str(&s.to_string());
        source = s.source();
    }
    line
}

/// The default output: its own format with a buffer of about 200 ms within
/// the range it gives, else its own buffer size, else its other formats,
/// else rodio's way. A failed attempt holds nothing open (cpal disposes of
/// its audio unit before returning the error). Each attempt, named by the
/// device, is told to `say`; the error names every attempt's. A buffer run
/// dry costs a click at most: it is not printed.
///
/// # Errors
///
/// No attempt opened.
#[cfg(feature = "playback")]
pub fn open_output(log: &str, say: &mut dyn FnMut(&str)) -> Result<Output, String> {
    use rodio::DeviceSinkBuilder;
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let lost = Arc::new(AtomicBool::new(false));
    let callback = {
        let lost = Arc::clone(&lost);
        let log = log.to_owned();
        move |e: rodio::CpalError| {
            use rodio::CpalErrorKind as K;
            match e.kind() {
                K::Xrun => {}
                // AirPods taken off, another output chosen: opened again
                // with that one's format before the next section.
                K::DeviceNotAvailable | K::StreamInvalidated | K::DeviceChanged => {
                    lost.store(true, Ordering::SeqCst);
                    eprintln!("{log}: audio: {e}");
                }
                _ => eprintln!("{log}: audio: {e}"),
            }
        }
    };
    let mut failed: Vec<String> = Vec::new();
    let mut attempt = |what: &str,
                       r: Result<rodio::MixerDeviceSink, rodio::DeviceSinkError>,
                       say: &mut dyn FnMut(&str)| match r {
        Ok(mut s) => {
            s.log_on_drop(false);
            say(&format!("{what}: open"));
            Some(s)
        }
        Err(e) => {
            let line = format!("{what}: {}", chain(&e));
            say(&line);
            failed.push(line);
            None
        }
    };
    if let Some(d) = rodio::cpal::default_host().default_output_device() {
        let name = d
            .description()
            .map_or_else(|_| "the default output".to_owned(), |n| n.to_string());
        match d.default_output_config() {
            Err(e) => {
                let r = Err(rodio::DeviceSinkError::DefaultSinkConfigError(e));
                let _ = attempt(&format!("{name}: its format"), r, say);
            }
            Ok(config) => {
                say(&format!(
                    "{name}: {} Hz, {} channels, {:?}, buffer {:?}",
                    config.sample_rate(),
                    config.channels(),
                    config.sample_format(),
                    config.buffer_size()
                ));
                let base = || {
                    DeviceSinkBuilder::default()
                        .with_device(d.clone())
                        .with_supported_config(&config)
                        .with_error_callback(callback.clone())
                };
                if let Some(frames) = buffer_frames(config.sample_rate(), config.buffer_size()) {
                    let r = base()
                        .with_buffer_size(rodio::cpal::BufferSize::Fixed(frames))
                        .open_stream();
                    let what = format!("{name}: a buffer of {frames} frames");
                    if let Some(sink) = attempt(&what, r, say) {
                        return Ok(Output { sink, lost });
                    }
                }
                let r = base()
                    .with_buffer_size(rodio::cpal::BufferSize::Default)
                    .open_stream();
                if let Some(sink) = attempt(&format!("{name}: its own buffer"), r, say) {
                    return Ok(Output { sink, lost });
                }
            }
        }
        // The device's other formats, 48 kHz where it takes it.
        let others: Vec<_> = match d.supported_output_configs() {
            Ok(c) => c.collect(),
            Err(e) => {
                let r = Err(rodio::DeviceSinkError::SupportedConfigsError(e));
                let _ = attempt(&format!("{name}: its formats"), r, say);
                Vec::new()
            }
        };
        for range in others {
            let c = range
                .try_with_sample_rate(48_000)
                .unwrap_or_else(|| range.with_max_sample_rate());
            let what = format!(
                "{name}: {} Hz, {} channels, {:?}, its own buffer",
                c.sample_rate(),
                c.channels(),
                c.sample_format()
            );
            let r = DeviceSinkBuilder::default()
                .with_device(d.clone())
                .with_supported_config(&c)
                .with_error_callback(callback.clone())
                .open_stream();
            if let Some(sink) = attempt(&what, r, say) {
                return Ok(Output { sink, lost });
            }
        }
    } else {
        let _ = attempt(
            "the default output",
            Err(rodio::DeviceSinkError::NoDevice),
            say,
        );
    }
    let r = DeviceSinkBuilder::open_default_sink();
    if let Some(sink) = attempt("rodio's way", r, say) {
        return Ok(Output { sink, lost });
    }
    Err(failed.join("; "))
}

/// An app's `--audio-check`: the output opened as a lesson's voice opens
/// it and a tone of 0.5 s at 440 Hz played, each attempt printed. `app`
/// names the log lines of the device's errors.
///
/// # Errors
///
/// The output did not open, or this build cannot play.
#[cfg(feature = "playback")]
pub fn audio_check(app: &str) -> Result<(), String> {
    use rodio::Source;
    let out = open_output(&app.to_lowercase(), &mut |line| println!("{line}"))?;
    let player = rodio::Player::connect_new(out.sink.mixer());
    player.append(
        rodio::source::SineWave::new(440.0)
            .take_duration(Duration::from_millis(500))
            .amplify(0.2),
    );
    player.sleep_until_end();
    // What is still in the device's buffer heard too.
    std::thread::sleep(Duration::from_millis(300));
    println!("played 0.5 s at 440 Hz");
    Ok(())
}

/// Without the feature `playback`: nothing plays.
///
/// # Errors
///
/// Always: this build cannot play.
#[cfg(not(feature = "playback"))]
pub fn audio_check(_app: &str) -> Result<(), String> {
    Err(opencad_ui::i18n::tr(Msg::NoPlayback).to_owned())
}

/// The speakers of a build without the feature `playback`: every play
/// fails and says so.
#[cfg(not(feature = "playback"))]
#[derive(Debug, Default)]
pub struct Speakers {
    error: Option<String>,
}

#[cfg(not(feature = "playback"))]
impl Speakers {
    pub fn new(_app: &str) -> Speakers {
        Speakers::default()
    }
}

#[cfg(not(feature = "playback"))]
impl Play for Speakers {
    fn play(&mut self, _wav: &Path) {
        self.error = Some(opencad_ui::i18n::tr(Msg::NoPlayback).to_owned());
    }
    fn stop(&mut self) {}
    fn playing(&self) -> bool {
        false
    }
    fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

opencad_ui::messages! {
    enum Msg {
        NoOutput: "no sound, the audio output did not open ({0})", "kein Ton, die Audio-Ausgabe ging nicht auf ({0})";
        NoPlayback: "this build cannot play sound", "dieser Build kann keinen Ton abspielen";
    }
}
