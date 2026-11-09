//! The voice on a toy app's course (no GPU, no voice, no audio device):
//! what it finds, its caches, its queue, the bundles, the player's clock
//! and bars, and a speaker that reads a course by the recordings' voice
//! alone or by a local one. The synth and the player are the tests'.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::bundle::{Bundle, Downloads, Fetch, Loading, Manifest, unpack};
use super::discover::{self, Found, Places};
use super::key::{self, ElevenKey, FishKey};
use super::meter::{self, BARS, Clock, Meter, REST};
use super::player::{self, Pcm, Play};
use super::queue::{Done, Kill, Queue, Synth};
use super::{
    At, ElevenRun, Engine, MOVING, Narrate, Prefer, Section, Settings, Speaker, Speech, Status,
    TICKING,
};
use crate::{Config, Lang, Recorded, VoiceConfig};

/// The toy app: its German recordings by an ElevenLabs voice, never
/// looking on the machine.
static CONFIG: Config = Config {
    app: "Toy",
    voice: VoiceConfig {
        recorded: &[Recorded {
            lang: Lang::De,
            voice: "toyvoice",
            model: "eleven_v3",
            language: "de",
        }],
        machine: false,
        ..VoiceConfig::NONE
    },
};

/// A tiny AAC-LC in MP4 (0.25 s of 440 Hz, 24 kHz mono), as a bundle's
/// sections are encoded.
const TONE: &[u8] = include_bytes!("tone.m4a");

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("egui-lessons-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

/// A machine with mlx-speech, S2 Pro's model and a voices folder; and the
/// tutorial venv with Kokoro.
fn fake_machine(dir: &Path, fish: bool, kokoro: bool) -> Places {
    let home = dir.join("home");
    let python = dir.join("mlx/bin/python");
    let w = |p: &Path, b: &[u8]| {
        std::fs::create_dir_all(p.parent().expect("a folder")).expect("a folder");
        std::fs::write(p, b).expect("written");
    };
    if fish {
        w(&python, b"");
        w(
            &home.join(".local/bin/mlx-speech"),
            format!("#!{}\nimport sys\n", python.display()).as_bytes(),
        );
        w(
            &home.join(
                ".cache/huggingface/hub/models--appautomaton--fishaudio-s2-pro-8bit-mlx/snapshots/1/x",
            ),
            b"",
        );
        let voices = home.join("dev/tts-voices");
        w(&voices.join("fish-s2-pro/en-gb-female.wav"), b"RIFF en");
        w(&voices.join("fish-s2-pro/de-male.wav"), b"RIFF de");
        w(
            &voices.join("voices.yaml"),
            b"dir: fish-s2-pro\nvoices:\n  en-gb-female: {language: English, reference: en-gb-female.wav, transcript: Hello there.}\n  de-male: {language: German, reference: de-male.wav, transcript: Hallo du.}\n",
        );
    }
    let tutorials = dir.join("tutorials");
    if kokoro {
        w(&tutorials.join("venv/bin/python"), b"");
        w(
            &tutorials.join("hf/hub/models--hexgrad--Kokoro-82M/snapshots/1/x"),
            b"",
        );
    }
    Places {
        voices: Some(home.join("dev/tts-voices")),
        hf_hub: Some(home.join(".cache/huggingface/hub")),
        home: Some(home),
        mlx_speech: None,
        fish_python: None,
        tutorials: kokoro.then_some(tutorials),
        recorded: Vec::new(),
    }
}

/// A WAV of a few samples.
fn tiny_wav() -> Vec<u8> {
    let data: Vec<u8> = [0i16, 1000, -1000, 0]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24_000u32.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    b
}

fn a_section(key: &str) -> Section {
    Section {
        engine: Engine::Fish,
        key: key.into(),
        text: format!("text {key}"),
        lang: Lang::En,
        voice: "v".into(),
        speech: Speech::Kokoro {
            voice: "v".into(),
            speed: 1.0,
        },
    }
}

/// The tests' synth: says what it spoke, waits for a gate before each.
#[derive(Clone, Default)]
struct Fake {
    spoken: Arc<Mutex<Vec<String>>>,
    gate: Arc<(Mutex<usize>, std::sync::Condvar)>,
    gated: bool,
    killed: Arc<AtomicUsize>,
}

impl Fake {
    fn open(&self, n: usize) {
        *self.gate.0.lock().expect("the gate") += n;
        self.gate.1.notify_all();
    }

    fn spoken(&self) -> Vec<String> {
        self.spoken.lock().expect("spoken").clone()
    }

    fn make(&self) -> (Box<dyn Synth>, Kill) {
        let k = Arc::clone(&self.killed);
        (
            Box::new(self.clone()),
            Arc::new(move || {
                k.fetch_add(1, Ordering::SeqCst);
            }),
        )
    }
}

impl Synth for Fake {
    fn speak(&mut self, s: &Section, out: &Path) -> Result<(), String> {
        if self.gated {
            let mut open = self.gate.0.lock().expect("the gate");
            while *open == 0 {
                open = self.gate.1.wait(open).expect("the gate");
            }
            *open -= 1;
        }
        self.spoken.lock().expect("spoken").push(s.key.clone());
        if s.key.starts_with("bad") {
            return Err("no voice".into());
        }
        std::fs::write(out, tiny_wav()).map_err(|e| e.to_string())
    }

    fn close(&mut self) {}
}

/// The tests' player: what it was asked.
#[derive(Clone, Default)]
struct Ears {
    log: Arc<Mutex<Vec<String>>>,
}

impl Play for Ears {
    fn play(&mut self, wav: &Path) {
        self.log
            .lock()
            .expect("log")
            .push(format!("play {}", wav.display()));
    }
    fn stop(&mut self) {
        self.log.lock().expect("log").push("stop".into());
    }
    fn playing(&self) -> bool {
        (self.log.lock().expect("log").last()).is_some_and(|l| l.starts_with("play"))
    }
    fn error(&self) -> Option<String> {
        None
    }
}

impl Ears {
    fn plays(&self) -> Vec<String> {
        (self.log.lock().expect("log").iter())
            .filter_map(|l| l.strip_prefix("play ").map(str::to_owned))
            .collect()
    }
}

/// The toy app's course "toy": three steps, the second says nothing.
/// Recorded by ElevenLabs in a language with its voice; spoken by S2 Pro
/// where found.
#[derive(Debug, Default)]
struct Toy;

const STEPS: [&str; 3] = ["Open [pause] the file.", "", "Then save it."];

impl Narrate for Toy {
    fn recorded(
        &mut self,
        course: &str,
        lang: Lang,
        eleven: Option<(&str, ElevenRun<'_>)>,
    ) -> Option<Vec<Option<Section>>> {
        (course == "toy").then(|| {
            STEPS
                .iter()
                .map(|t| {
                    let (voice, run) = eleven?;
                    (!t.is_empty()).then(|| eleven_of(t, lang, voice, run))
                })
                .collect()
        })
    }

    fn local(
        &mut self,
        course: &str,
        lang: Lang,
        step: usize,
        found: &Found,
        _: Prefer,
    ) -> Option<Section> {
        let text = STEPS
            .get(step)
            .filter(|t| course == "toy" && !t.is_empty())?;
        let v = (found.fish.as_ref()?.voices.iter()).find(|v| v.lang == lang)?;
        let k = FishKey {
            reference: v.hash.clone(),
            transcript: v.transcript.clone(),
            language: v.language.clone(),
            seed: key::FISH_SEED,
        };
        Some(Section {
            engine: Engine::Fish,
            key: k.key(text),
            text: (*text).to_owned(),
            lang,
            voice: v.name.clone(),
            speech: Speech::Fish {
                reference: v.reference.clone(),
                transcript: v.transcript.clone(),
                language: v.language.clone(),
                seed: key::FISH_SEED,
            },
        })
    }
}

fn eleven_of(text: &str, lang: Lang, voice: &str, run: ElevenRun<'_>) -> Section {
    let mut k = ElevenKey::new(voice, 1.0);
    if let Some(m) = run.model {
        m.clone_into(&mut k.model_id);
    }
    k.language_code = key::eleven_language(run.language, None, lang);
    Section {
        engine: Engine::ElevenLabs,
        key: k.key(text),
        text: key::untagged(text),
        lang,
        voice: voice.to_owned(),
        speech: Speech::Recorded,
    }
}

/// The toy's sections in German as its recordings' voice said them.
fn german_keys() -> Vec<String> {
    let settings = Settings::new(CONFIG.voice.recorded);
    let eleven = settings
        .eleven(Lang::De)
        .map(|v| (v, ElevenRun::of(&settings, Lang::De)));
    (Toy.recorded("toy", Lang::De, eleven).expect("the toy"))
        .into_iter()
        .flatten()
        .map(|s| s.key)
        .collect()
}

/// A speaker of the toy on a fake machine (S2 Pro found), the synth and
/// the player the tests'; its settings in `dir`.
fn speaker(dir: &Path, fake: &Fake, ears: &Ears) -> Speaker {
    let mut v = Speaker::new(&CONFIG, Box::new(Toy));
    v.places = Some(fake_machine(dir, true, false));
    v.cache = Some(dir.join("cache"));
    let f = fake.clone();
    v.make_synth = Arc::new(move |_: &Found, _: &Path| f.make());
    let e = ears.clone();
    v.make_player = Arc::new(move || Box::new(e.clone()));
    v.load(&dir.join("config"));
    v
}

fn at(step: usize, lang: Lang) -> Option<At> {
    Some(At {
        course: "toy".into(),
        step,
        lang,
    })
}

/// Frames until `ok`.
fn until(v: &mut Speaker, now: &Option<At>, what: &str, ok: impl Fn(&Speaker) -> bool) {
    let ctx = egui::Context::default();
    for _ in 0..600 {
        v.frame(now.clone(), &ctx);
        if ok(v) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("never: {what}");
}

#[test]
fn voices_are_found_where_they_are() {
    let dir = scratch("discover");
    let found = discover::discover(&fake_machine(&dir, true, true));
    let fish = found.fish.as_ref().expect("S2 Pro");
    assert_eq!(fish.python, dir.join("mlx/bin/python"));
    let names: Vec<(&str, Option<&str>)> = (fish.voices.iter())
        .map(|v| (v.name.as_str(), v.language.as_deref()))
        .collect();
    assert_eq!(names, [("en-gb-female", None), ("de-male", Some("German"))]);
    assert_eq!(fish.voices[0].hash, key::sha256(b"RIFF en"));
    assert!(found.kokoro.is_some());
    assert!(found.speaks(Lang::En) && found.speaks(Lang::De));

    let kokoro_only = discover::discover(&fake_machine(&scratch("discover-k"), false, true));
    assert!(kokoro_only.fish.is_none());
    assert!(kokoro_only.speaks(Lang::En) && !kokoro_only.speaks(Lang::De));
    assert!(kokoro_only.missing[0].contains("mlx-speech is not installed"));

    let none = discover::discover(&fake_machine(&scratch("discover-0"), false, false));
    assert!(!none.any());
    assert_eq!(none.missing.len(), 2);

    // The model not downloaded: no S2 Pro, and why.
    let d = scratch("discover-m");
    let places = fake_machine(&d, true, false);
    std::fs::remove_dir_all(places.hf_hub.as_ref().expect("a hub")).expect("removed");
    let found = discover::discover(&places);
    assert!(found.fish.is_none());
    assert!(found.missing[0].contains("not in the Hugging Face cache"));
}

/// The app's work folder: its variable, else its folder where it exists.
#[test]
fn the_work_folder_is_the_apps() {
    let dir = scratch("work");
    static WORK: Config = Config {
        app: "Toy",
        voice: VoiceConfig {
            work_env: Some("EGUI_LESSONS_TEST_WORK_UNSET"),
            work_dir: Some("/nonexistent/egui-lessons/target/tutorials"),
            ..VoiceConfig::NONE
        },
    };
    assert_eq!(Places::here(&WORK, None).tutorials, None);
    let leaked: &'static str = Box::leak(dir.display().to_string().into_boxed_str());
    let config: &'static Config = Box::leak(Box::new(Config {
        app: "Toy",
        voice: VoiceConfig {
            work_dir: Some(leaked),
            ..VoiceConfig::NONE
        },
    }));
    assert_eq!(Places::here(config, None).tutorials, Some(dir));
}

#[test]
fn wavs_are_read() {
    let pcm = player::read_wav(&tiny_wav()).expect("a WAV");
    assert_eq!((pcm.channels, pcm.rate, pcm.samples.len()), (1, 24_000, 4));
    assert!((pcm.samples[1] - 1000.0 / 32768.0).abs() < 1e-6);
    assert!(player::read_wav(b"nothing").is_err());
}

/// The output's buffer: about 200 ms, within what the device takes (a
/// Mac's speakers take 15 to 4096 frames, so 8192 never opened; AirPods
/// too), else the device's own.
#[cfg(feature = "playback")]
#[test]
fn the_buffer_is_one_the_device_takes() {
    use rodio::cpal::SupportedBufferSize::{Range, Unknown};
    let mac = Range { min: 15, max: 4096 };
    assert_eq!(player::buffer_frames(48_000, &mac), Some(4096));
    assert_eq!(player::buffer_frames(192_000, &mac), Some(4096));
    let wide = Range {
        min: 64,
        max: 16_384,
    };
    assert_eq!(player::buffer_frames(48_000, &wide), Some(9600));
    assert_eq!(player::buffer_frames(44_100, &wide), Some(8820));
    let small = Range {
        min: 256,
        max: 1024,
    };
    assert_eq!(player::buffer_frames(48_000, &small), Some(1024));
    assert_eq!(player::buffer_frames(1000, &small), Some(256));
    assert_eq!(player::buffer_frames(48_000, &Unknown), None);
    assert_eq!(
        player::buffer_frames(48_000, &Range { min: 0, max: 0 }),
        None
    );
}

fn wait_for(q: &Queue, n: usize) -> Vec<Done> {
    let mut all = Vec::new();
    for _ in 0..500 {
        all.extend(q.poll());
        if all.len() >= n {
            return all;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("only {} of {n} spoken", all.len());
}

#[test]
fn the_queue_speaks_the_list_asked_last_in_order() {
    let root = scratch("queue");
    let fake = Fake {
        gated: true,
        ..Fake::default()
    };
    let (synth, kill) = fake.make();
    let q = Queue::new("Toy", root.clone(), synth, kill);
    q.want(["a", "b", "c"].map(a_section).to_vec());
    // `a` is taken; then the step changes: d first, b after, c no more.
    for _ in 0..200 {
        if q.busy().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(q.busy().as_deref(), Some("a"));
    q.want(["a", "d", "b"].map(a_section).to_vec());
    assert_eq!(
        q.pending(),
        ["d", "b"],
        "the one being spoken is not asked again"
    );
    fake.open(3);
    let done = wait_for(&q, 3);
    assert_eq!(fake.spoken(), ["a", "d", "b"]);
    for d in &done {
        let wav = d.result.as_ref().expect("spoken");
        assert_eq!(wav, &root.join(&d.key).join("audio.wav"));
        let meta = std::fs::read_to_string(root.join(&d.key).join("meta.json")).expect("meta");
        assert!(meta.contains("Toy's lessons"), "{meta}");
    }
    // Dropped (the course closed): what waits is not spoken, the worker
    // is killed.
    q.want(["e", "f"].map(a_section).to_vec());
    let killed = Arc::clone(&fake.killed);
    drop(q);
    fake.open(5);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(killed.load(Ordering::SeqCst), 1);
    assert!(fake.spoken().len() <= 4, "{:?}", fake.spoken());
}

#[test]
fn a_failure_is_said_back() {
    let fake = Fake::default();
    let (synth, kill) = fake.make();
    let q = Queue::new("Toy", scratch("queue-bad"), synth, kill);
    q.want(vec![a_section("bad1")]);
    let done = wait_for(&q, 1);
    assert_eq!(done[0].result, Err("no voice".into()));
}

#[test]
fn today_is_a_date() {
    let t = super::queue::today();
    assert_eq!(t.len(), 10, "{t}");
    assert!(
        t.starts_with("20") && &t[4..5] == "-" && &t[7..8] == "-",
        "{t}"
    );
}

#[test]
fn the_recordings_are_found_before_the_apps_cache() {
    let dir = scratch("cache");
    let tutorials = dir.join("tutorials");
    let lasting = dir.join("tutorial-voices");
    let app = dir.join("app");
    let found = discover::discover(&Places {
        tutorials: Some(tutorials.clone()),
        recorded: vec![lasting.clone()],
        ..Places::default()
    });
    assert_eq!(found.recorded, [lasting.clone(), tutorials.join("voices")]);
    let s = a_section("k1");
    assert_eq!(super::cache::find(&s, &found, Some(&app)), None);
    let put = |p: &Path| {
        std::fs::create_dir_all(p.parent().expect("a folder")).expect("a folder");
        std::fs::write(p, tiny_wav()).expect("written");
    };
    let mine = app.join("k1/audio.wav");
    put(&mine);
    assert_eq!(super::cache::find(&s, &found, Some(&app)), Some(mine));
    let recorded = tutorials.join("voices/k1/audio.wav");
    put(&recorded);
    assert_eq!(super::cache::find(&s, &found, Some(&app)), Some(recorded));
    // The lasting cache before the work folder's.
    let kept = lasting.join("k1/audio.wav");
    put(&kept);
    assert_eq!(super::cache::find(&s, &found, Some(&app)), Some(kept));
    // Kokoro's: `<work>/audio/<key>.wav`.
    let k = Section {
        engine: Engine::Kokoro,
        ..a_section("k2")
    };
    let kokoro = tutorials.join("audio/k2.wav");
    std::fs::create_dir_all(kokoro.parent().expect("a folder")).expect("a folder");
    std::fs::write(&kokoro, tiny_wav()).expect("written");
    assert_eq!(super::cache::find(&k, &found, None), Some(kokoro));
}

/// The recordings' lasting cache is not the app's own, which is the only
/// one written; both named by the app.
#[test]
fn the_lasting_cache_is_not_the_apps() {
    let lasting = super::cache::lasting_dir("OpenSolve");
    assert!(
        lasting
            .as_ref()
            .is_some_and(|d| d.ends_with("tutorial-voices"))
    );
    let named = lasting.as_ref().is_some_and(|d| {
        let s = d.display().to_string();
        s.contains("OpenSolve") || s.contains("opensolve")
    });
    assert!(named, "{lasting:?}");
    let app = super::cache::app_dir("OpenSolve");
    assert!(
        app.as_ref()
            .is_some_and(|d| d.ends_with("opensolve/voices")),
        "{app:?}"
    );
    assert_ne!(lasting, app);
    assert_eq!(
        super::cache::recorded_dirs("OpenSolve").first(),
        lasting.as_ref()
    );
}

/// The recordings' ElevenLabs voices are kept in `voice.json`; never set,
/// a language has the app's and English none.
#[test]
fn the_elevenlabs_voices_are_remembered() {
    let dir = scratch("eleven-settings");
    let s = Settings::load(&dir, CONFIG.voice.recorded);
    assert_eq!(s.eleven(Lang::De), Some("toyvoice"));
    assert_eq!(s.eleven(Lang::En), None);
    assert_eq!(
        (s.eleven_model(Lang::De), s.eleven_language(Lang::De)),
        (Some("eleven_v3"), Some("de"))
    );
    assert_eq!(
        (s.eleven_model(Lang::En), s.eleven_language(Lang::En)),
        (None, None)
    );
    let set = Settings {
        eleven_de: Some(String::new()),
        eleven_en: Some("abc".into()),
        eleven_model_de: Some(String::new()),
        eleven_model_en: Some("eleven_v3".into()),
        eleven_language_en: Some("auto".into()),
        ..s
    };
    set.save(&dir, "Toy");
    let back = Settings::load(&dir, CONFIG.voice.recorded);
    assert_eq!(back, set);
    assert_eq!(
        (back.eleven(Lang::De), back.eleven(Lang::En)),
        (None, Some("abc"))
    );
    assert_eq!(
        (back.eleven_model(Lang::De), back.eleven_model(Lang::En)),
        (None, Some("eleven_v3"))
    );
    assert_eq!(
        (
            back.eleven_language(Lang::De),
            back.eleven_language(Lang::En)
        ),
        (Some("de"), Some("auto"))
    );
    // An app without recordings: none in any language.
    assert_eq!(Settings::load(&dir, &[]).eleven_model(Lang::De), None);
}

/// A course read by a local voice (no recordings of it): its step spoken
/// in the background and played when ready, the next ones after it, the
/// step that says nothing silent; switched off, the worker ends.
#[test]
fn a_course_without_recordings_is_spoken_here() {
    opencad_ui::i18n::set_thread_lang(Lang::En);
    let dir = scratch("local");
    let (fake, ears) = (Fake::default(), Ears::default());
    let mut v = speaker(&dir, &fake, &ears);
    assert!(v.available(Lang::En) && v.on(Lang::En));
    let now = at(0, Lang::En);
    until(&mut v, &now, "the step played", |_| ears.plays().len() == 1);
    let first = v.current.clone().expect("a section");
    assert_eq!(first.engine, Engine::Fish);
    assert_eq!(fake.spoken().first(), Some(&first.key));
    assert!(ears.plays()[0].contains(&first.key));
    assert!(v.info().contains("Fish Audio S2 Pro"), "{}", v.info());
    // The step after it, spoken ahead.
    until(&mut v, &now, "spoken ahead", |_| fake.spoken().len() == 2);

    // The step that says nothing: no section, nothing played.
    let silent = at(1, Lang::En);
    until(&mut v, &silent, "the step open", |v| v.current.is_none());
    assert_eq!(v.status, Status::Quiet);
    assert_eq!(ears.plays().len(), 1);

    v.switch(false);
    assert!(v.queue().is_none());
    // Remembered.
    let back = Settings::load(&dir.join("config"), CONFIG.voice.recorded);
    assert_eq!(back.on, Some(false));
}

/// A course with recordings is read by their voice alone: the recorded
/// step plays from the recordings' cache; a step not recorded stays
/// silent and says so, never read by another voice.
#[test]
fn one_speaker_per_lesson() {
    opencad_ui::i18n::set_thread_lang(Lang::De);
    let dir = scratch("one-speaker");
    let (fake, ears) = (Fake::default(), Ears::default());
    let lasting = dir.join("tutorial-voices");
    let mut v = speaker(&dir, &fake, &ears);
    if let Some(p) = v.places.as_mut() {
        p.recorded = vec![lasting.clone()];
    }
    let keys = german_keys();
    assert_eq!(keys.len(), 2);
    let wav = lasting.join(&keys[0]).join("audio.wav");
    std::fs::create_dir_all(wav.parent().expect("a folder")).expect("a folder");
    std::fs::write(&wav, tiny_wav()).expect("written");

    let now = at(0, Lang::De);
    until(&mut v, &now, "the recorded step played", |_| {
        ears.plays().len() == 1
    });
    assert_eq!(ears.plays()[0], wav.display().to_string());
    assert_eq!(
        v.current.as_ref().map(|c| c.engine),
        Some(Engine::ElevenLabs)
    );
    assert!(v.info().contains("ElevenLabs"), "{}", v.info());

    let unrecorded = at(2, Lang::De);
    until(&mut v, &unrecorded, "silent", |v| {
        v.status == Status::Unrecorded
    });
    assert!(v.current.is_none());
    assert!(v.info().contains("keine Aufnahme"), "{}", v.info());
    assert!(fake.spoken().is_empty(), "no second voice");
    assert!(v.queue().is_none());
    assert_eq!(ears.plays().len(), 1);
}

/// A tar of `files` (path, bytes), as the build of the bundles writes one.
fn tar_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut t = tar::Builder::new(Vec::new());
    for (path, data) in files {
        let mut h = tar::Header::new_ustar();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        // The name as written, `..` too (tar's own setter refuses it).
        h.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        h.set_cksum();
        t.append(&h, data.as_slice()).expect("appended");
    }
    t.into_inner().expect("a tar")
}

/// A bundle of `keys`, each with the tone and its JSON, and its entry.
fn bundle_of(course: &str, lang: &str, keys: &[String]) -> (Bundle, Vec<u8>) {
    let mut files = Vec::new();
    for k in keys {
        files.push((format!("{k}/audio.m4a"), TONE.to_vec()));
        files.push((format!("{k}/words.json"), b"[]".to_vec()));
        files.push((format!("{k}/meta.json"), b"{}".to_vec()));
    }
    let bytes = tar_of(&files);
    let b = Bundle {
        course: course.into(),
        lang: lang.into(),
        file: format!("{course}.{lang}.000000000000.tar"),
        sha256: key::sha256(&bytes),
        size: bytes.len() as u64,
        keys: keys.to_vec(),
    };
    (b, bytes)
}

#[test]
fn a_malformed_manifest_is_refused() {
    let good = r#"{"format": 1, "base_url": "https://x", "bundles": [{"course": "c", "lang": "de",
        "file": "c.de.abc.tar", "sha256": "SHA", "size": 10, "sections": 1, "keys": ["ab12"]}]}"#;
    let sha = "a".repeat(64);
    let m = Manifest::parse(&good.replace("SHA", &sha)).expect("reads");
    assert_eq!(m.bundles[0].keys, vec!["ab12".to_owned()]);
    assert_eq!(m.url(&m.bundles[0]), "https://x/c.de.abc.tar");
    assert_eq!(m.of("c", Lang::De), Some(&m.bundles[0]));
    assert!(m.any(Lang::De) && !m.any(Lang::En));
    for bad in [
        good.replace("SHA", "short"),
        good.replace("SHA", &sha)
            .replace("c.de.abc.tar", "../c.tar"),
        good.replace("SHA", &sha).replace("c.de.abc.tar", "a/c.tar"),
        good.replace("SHA", &sha).replace("\"ab12\"", "\"../x\""),
        good.replace("SHA", &sha)
            .replace("\"format\": 1", "\"format\": 2"),
        "not json".to_owned(),
    ] {
        assert!(Manifest::parse(&bad).is_err(), "{bad}");
    }
}

/// The app's manifest, its base replaced by the app's variable; none
/// without one, or looking nowhere.
#[test]
fn the_apps_manifest_is_downloaded_from() {
    let sha = "b".repeat(64);
    let manifest: &'static str = Box::leak(
        format!(
            r#"{{"format": 1, "base_url": "https://x", "bundles": [{{"course": "c", "lang": "de",
            "file": "c.de.abc.tar", "sha256": "{sha}", "size": 10, "keys": ["ab12"]}}]}}"#
        )
        .into_boxed_str(),
    );
    let config: &'static Config = Box::leak(Box::new(Config {
        app: "Toy",
        voice: VoiceConfig {
            bundles: manifest,
            bundles_env: Some("EGUI_LESSONS_TEST_BUNDLES_UNSET"),
            ..VoiceConfig::NONE
        },
    }));
    let d = Downloads::here(config).expect("downloads");
    assert_eq!(
        d.manifest.url(&d.manifest.bundles[0]),
        "https://x/c.de.abc.tar"
    );
    assert!(d.root.ends_with("tutorial-voices"));
    assert!(Downloads::here(&CONFIG).is_none(), "no manifest");
    // Not looking on the machine (an app's tests): no downloads.
    let off: &'static Config = Box::leak(Box::new(Config {
        voice: VoiceConfig {
            machine: false,
            ..config.voice
        },
        ..*config
    }));
    assert!(Speaker::new(off, Box::new(Toy)).bundles.is_none());
}

/// A bundle is checked before anything is written; then each section goes
/// into a folder of its own, one already there kept as it is.
#[test]
fn a_bundle_is_checked_and_unpacked() {
    let dir = scratch("bundle-unpack");
    let root = dir.join("tutorial-voices");
    let keys = vec!["aa11".to_owned(), "bb22".to_owned()];
    let (b, bytes) = bundle_of("c", "de", &keys);

    // Another file than the manifest's: refused, nothing written.
    let mut other = bytes.clone();
    let last = other.len() - 1;
    other[last] ^= 1;
    assert!(unpack(&other, &b, &root).is_err());
    assert!(unpack(&bytes[..bytes.len() - 1], &b, &root).is_err());
    assert!(!root.exists());

    // A section there already (a recording's) is left alone.
    std::fs::create_dir_all(root.join("aa11")).expect("a folder");
    std::fs::write(root.join("aa11/audio.wav"), tiny_wav()).expect("written");
    assert_eq!(unpack(&bytes, &b, &root), Ok(1));
    assert!(!root.join("aa11/audio.m4a").exists());
    for f in ["audio.m4a", "words.json", "meta.json"] {
        assert!(root.join("bb22").join(f).is_file(), "{f}");
    }
    assert_eq!(
        std::fs::read(root.join("bb22/audio.m4a")).expect("read"),
        TONE
    );
    // No leftovers.
    let names: BTreeSet<String> = std::fs::read_dir(&root)
        .expect("the cache")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, ["aa11", "bb22"].map(String::from).into());
    // Again: nothing new.
    assert_eq!(unpack(&bytes, &b, &root), Ok(0));
}

/// Only its own sections' files: a path elsewhere, or a key the manifest
/// does not list, refuses the bundle.
#[test]
fn a_bundle_with_strange_files_is_refused() {
    let root = scratch("bundle-strange").join("tutorial-voices");
    for path in ["../evil/audio.m4a", "zz99/audio.m4a", "aa11/run.sh", "aa11"] {
        let bytes = tar_of(&[(path.to_owned(), TONE.to_vec())]);
        let b = Bundle {
            course: "c".into(),
            lang: "de".into(),
            file: "c.de.x.tar".into(),
            sha256: key::sha256(&bytes),
            size: bytes.len() as u64,
            keys: vec!["aa11".into()],
        };
        assert!(unpack(&bytes, &b, &root).is_err(), "{path}");
    }
    assert!(!root.exists() || std::fs::read_dir(&root).expect("a dir").next().is_none());
    assert!(!root.parent().expect("a folder").join("evil").exists());
}

/// The sections' AAC decodes: its rate, one channel, about its length, a
/// tone and not silence.
#[cfg(feature = "playback")]
#[test]
fn aac_decodes() {
    let pcm = player::read_m4a(TONE.to_vec()).expect("decoded");
    assert_eq!((pcm.channels, pcm.rate), (1, 24_000));
    let secs = pcm.samples.len() as f32 / 24_000.0;
    assert!((0.24..0.45).contains(&secs), "{secs} s");
    let rms = (pcm.samples.iter().map(|s| s * s).sum::<f32>() / pcm.samples.len() as f32).sqrt();
    assert!(rms > 0.1, "{rms}");
    assert!(player::read_m4a(b"not an mp4".to_vec()).is_err());
    // Played by its extension.
    let dir = scratch("bundle-aac");
    std::fs::write(dir.join("audio.m4a"), TONE).expect("written");
    assert_eq!(
        player::read_audio(&dir.join("audio.m4a")).expect("read"),
        pcm
    );
}

/// A fetcher that serves `bytes` (or fails with `fail`), counting.
fn fetcher(bytes: Vec<u8>, fail: Option<&str>, asked: &Arc<AtomicUsize>) -> Fetch {
    let asked = Arc::clone(asked);
    let fail = fail.map(str::to_owned);
    Arc::new(move |_url: &str, progress: &dyn Fn(u64, Option<u64>)| {
        asked.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = &fail {
            return Err(e.clone());
        }
        progress(bytes.len() as u64 / 2, Some(bytes.len() as u64));
        Ok(bytes.clone())
    })
}

fn wait(d: &Downloads, file: &str) -> Loading {
    for _ in 0..500 {
        match d.state(file) {
            Some(Loading::Running { .. }) | None => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Some(s) => return s,
        }
    }
    panic!("never done");
}

/// Downloaded once: asked again, nothing is fetched; a failure is kept
/// until it is forgotten, then tried again.
#[test]
fn a_bundle_is_downloaded_once() {
    let root = scratch("bundle-once").join("tutorial-voices");
    let (b, bytes) = bundle_of("c", "de", &["aa11".to_owned()]);
    let asked = Arc::new(AtomicUsize::new(0));
    let m = Manifest {
        base_url: "https://example.invalid".into(),
        bundles: vec![b.clone()],
    };
    let d = Downloads::new(
        m.clone(),
        root.clone(),
        fetcher(bytes.clone(), None, &asked),
    );
    d.want(&b);
    assert_eq!(wait(&d, &b.file), Loading::Done);
    d.want(&b);
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert!(root.join("aa11/audio.m4a").is_file());

    let failing = Arc::new(AtomicUsize::new(0));
    let root = scratch("bundle-offline").join("tutorial-voices");
    let d = Downloads::new(m, root.clone(), fetcher(bytes, Some("offline"), &failing));
    d.want(&b);
    assert_eq!(wait(&d, &b.file), Loading::Failed("offline".into()));
    d.want(&b);
    assert_eq!(failing.load(Ordering::SeqCst), 1, "not again while open");
    d.forget_failed();
    d.want(&b);
    let _ = wait(&d, &b.file);
    assert_eq!(failing.load(Ordering::SeqCst), 2);
    assert!(!root.exists());
}

/// The toy's German course on a machine without its sections: its bundle
/// downloaded in the background, the step silent and loading meanwhile,
/// then played from it, AAC; closed and opened again, nothing fetched.
/// Offline, it says so and stays silent.
#[test]
fn a_lesson_downloads_its_voice_once() {
    opencad_ui::i18n::set_thread_lang(Lang::De);
    let dir = scratch("bundle-lesson");
    let (fake, ears) = (Fake::default(), Ears::default());
    let lasting = dir.join("tutorial-voices");
    let (b, bytes) = bundle_of("toy", "de", &german_keys());
    let gate = Arc::new(Mutex::new(false));
    let asked = Arc::new(AtomicUsize::new(0));
    let fetch: Fetch = {
        let (asked, gate) = (Arc::clone(&asked), Arc::clone(&gate));
        Arc::new(move |_url: &str, progress: &dyn Fn(u64, Option<u64>)| {
            asked.fetch_add(1, Ordering::SeqCst);
            progress(1, Some(bytes.len() as u64));
            while !*gate.lock().expect("the gate") {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(bytes.clone())
        })
    };
    let manifest = Manifest {
        base_url: "https://example.invalid".into(),
        bundles: vec![b],
    };
    let with_bundles = |v: &mut Speaker, fetch: Fetch| {
        if let Some(p) = v.places.as_mut() {
            p.recorded = vec![lasting.clone()];
        }
        v.bundles = Some(Downloads::new(manifest.clone(), lasting.clone(), fetch));
    };
    let mut v = speaker(&dir, &fake, &ears);
    with_bundles(&mut v, fetch);
    assert!(v.available(Lang::De));
    let now = at(0, Lang::De);
    until(&mut v, &now, "loading shown", |v| {
        matches!(v.status, Status::Loading { .. })
    });
    assert!(v.info().contains("wird geladen"), "{}", v.info());
    *gate.lock().expect("the gate") = true;
    until(&mut v, &now, "the step played", |_| ears.plays().len() == 1);
    let played = ears.plays()[0].clone();
    assert!(played.ends_with("audio.m4a"), "{played}");
    assert!(played.contains(&lasting.display().to_string()), "{played}");
    assert!(fake.spoken().is_empty(), "no local voice");
    assert_eq!(asked.load(Ordering::SeqCst), 1);

    // Another speaker on the same machine: complete, nothing fetched.
    let again = Arc::new(AtomicUsize::new(0));
    let mut v = speaker(&dir, &fake, &Ears::default());
    with_bundles(&mut v, fetcher(Vec::new(), Some("never"), &again));
    until(&mut v, &now, "the step open", |v| v.current.is_some());
    assert_eq!(again.load(Ordering::SeqCst), 0);

    // Offline on another machine: silent, and said plainly.
    let dir = scratch("bundle-offline-lesson");
    let ears = Ears::default();
    let mut v = speaker(&dir, &fake, &ears);
    let failing = Arc::new(AtomicUsize::new(0));
    let lasting = dir.join("tutorial-voices");
    if let Some(p) = v.places.as_mut() {
        p.recorded = vec![lasting.clone()];
    }
    v.bundles = Some(Downloads::new(
        manifest.clone(),
        lasting,
        fetcher(Vec::new(), Some("keine Verbindung"), &failing),
    ));
    until(&mut v, &now, "the failure said", |v| {
        matches!(v.status, Status::NotLoaded(_))
    });
    let info = v.info();
    assert!(
        info.contains("konnte nicht geladen werden") && info.contains("keine Verbindung"),
        "{info}"
    );
    assert!(ears.plays().is_empty());
    assert!(fake.spoken().is_empty(), "no other voice");
}

/// Without a voice or recordings: not available, the switch off.
#[test]
fn without_a_voice_it_is_off() {
    let dir = scratch("no-voice");
    let mut v = Speaker::new(&CONFIG, Box::new(Toy));
    v.places = Some(fake_machine(&dir, false, false));
    assert!(!v.available(Lang::En) && !v.on(Lang::En));
    // A Speaker without the app's narration says nothing.
    let mut silent = Speaker::new(&CONFIG, Box::new(super::Silent));
    silent.places = Some(fake_machine(&dir, true, false));
    until(&mut silent, &at(0, Lang::En), "open", |v| {
        v.status == Status::Quiet
    });
    assert!(silent.current.is_none());
}

/// Two seconds at 8 kHz: a loud second (a tone at half scale), then a
/// silent one.
fn loud_then_silent() -> Pcm {
    let rate = 8000;
    let samples = (0..2 * rate)
        .map(|i| {
            if i < rate {
                0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin()
            } else {
                0.0
            }
        })
        .collect();
    Pcm {
        channels: 1,
        rate,
        samples,
    }
}

struct Tape {
    clock: Clock,
    playing: bool,
    t0: Instant,
    gone: Duration,
    pcm: Arc<Pcm>,
}

/// The tests' player of [`loud_then_silent`]: a [`Clock`] as the
/// speakers', on a time of the test's.
#[derive(Clone)]
struct Deck(Arc<Mutex<Tape>>);

impl Deck {
    fn new() -> Deck {
        Deck(Arc::new(Mutex::new(Tape {
            clock: Clock::default(),
            playing: false,
            t0: Instant::now(),
            gone: Duration::ZERO,
            pcm: Arc::new(loud_then_silent()),
        })))
    }

    fn tape<R>(&self, f: impl FnOnce(&mut Tape) -> R) -> R {
        f(&mut self.0.lock().expect("the tape"))
    }

    fn advance(&self, d: Duration) {
        self.tape(|t| t.gone += d);
    }
}

const TOTAL: Duration = Duration::from_secs(2);

impl Play for Deck {
    fn play(&mut self, _: &Path) {
        self.tape(|t| {
            let now = t.t0 + t.gone;
            t.clock = Clock::default();
            t.clock.start(TOTAL, now);
            t.playing = true;
        });
    }
    fn stop(&mut self) {
        self.tape(|t| t.playing = false);
    }
    fn playing(&self) -> bool {
        self.tape(|t| t.playing)
    }
    fn error(&self) -> Option<String> {
        None
    }
    fn position(&self) -> Option<(Duration, Duration)> {
        self.tape(|t| t.playing.then(|| (t.clock.at(t.t0 + t.gone), TOTAL)))
    }
    fn seek(&mut self, to: Duration) {
        self.tape(|t| t.clock.seek(to, t.t0 + t.gone));
    }
    fn pause(&mut self, paused: bool) {
        self.tape(|t| t.clock.pause(paused, t.t0 + t.gone));
    }
    fn paused(&self) -> bool {
        self.tape(|t| t.clock.paused())
    }
    fn pcm(&self) -> Option<Arc<Pcm>> {
        Some(self.tape(|t| Arc::clone(&t.pcm)))
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Whether the bars are at rest.
fn rest(bars: [f32; BARS]) -> bool {
    bars.iter().zip(REST).all(|(b, r)| (b - r).abs() < 1e-6)
}

/// The speakers' clock: from where it starts, paused and on again, a jump,
/// never past the end.
#[test]
fn the_clock_moves_pauses_and_jumps() {
    let t0 = Instant::now();
    let mut c = Clock::default();
    assert_eq!(c.at(t0), Duration::ZERO);
    assert_eq!(c.total(), None);
    c.start(ms(3000), t0);
    assert_eq!(c.at(t0 + ms(1200)), ms(1200));
    c.pause(true, t0 + ms(1500));
    assert!(c.paused());
    assert_eq!(c.at(t0 + ms(2500)), ms(1500), "still while paused");
    c.pause(false, t0 + ms(2500));
    assert_eq!(c.at(t0 + ms(2600)), ms(1600));
    c.seek(ms(500), t0 + ms(2600));
    assert_eq!(c.at(t0 + ms(2700)), ms(600));
    assert_eq!(c.at(t0 + ms(9000)), ms(3000), "not past the end");
    // Paused before the samples were handed over: it starts paused.
    let mut c = Clock::default();
    c.pause(true, t0);
    c.seek(ms(800), t0);
    c.start(ms(3000), t0);
    assert_eq!(c.at(t0 + ms(1000)), ms(800));
    assert_eq!(meter::clock_text(ms(71_900)), "1:11");
}

/// The bars as loud as the samples about the time played: high where the
/// tone is, at rest in the silence; up at once, down more slowly.
#[test]
fn the_bars_follow_the_samples() {
    let pcm = loud_then_silent();
    let loud = meter::levels(&pcm, ms(500));
    assert!(loud.iter().all(|&b| b > 0.8), "{loud:?}");
    let silent = meter::levels(&pcm, ms(1500));
    assert!(rest(silent), "{silent:?}");
    // At the edge, half of the window in each.
    let edge = meter::levels(&pcm, ms(1000));
    assert!(edge[0] > 0.8 && edge[BARS - 1] == 0.0, "{edge:?}");
    // Past the end: nothing, no panic.
    assert!(rest(meter::levels(&pcm, ms(5000))));

    let mut m = Meter::default();
    let frame = 1.0 / 30.0;
    m.step(Some(REST), 0.0);
    let up = m.step(Some([1.0; BARS]), frame)[0];
    assert!(up > 0.6, "up fast: {up}");
    for i in 2..30 {
        m.step(Some([1.0; BARS]), f64::from(i) * frame);
    }
    let top = m.shown[0];
    let down = m.step(Some(REST), 30.0 * frame)[0];
    assert!(
        top - down < 0.3 && down < top,
        "down slowly: {top} → {down}"
    );
    for i in 31..60 {
        m.step(Some(REST), f64::from(i) * frame);
    }
    assert!(m.shown[0] < 0.05, "{}", m.shown[0]);
    assert!(rest(m.step(None, 3.0)), "at rest at once");
}

/// The delay egui was asked to draw again after, `None` for never.
fn repaint(ctx: &egui::Context, v: &mut Speaker) -> Option<Duration> {
    // A new context asks for the first few frames itself: the last one's.
    let mut d = None;
    for _ in 0..5 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| v.frame(None, ui.ctx()));
        out.textures_delta.clear();
        d = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|o| o.repaint_delay)
            .filter(|d| *d != Duration::MAX);
    }
    d
}

/// A speaker with the deck as its player, the deck playing.
fn playing(deck: &Deck) -> Speaker {
    let mut v = Speaker::new(&CONFIG, Box::new(Toy));
    let mut d = deck.clone();
    d.play(Path::new("section.wav"));
    v.player = Some(Box::new(d));
    v
}

/// Quiet, nothing is drawn again; playing, often enough for the bars;
/// paused, nothing again. Where it is and how long it is, as the player
/// says; the bars as loud as the samples there.
#[test]
fn the_window_is_drawn_again_only_while_it_plays() {
    let ctx = egui::Context::default();
    let mut quiet = Speaker::new(&CONFIG, Box::new(Toy));
    assert_eq!(repaint(&ctx, &mut quiet), None);
    assert_eq!(quiet.progress(), None);
    assert!(rest(quiet.bars(0.0, false)));

    let deck = Deck::new();
    let mut v = playing(&deck);
    assert_eq!(repaint(&ctx, &mut v), Some(MOVING));
    assert_eq!(v.progress(), Some((Duration::ZERO, TOTAL)));
    deck.advance(ms(500));
    assert_eq!(v.progress(), Some((ms(500), TOTAL)));
    let bars = v.bars(1.0, false);
    assert!(bars.iter().all(|&b| b > 0.8), "loud: {bars:?}");
    v.seek(ms(1500));
    assert_eq!(v.progress(), Some((ms(1500), TOTAL)));
    for i in 1..30 {
        v.bars(1.0 + f64::from(i) / 30.0, false);
    }
    let bars = v.bars(2.0, false);
    assert!(bars.iter().all(|&b| b < 0.05), "silent: {bars:?}");

    v.seek(ms(200));
    v.pause(true);
    assert!(v.paused());
    deck.advance(ms(700));
    assert_eq!(v.progress(), Some((ms(200), TOTAL)), "still while paused");
    assert!(rest(v.bars(3.0, false)), "the bars at rest while paused");
    assert_eq!(repaint(&ctx, &mut v), None, "paused: not drawn again");
    v.pause(false);
    deck.advance(ms(100));
    assert_eq!(v.progress(), Some((ms(300), TOTAL)));
    v.stop();
    assert_eq!(v.progress(), None);
    assert_eq!(repaint(&ctx, &mut v), None);
}

/// Without animations: the bars still, the window drawn again only for
/// the time and the progress bar.
#[test]
fn without_animations_the_bars_stand_still() {
    let ctx = egui::Context::default();
    ctx.global_style_mut(|s| s.animation_time = 0.0);
    let deck = Deck::new();
    let mut v = playing(&deck);
    deck.advance(ms(500));
    assert_eq!(repaint(&ctx, &mut v), Some(TICKING));
    assert!(rest(v.bars(1.0, true)));
    assert_eq!(v.progress(), Some((ms(500), TOTAL)));
}

/// The voice's row on a toy pane (no GPU): without a voice the info and
/// the switch, greyed; with one, the switch, Replay and the level while a
/// step is there, the bar and the time while it plays; the switch
/// clicked off is remembered.
#[cfg(feature = "ui")]
#[test]
fn the_row_shows_the_voice() {
    use super::ui::{Spots, failure, progress, row};
    opencad_ui::i18n::set_thread_lang(Lang::En);
    let dir = scratch("row");
    let settings = dir.join("config");
    let ctx = egui::Context::default();
    opencad_ui::install(&ctx, |_| {});
    let mut time = 0.0;
    let mut pane = |v: &mut Speaker, events: Vec<egui::Event>| -> Spots {
        time += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 400.0),
            )),
            events,
            time: Some(time),
            ..Default::default()
        };
        let mut spots = Spots::new();
        let mut out = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| row(ui, v, &settings, &mut spots));
            progress(ui, v, 300.0, &mut spots);
            failure(ui, v, &mut spots);
        });
        out.textures_delta.clear();
        spots
    };
    let names = |s: &Spots| s.iter().map(|(n, _)| n.clone()).collect::<BTreeSet<_>>();

    let mut none = Speaker::new(&CONFIG, Box::new(Toy));
    none.places = Some(fake_machine(&scratch("row-none"), false, false));
    let s = pane(&mut none, Vec::new());
    assert_eq!(
        names(&s),
        ["Read aloud", "Voice info"].map(String::from).into()
    );

    let (fake, deck) = (Fake::default(), Deck::new());
    let mut v = speaker(&dir, &fake, &Ears::default());
    let d = deck.clone();
    v.make_player = Arc::new(move || Box::new(d.clone()));
    until(&mut v, &at(0, Lang::En), "played", |v| v.playing());
    let s = pane(&mut v, Vec::new());
    for spot in [
        "Read aloud",
        "Voice info",
        "Stop voice",
        "Pause voice",
        "Voice level",
        "Voice progress",
        "Voice time",
    ] {
        assert!(names(&s).contains(spot), "no {spot}: {:?}", names(&s));
    }
    let switch = (s.iter().find(|(n, _)| n == "Read aloud")).map(|(_, r)| r.center());
    let at_ = switch.expect("the switch");
    let button = |pressed| egui::Event::PointerButton {
        pos: at_,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    pane(&mut v, vec![egui::Event::PointerMoved(at_)]);
    pane(&mut v, vec![button(true)]);
    pane(&mut v, vec![button(false)]);
    assert_eq!(v.settings.on, Some(false));
    assert!(!v.playing());
    let back = Settings::load(&settings, CONFIG.voice.recorded);
    assert_eq!(back.on, Some(false));
}

/// The app's own lesson "mine", read by a voice it reaches itself: every
/// step a remote section, whatever is on the machine.
struct Mine {
    remote: Fake,
    /// The remote voice gives AAC in an MP4, not a WAV.
    aac: bool,
}

impl Narrate for Mine {
    fn recorded(
        &mut self,
        course: &str,
        _: Lang,
        _: Option<(&str, ElevenRun<'_>)>,
    ) -> Option<Vec<Option<Section>>> {
        (course == "mine").then(|| vec![None; STEPS.len()])
    }

    fn local(
        &mut self,
        course: &str,
        lang: Lang,
        step: usize,
        _: &Found,
        _: Prefer,
    ) -> Option<Section> {
        let text = STEPS
            .get(step)
            .filter(|t| course == "mine" && !t.is_empty())?;
        Some(Section {
            engine: Engine::Remote,
            key: format!("remote-{}", key::sha256(text.as_bytes())[..12].to_owned()),
            text: key::untagged(text),
            lang,
            voice: "de-male".into(),
            speech: Speech::Remote {
                backend: "toy-server".into(),
                model: "chatterbox".into(),
                label: "Chatterbox, de-male".into(),
            },
        })
    }

    fn speaks(&mut self, course: &str, _: Lang) -> bool {
        course == "mine"
    }

    fn remote(&mut self) -> Option<(Box<dyn Synth>, Kill)> {
        let (synth, kill) = self.remote.make();
        if !self.aac {
            return Some((synth, kill));
        }
        // As the fake, but AAC in its file.
        struct Aac(Box<dyn Synth>);
        impl Synth for Aac {
            fn speak(&mut self, s: &Section, out: &Path) -> Result<(), String> {
                self.0.speak(s, out)?;
                std::fs::write(out, TONE).map_err(|e| e.to_string())
            }
            fn close(&mut self) {}
        }
        Some((Box::new(Aac(synth)), kill))
    }
}

/// A lesson of the app's own voice: available and on with no voice on the
/// machine, each step spoken by the app's synth (never the local worker),
/// kept in the lasting cache with its backend and model, the info naming
/// the voice; AAC from the synth is kept as `audio.m4a` and decodes.
#[test]
fn a_remote_voice_reads_the_apps_own_lessons() {
    opencad_ui::i18n::set_thread_lang(Lang::De);
    for aac in [false, true] {
        let dir = scratch(&format!("remote-{aac}"));
        let (local, remote, ears) = (Fake::default(), Fake::default(), Ears::default());
        let mut v = Speaker::new(
            &CONFIG,
            Box::new(Mine {
                remote: remote.clone(),
                aac,
            }),
        );
        v.places = Some(fake_machine(&dir, false, false));
        v.cache = Some(dir.join("cache"));
        v.lasting = Some(dir.join("lasting"));
        let f = local.clone();
        v.make_synth = Arc::new(move |_: &Found, _: &Path| f.make());
        let e = ears.clone();
        v.make_player = Arc::new(move || Box::new(e.clone()));
        v.load(&dir.join("config"));
        assert!(!v.available(Lang::De), "no course open, no voice here");
        let now = Some(At {
            course: "mine".into(),
            step: 0,
            lang: Lang::De,
        });
        until(&mut v, &now, "the step played", |_| ears.plays().len() == 1);
        assert!(v.available(Lang::De) && v.on(Lang::De));
        let s = v.current.clone().expect("a section");
        assert!(local.spoken().is_empty(), "{:?}", local.spoken());
        assert_eq!(remote.spoken().first(), Some(&s.key));
        assert_eq!(v.info(), "KI-Stimme (Chatterbox, de-male)");
        let name = if aac { "audio.m4a" } else { "audio.wav" };
        let kept = dir.join("lasting").join(&s.key);
        assert_eq!(ears.plays()[0], kept.join(name).display().to_string());
        assert!(!dir.join("cache").join(&s.key).exists(), "not twice");
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(kept.join("meta.json")).expect("meta"))
                .expect("JSON");
        assert_eq!(meta["backend"], "toy-server");
        assert_eq!(meta["model"], "chatterbox");
        assert_eq!(meta["voice"], "de-male");
        #[cfg(feature = "playback")]
        assert!(player::read_audio(&kept.join(name)).is_ok());

        // Played again from the lasting cache: nothing asked twice.
        v.replay();
        until(&mut v, &now, "played again", |_| ears.plays().len() == 2);
        until(&mut v, &now, "spoken ahead", |_| remote.spoken().len() == 2);
        assert_eq!(remote.spoken()[0], s.key);
    }
}

/// A remote section without the app's synth says so; a local section is
/// never the remote synth's.
#[test]
fn the_worker_sends_each_section_to_its_synth() {
    let (local, remote) = (Fake::default(), Fake::default());
    let dir = scratch("dispatch");
    let mut mine = Mine {
        remote: remote.clone(),
        aac: false,
    };
    let section = mine
        .local("mine", Lang::En, 0, &Found::default(), Prefer::Fish)
        .expect("a section");
    let (mut both, _) = super::synth::Dispatch::make(local.make(), mine.remote());
    both.speak(&section, &dir.join("a.wav")).expect("spoken");
    both.speak(&a_section("here"), &dir.join("b.wav"))
        .expect("spoken");
    assert_eq!(remote.spoken(), std::slice::from_ref(&section.key));
    assert_eq!(local.spoken(), ["here"]);
    let (mut alone, _) = super::synth::Dispatch::make(local.make(), None);
    let e = alone
        .speak(&section, &dir.join("c.wav"))
        .expect_err("no remote synth");
    assert!(e.contains("Chatterbox, de-male"), "{e}");
    let (mut process, _) = super::synth::Process::make("Toy", Found::default(), dir.join("worker"));
    assert!(process.speak(&section, &dir.join("d.wav")).is_err());
}

/// A WAV played on this machine's speakers: run with `EGUI_LESSONS_WAV`
/// (`cargo test -p egui-lessons speakers_play -- --ignored`).
#[cfg(feature = "playback")]
#[test]
#[ignore = "needs an audio device"]
fn speakers_play_a_wav() {
    let wav = PathBuf::from(std::env::var("EGUI_LESSONS_WAV").expect("a WAV"));
    let mut s = player::Speakers::new("Toy");
    s.play(&wav);
    let started = Instant::now();
    while s.playing() && started.elapsed().as_secs() < 30 {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(s.error(), None);
    assert!(
        started.elapsed().as_millis() > 1000,
        "it played for a while"
    );
}
