//! A lesson read aloud (OpenSolve's ADR 0056): each step's narration
//! played when the step opens, by the voice its recordings spoke where the
//! course has them (ElevenLabs, or in English Kokoro, from the recordings'
//! caches or a downloaded bundle), else by Fish Audio S2 Pro (mlx-speech)
//! or Kokoro where they are on the machine, the next steps spoken ahead in
//! the background; nothing installed, and no text leaves the machine. The
//! app never asks ElevenLabs itself. A course's recorded sections are
//! downloaded once as a bundle where they are not on the machine
//! ([`bundle`]).
//!
//! What a step says is the app's ([`Narrate`]): it keys each step's text
//! as the recorder keyed it ([`key`]); the [`Speaker`] finds, speaks, plays
//! and shows it.

pub mod bundle;
pub mod cache;
pub mod discover;
pub mod key;
pub mod meter;
pub mod narration;
pub mod player;
pub mod queue;
pub mod synth;
#[cfg(test)]
mod tests;
#[cfg(feature = "ui")]
pub mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use opencad_ui::i18n::{tr, trf};
use serde_json::{Value, json};

use crate::{Config, Lang, Recorded};
use discover::{Found, Places};
use player::Play;
use queue::{Kill, Queue, Synth};

/// How many steps after the one shown are spoken ahead.
const AHEAD: usize = 3;

/// How often the window is drawn while a section plays: the speaker's
/// bars move (about 30 times a second); without animations only the time
/// and the progress bar (4 times a second).
pub const MOVING: Duration = Duration::from_millis(33);
pub const TICKING: Duration = Duration::from_millis(250);

/// Whether the window animates: egui's animations not off (the app's
/// reduced motion, as the lesson's dim has it).
pub fn animates(ctx: &egui::Context) -> bool {
    ctx.global_style().animation_time > 0.0
}

/// Who speaks a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Fish,
    Kokoro,
    /// ElevenLabs, only as the recordings had it speak: played from their
    /// cache, never asked for.
    ElevenLabs,
    /// A voice the app reaches itself (a server's model): its sections are
    /// [`Speech::Remote`], spoken by the app's [`Narrate::remote`] synth,
    /// never by the local worker.
    Remote,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Fish => "Fish Audio S2 Pro",
            Engine::Kokoro => "Kokoro",
            Engine::ElevenLabs => "ElevenLabs",
            Engine::Remote => "remote",
        }
    }

    /// As the recordings' `meta.json` names it (a remote section's own
    /// backend is its [`Speech::Remote`]'s).
    pub fn backend(self) -> &'static str {
        match self {
            Engine::Fish => "fish-s2-pro",
            Engine::Kokoro => "kokoro",
            Engine::ElevenLabs => "elevenlabs",
            Engine::Remote => "remote",
        }
    }
}

/// What the worker needs besides the text.
#[derive(Debug, Clone, PartialEq)]
pub enum Speech {
    /// Spoken by the app's remote synth ([`Narrate::remote`]): `backend` and
    /// `model` as the cache's `meta.json` names them (`ai-connector`,
    /// `chatterbox-v3`), `label` as the voice's info says it ("Chatterbox,
    /// de-male").
    Remote {
        backend: String,
        model: String,
        label: String,
    },
    Fish {
        reference: PathBuf,
        transcript: String,
        language: Option<String>,
        seed: u32,
    },
    Kokoro {
        voice: String,
        speed: f64,
    },
    /// What a recording spoke: only played, never spoken here.
    Recorded,
}

/// A step's narration as one voice says it, and its key.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub engine: Engine,
    pub key: String,
    /// As sent to the voice: tags kept for S2 Pro, gone for Kokoro.
    pub text: String,
    pub lang: Lang,
    /// The voice's name.
    pub voice: String,
    pub speech: Speech,
}

/// Which voice, where both speak the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Prefer {
    #[default]
    Fish,
    Kokoro,
}

/// The settings, in `voice.json` beside `lessons.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// The app's recordings' ElevenLabs voices
    /// ([`crate::VoiceConfig::recorded`]): what a language has when the
    /// settings say nothing. Not saved.
    pub defaults: &'static [Recorded],
    /// Read aloud; `None`: never set, on when a voice is found.
    pub on: Option<bool>,
    /// The voices folder of S2 Pro; `None`: `~/dev/tts-voices`.
    pub voices: Option<PathBuf>,
    pub prefer: Prefer,
    /// The ElevenLabs voice id of the recordings in German and English;
    /// `None`: never set, the app's ([`Settings::defaults`]); empty: none.
    pub eleven_de: Option<String>,
    pub eleven_en: Option<String>,
    /// The model the recordings had ElevenLabs use in German and English
    /// (`--voice-model`); `None`: the app's, else the script's or
    /// ElevenLabs' default; empty: the script's or the default.
    pub eleven_model_de: Option<String>,
    pub eleven_model_en: Option<String>,
    /// The language they told it (`--voice-language`: `de`, `auto`,
    /// `off`); `None`: the app's, else none; empty: the script's
    /// `voice_language`, else none.
    pub eleven_language_de: Option<String>,
    pub eleven_language_en: Option<String>,
}

/// A setting by language: as set (trimmed, empty is none), else `default`.
fn set_or<'a>(set: &'a Option<String>, default: Option<&'a str>) -> Option<&'a str> {
    set.as_deref()
        .map_or(default, |s| Some(s.trim()))
        .filter(|s| !s.is_empty())
}

impl Settings {
    /// Nothing set, the app's recordings' voices as the defaults.
    pub fn new(defaults: &'static [Recorded]) -> Settings {
        Settings {
            defaults,
            ..Settings::default()
        }
    }

    fn file(dir: &Path) -> PathBuf {
        dir.join("voice.json")
    }

    /// The app's recordings' voice in `lang`.
    fn default_of(&self, lang: Lang) -> Option<&'static Recorded> {
        self.defaults.iter().find(|r| r.lang == lang)
    }

    /// The ElevenLabs voice whose recorded sections are played in `lang`.
    pub fn eleven(&self, lang: Lang) -> Option<&str> {
        let set = match lang {
            Lang::De => &self.eleven_de,
            _ => &self.eleven_en,
        };
        set_or(set, self.default_of(lang).map(|r| r.voice))
    }

    /// The model the recordings' ElevenLabs used in `lang`, over the
    /// script's (`--voice-model`).
    pub fn eleven_model(&self, lang: Lang) -> Option<&str> {
        let set = match lang {
            Lang::De => &self.eleven_model_de,
            _ => &self.eleven_model_en,
        };
        set_or(set, self.default_of(lang).map(|r| r.model))
    }

    /// The language the recordings told ElevenLabs in `lang`, as given
    /// (`--voice-language`: a code, `auto`, `off`), over the script's.
    pub fn eleven_language(&self, lang: Lang) -> Option<&str> {
        let set = match lang {
            Lang::De => &self.eleven_language_de,
            _ => &self.eleven_language_en,
        };
        set_or(set, self.default_of(lang).map(|r| r.language))
    }

    /// The settings of `dir`, the app's recordings' voices as the defaults.
    pub fn load(dir: &Path, defaults: &'static [Recorded]) -> Settings {
        let Ok(text) = std::fs::read_to_string(Settings::file(dir)) else {
            return Settings::new(defaults);
        };
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        let eleven = |l: &str| v["elevenlabs"][l].as_str().map(str::to_owned);
        let model = |l: &str| v["elevenlabs_model"][l].as_str().map(str::to_owned);
        let language = |l: &str| v["elevenlabs_language"][l].as_str().map(str::to_owned);
        Settings {
            defaults,
            eleven_de: eleven("de"),
            eleven_en: eleven("en"),
            eleven_model_de: model("de"),
            eleven_model_en: model("en"),
            eleven_language_de: language("de"),
            eleven_language_en: language("en"),
            on: v["on"].as_bool(),
            voices: v["voices"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            prefer: if v["prefer"].as_str() == Some("kokoro") {
                Prefer::Kokoro
            } else {
                Prefer::Fish
            },
        }
    }

    /// Into `dir`'s `voice.json`; `app` names the log line of a failure.
    pub fn save(&self, dir: &Path, app: &str) {
        let v = json!({
            "on": self.on,
            "voices": self.voices.as_ref().map(|p| p.display().to_string()),
            "prefer": match self.prefer { Prefer::Fish => "fish", Prefer::Kokoro => "kokoro" },
            "elevenlabs": { "de": self.eleven_de, "en": self.eleven_en },
            "elevenlabs_model": { "de": self.eleven_model_de, "en": self.eleven_model_en },
            "elevenlabs_language": { "de": self.eleven_language_de, "en": self.eleven_language_en },
        });
        let path = Settings::file(dir);
        let written = std::fs::create_dir_all(dir).and_then(|()| {
            std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default())
        });
        if let Err(e) = written {
            eprintln!("{}: {}: {e}", app.to_lowercase(), path.display());
        }
    }
}

/// A setting of the recordings' ElevenLabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevenField {
    Voice,
    Model,
    Language,
}

/// What the recorder was told besides the script (`--voice-model`,
/// `--voice-language`): the settings' of the language.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ElevenRun<'a> {
    pub model: Option<&'a str>,
    pub language: Option<&'a str>,
}

impl<'a> ElevenRun<'a> {
    pub fn of(settings: &'a Settings, lang: Lang) -> ElevenRun<'a> {
        ElevenRun {
            model: settings.eleven_model(lang),
            language: settings.eleven_language(lang),
        }
    }
}

/// What each step of a course says, keyed as the recorder keyed it: the
/// app's. [`narration::Scripts`] is that of the app's embedded courses; an
/// app with lessons of its own (an assistant's, read by a voice it reaches
/// itself) wraps it.
pub trait Narrate: Send {
    /// Each step of `course` in `lang` as the recordings' voice said it:
    /// ElevenLabs `eleven` (its voice id, and the model and language the
    /// recorder was told) where the language has one, else in English the
    /// recordings' Kokoro; `None` for a step that says nothing, or every
    /// step of a language without a recordings' voice. `None`: no such
    /// course. One entry per step.
    fn recorded(
        &mut self,
        course: &str,
        lang: Lang,
        eleven: Option<(&str, ElevenRun<'_>)>,
    ) -> Option<Vec<Option<Section>>>;

    /// Step `step` of `course` in `lang` as a voice `found` here says it
    /// (S2 Pro, or Kokoro in English; `prefer` where both do); `None` when
    /// it says nothing or no voice speaks the language.
    fn local(
        &mut self,
        course: &str,
        lang: Lang,
        step: usize,
        found: &Found,
        prefer: Prefer,
    ) -> Option<Section>;

    /// Whether a voice of the app's own speaks `course` in `lang`, whatever
    /// is on the machine or in the bundles (the remote voice of its own
    /// lessons): the voice is then available and on.
    fn speaks(&mut self, course: &str, lang: Lang) -> bool {
        let _ = (course, lang);
        false
    }

    /// The synth of the app's [`Speech::Remote`] sections, and what stops
    /// it while it speaks; asked each time the worker starts. `None`: such
    /// a section fails ("not on this computer").
    fn remote(&mut self) -> Option<(Box<dyn Synth>, Kill)> {
        None
    }
}

/// A course says nothing: what a [`Speaker`] without the app's narration
/// reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct Silent;

impl Narrate for Silent {
    fn recorded(
        &mut self,
        _: &str,
        _: Lang,
        _: Option<(&str, ElevenRun<'_>)>,
    ) -> Option<Vec<Option<Section>>> {
        None
    }

    fn local(&mut self, _: &str, _: Lang, _: usize, _: &Found, _: Prefer) -> Option<Section> {
        None
    }
}

/// Makes the synth of what was found, its process killed by `Kill`.
pub type MakeSynth = Arc<dyn Fn(&Found, &Path) -> (Box<dyn Synth>, Kill) + Send + Sync>;
/// Makes the player.
pub type MakePlayer = Arc<dyn Fn() -> Box<dyn Play> + Send + Sync>;

/// The step spoken for: course, step, language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct At {
    pub course: String,
    pub step: usize,
    pub lang: Lang,
}

/// What the voice does.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Nothing, or the step said.
    Quiet,
    /// The step's audio is being spoken; it plays when ready.
    Preparing,
    /// The course has its sections in ElevenLabs, but not this step's:
    /// it stays silent rather than be read by another voice.
    Unrecorded,
    Failed(String),
    /// The course's voice is being downloaded: so many bytes of so many.
    Loading {
        done: u64,
        total: u64,
    },
    /// The course's voice could not be downloaded; tried again the next
    /// time a lesson opens. The step stays silent.
    NotLoaded(String),
}

/// The voice of the lessons.
pub struct Speaker {
    pub settings: Settings,
    loaded: Option<PathBuf>,
    found: Option<Found>,
    /// Where voices are looked for; `None`: this machine's (none in tests).
    pub places: Option<Places>,
    /// The app's cache of sections.
    pub cache: Option<PathBuf>,
    /// The lasting cache, where the remote sections are kept (beside the
    /// recordings', which are never written); `None`: the app's cache
    /// (an app's tests, unless they say where).
    pub lasting: Option<PathBuf>,
    /// Makes the local worker; the remote sections go to
    /// [`Narrate::remote`]'s synth.
    pub make_synth: MakeSynth,
    pub make_player: MakePlayer,
    queue: Option<Queue>,
    player: Option<Box<dyn Play>>,
    at: Option<At>,
    /// What the steps say: the app's.
    narrate: Box<dyn Narrate>,
    /// The app's names and places.
    config: &'static Config,
    /// The step's section.
    pub current: Option<Section>,
    pub status: Status,
    /// The step's section plays when it is ready.
    autoplay: bool,
    /// The speaker's bars.
    meter: meter::Meter,
    /// The courses' voices downloaded as bundles; `None`: never (the
    /// tests, unless they say how).
    pub bundles: Option<bundle::Downloads>,
    /// The bundle of the course open being downloaded.
    waiting: Option<String>,
}

impl std::fmt::Debug for Speaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speaker")
            .field("settings", &self.settings)
            .field("found", &self.found)
            .field("at", &self.at)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Speaker {
    /// The voice of an app with `config`, its steps' words by `narrate`.
    /// Looking on the machine ([`crate::VoiceConfig::machine`]), it
    /// downloads the courses' bundles of the config's manifest.
    pub fn new(config: &'static Config, narrate: Box<dyn Narrate>) -> Speaker {
        let app = config.app;
        Speaker {
            settings: Settings::new(config.voice.recorded),
            loaded: None,
            found: None,
            places: None,
            cache: cache::app_dir(app),
            lasting: if config.voice.here() {
                cache::lasting_dir(app)
            } else {
                None
            },
            make_synth: Arc::new(move |found, root| {
                synth::Process::make(app, found.clone(), root.to_path_buf())
            }),
            make_player: Arc::new(move || Box::new(player::Speakers::new(app))),
            queue: None,
            player: None,
            at: None,
            narrate,
            config,
            current: None,
            status: Status::Quiet,
            autoplay: false,
            meter: meter::Meter::default(),
            bundles: if config.voice.here() {
                bundle::Downloads::here(config)
            } else {
                None
            },
            waiting: None,
        }
    }

    /// The app's names and places.
    pub fn config(&self) -> &'static Config {
        self.config
    }

    /// The settings of `dir`, read once.
    pub fn load(&mut self, dir: &Path) {
        if self.loaded.as_deref() != Some(dir) {
            self.settings = Settings::load(dir, self.config.voice.recorded);
            self.loaded = Some(dir.to_path_buf());
            self.found = None;
        }
    }

    fn save(&self) {
        if let Some(dir) = &self.loaded {
            self.settings.save(dir, self.config.app);
        }
    }

    /// What speaks here, looked for once (again when the folder changes).
    pub fn found(&mut self) -> &Found {
        if self.found.is_none() {
            let places = match &self.places {
                Some(p) => Some(p.clone()),
                // An app's tests never speak unless they say where.
                None if !self.config.voice.here() => None,
                None => Some(Places::here(self.config, self.settings.voices.clone())),
            };
            self.found = Some(places.map_or_else(Found::default, |p| discover::discover(&p)));
        }
        self.found.get_or_insert_with(Found::default)
    }

    /// The voices folder set: looked for again.
    pub fn set_voices(&mut self, voices: Option<PathBuf>) {
        if self.settings.voices != voices {
            self.settings.voices = voices.clone();
            if let Some(p) = &mut self.places {
                p.voices = voices.or_else(|| discover::default_voices(p.home.as_deref()));
            }
            self.found = None;
            self.queue = None;
            self.save();
            self.restart();
        }
    }

    pub fn set_prefer(&mut self, prefer: Prefer) {
        if self.settings.prefer != prefer {
            self.settings.prefer = prefer;
            self.queue = None;
            self.save();
            self.restart();
        }
    }

    /// Whether a voice speaks `lang`: the app's own for the course open
    /// ([`Narrate::speaks`]), a local one, or the recordings' (ElevenLabs,
    /// in English else Kokoro) where a recordings' cache is on the machine
    /// or a bundle of the language can be downloaded.
    pub fn available(&mut self, lang: Lang) -> bool {
        if let Some(at) = &self.at
            && self.narrate.speaks(&at.course, lang)
        {
            return true;
        }
        let eleven = self.settings.eleven(lang).is_some();
        let found = self.found();
        let recorded = found.recorded.iter().any(|d| d.is_dir());
        let speaks = found.speaks(lang);
        // English is read by the recordings' Kokoro where no ElevenLabs
        // voice is set; a bundle of the language can be downloaded.
        let bundled = self.bundles.as_ref().is_some_and(|b| b.manifest.any(lang));
        speaks || ((eleven || lang == Lang::En) && (recorded || bundled))
    }

    /// The recordings' ElevenLabs voice, model or language in `lang` set
    /// (`""`: none, or the script's).
    pub fn set_eleven(&mut self, lang: Lang, field: ElevenField, value: &str) {
        let value = Some(value.trim().to_owned());
        let s = &mut self.settings;
        let slot = match (field, lang) {
            (ElevenField::Voice, Lang::De) => &mut s.eleven_de,
            (ElevenField::Voice, _) => &mut s.eleven_en,
            (ElevenField::Model, Lang::De) => &mut s.eleven_model_de,
            (ElevenField::Model, _) => &mut s.eleven_model_en,
            (ElevenField::Language, Lang::De) => &mut s.eleven_language_de,
            (ElevenField::Language, _) => &mut s.eleven_language_en,
        };
        if *slot != value {
            *slot = value;
            self.save();
            self.restart();
        }
    }

    /// Read aloud: as set, else whether a voice is found.
    pub fn on(&mut self, lang: Lang) -> bool {
        let available = self.available(lang);
        available && self.settings.on.unwrap_or(true)
    }

    fn player(&mut self) -> &mut Box<dyn Play> {
        let make = Arc::clone(&self.make_player);
        self.player.get_or_insert_with(|| make())
    }

    /// Whether the step's audio plays.
    pub fn playing(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.playing())
    }

    /// Whether the step's audio is paused (it still counts as playing).
    pub fn paused(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|p| p.playing() && p.paused())
    }

    /// Paused, or on again.
    pub fn pause(&mut self, paused: bool) {
        if let Some(p) = &mut self.player {
            p.pause(paused);
        }
    }

    /// From `to` on.
    pub fn seek(&mut self, to: Duration) {
        if let Some(p) = &mut self.player {
            p.seek(to);
        }
    }

    /// Where the step's audio is and how long it is, while it plays.
    pub fn progress(&self) -> Option<(Duration, Duration)> {
        let p = self.player.as_ref().filter(|p| p.playing())?;
        p.position()
    }

    /// The speaker's bars at `now` (egui's time): the loudness of what is
    /// being played, smoothed; at rest when paused, quiet, or `still`
    /// (without animations).
    pub fn bars(&mut self, now: f64, still: bool) -> [f32; meter::BARS] {
        let target = (!still && !self.paused())
            .then(|| {
                let (at, _) = self.progress()?;
                let pcm = self.player.as_ref()?.pcm()?;
                Some(meter::levels(&pcm, at))
            })
            .flatten();
        self.meter.step(target, now)
    }

    /// Why the audio did not play.
    pub fn play_error(&self) -> Option<String> {
        self.player.as_ref().and_then(|p| p.error())
    }

    /// The sections of the step at `at` and the next few; and whether the
    /// step stays silent for want of its recording ([`Status::Unrecorded`]).
    /// Public for an app's tests of its narration.
    pub fn sections(&mut self, at: &At) -> (Vec<Section>, bool) {
        let prefer = self.settings.prefer;
        let settings = self.settings.clone();
        let eleven = (settings.eleven(at.lang)).map(|v| (v, ElevenRun::of(&settings, at.lang)));
        let Some(recorded) = self.narrate.recorded(&at.course, at.lang, eleven) else {
            return (Vec::new(), false);
        };
        let found = self.found().clone();
        // Each step as the recordings' voice of the language said it (the
        // ElevenLabs voice set, else in English Kokoro), and whether the
        // caches have it.
        let wanted: Vec<Option<(Section, bool)>> = (recorded.into_iter())
            .map(|s| {
                let s = s?;
                let here = cache::find(&s, &found, None).is_some();
                Some((s, here))
            })
            .collect();
        // A course with sections of the recordings' voice, or with a
        // bundle of them to download, is read by that voice alone: a step
        // not recorded stays silent, never another voice's (two speakers
        // in one lesson). One without any (a lesson the assistant made) is
        // the local voice's.
        let bundled = wanted.iter().any(Option::is_some)
            && (self.bundles.as_ref())
                .is_some_and(|b| b.manifest.of(&at.course, at.lang).is_some());
        let recorded_course = bundled || wanted.iter().flatten().any(|(_, here)| *here);
        let mut out: Vec<Section> = Vec::new();
        for i in (0..wanted.len()).skip(at.step).take(AHEAD + 1) {
            let chosen = match wanted.get(i).cloned().flatten() {
                Some((s, true)) => Some(s),
                Some(_) if recorded_course && i == at.step => return (Vec::new(), true),
                _ if recorded_course => None,
                _ => (self.narrate).local(&at.course, at.lang, i, &found, prefer),
            };
            match chosen {
                Some(s) => out.push(s),
                // The step itself says nothing: no section of its own.
                None if i == at.step => return (Vec::new(), false),
                None => {}
            }
        }
        (out, false)
    }

    /// The course's bundle at `at` asked for, unless the caches have all
    /// its sections; how its download goes, while it does or if it failed.
    fn download(&mut self, at: &At) -> Option<Status> {
        let b = self
            .bundles
            .as_ref()?
            .manifest
            .of(&at.course, at.lang)?
            .clone();
        let found = self.found().clone();
        let here = |k: &String| {
            cache::in_folders(k, &found.recorded).is_some()
                || (found.tutorials.as_deref())
                    .is_some_and(|t| t.join("audio").join(format!("{k}.wav")).is_file())
        };
        let downloads = self.bundles.as_ref()?;
        if downloads.state(&b.file).is_none() && b.keys.iter().all(here) {
            return None;
        }
        downloads.want(&b);
        let state = downloads.state(&b.file)?;
        self.waiting = Some(b.file);
        match state {
            bundle::Loading::Running { done, total } => Some(Status::Loading { done, total }),
            bundle::Loading::Failed(e) => Some(Status::NotLoaded(e)),
            bundle::Loading::Done => None,
        }
    }

    /// Where a section was spoken before.
    fn cached(&mut self, s: &Section) -> Option<PathBuf> {
        let cache = self.cache.clone();
        let lasting = self.lasting.clone();
        cache::find(s, self.found(), cache.as_deref()).or_else(|| {
            let remote = matches!(s.speech, Speech::Remote { .. });
            cache::in_folders(&s.key, &[lasting.filter(|_| remote)?])
        })
    }

    /// The step's section and the next ones: played if `play` and ready,
    /// the others asked of the worker.
    fn begin(&mut self, play: bool) {
        let Some(at) = self.at.clone() else {
            return;
        };
        let (sections, unrecorded) = self.sections(&at);
        self.current = sections.first().cloned();
        let loading = self.download(&at);
        self.status = match loading {
            Some(s) if unrecorded => s,
            _ if unrecorded => Status::Unrecorded,
            _ => Status::Quiet,
        };
        self.autoplay = false;
        let Some(current) = self.current.clone() else {
            return;
        };
        if let Some(wav) = self.cached(&current) {
            if play {
                self.player().play(&wav);
            }
        } else {
            self.status = Status::Preparing;
            self.autoplay = play;
        }
        // A recorded section is never spoken here.
        let missing: Vec<Section> = sections
            .into_iter()
            .filter(|s| s.speech != Speech::Recorded && self.cached(s).is_none())
            .collect();
        if missing.is_empty() {
            return;
        }
        let Some(root) = self.cache.clone() else {
            self.status = Status::Failed("no cache folder".into());
            return;
        };
        // The recordings' caches are only read: the app's own sections go
        // elsewhere, so that nothing of theirs is ever replaced.
        if self.found().recorded.contains(&root) {
            self.status = Status::Failed(format!("{}: the recordings' cache", root.display()));
            return;
        }
        if self.queue.is_none() {
            let found = self.found().clone();
            let local = (self.make_synth)(&found, &root);
            let (synth, kill) = synth::Dispatch::make(local, self.narrate.remote());
            let lasting = self.lasting.clone();
            self.queue = Some(Queue::keeping(self.config.app, root, lasting, synth, kill));
        }
        if let Some(q) = &self.queue {
            q.want(missing);
        }
    }

    /// The settings changed: the step said again, should it be read aloud.
    fn restart(&mut self) {
        self.stop();
        let lang = self.at.as_ref().map(|a| a.lang);
        if let Some(lang) = lang
            && self.on(lang)
        {
            self.begin(true);
        }
    }

    /// The step's audio stopped; it will not play when ready.
    pub fn stop(&mut self) {
        self.autoplay = false;
        if let Some(p) = &mut self.player {
            p.stop();
        }
    }

    /// The step said again from its start.
    pub fn replay(&mut self) {
        self.stop();
        self.begin(true);
    }

    /// Read aloud switched.
    pub fn switch(&mut self, on: bool) {
        self.settings.on = Some(on);
        self.save();
        if on {
            self.begin(true);
        } else {
            self.stop();
            // Its memory back: the worker ends, the audio output closes.
            self.queue = None;
            self.player = None;
            self.status = Status::Quiet;
        }
    }

    /// Once a frame: the step shown `now`; what the worker spoke.
    pub fn frame(&mut self, now: Option<At>, ctx: &egui::Context) {
        for done in self.queue.as_ref().map(Queue::poll).unwrap_or_default() {
            let current = self.current.as_ref().is_some_and(|c| c.key == done.key);
            match done.result {
                Ok(wav) if current => {
                    self.status = Status::Quiet;
                    if std::mem::take(&mut self.autoplay) {
                        self.player().play(&wav);
                    }
                }
                Err(e) if current => self.status = Status::Failed(e),
                _ => {}
            }
        }
        self.downloaded();
        if now != self.at {
            self.stop();
            self.at.clone_from(&now);
            self.current = None;
            self.status = Status::Quiet;
            match &now {
                // The course closed: the worker ends, the audio output
                // closes.
                None => {
                    self.queue = None;
                    self.player = None;
                    // A download that failed is tried again next time.
                    self.waiting = None;
                    if let Some(d) = &self.bundles {
                        d.forget_failed();
                    }
                }
                Some(at) => {
                    if self.on(at.lang) {
                        self.begin(true);
                    }
                }
            }
        }
        if self.playing() && !self.paused() {
            ctx.request_repaint_after(if animates(ctx) { MOVING } else { TICKING });
        } else if matches!(self.status, Status::Preparing | Status::Loading { .. }) {
            // The spinner; paused or quiet, nothing is drawn again.
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    /// How the course's download goes: its progress shown; once it is
    /// unpacked, the step silent for want of it read aloud.
    fn downloaded(&mut self) {
        let Some(file) = self.waiting.clone() else {
            return;
        };
        let state = self.bundles.as_ref().and_then(|d| d.state(&file));
        let loading = matches!(self.status, Status::Loading { .. });
        match state {
            Some(bundle::Loading::Running { done, total }) => {
                if loading {
                    self.status = Status::Loading { done, total };
                }
            }
            Some(bundle::Loading::Done) => {
                self.waiting = None;
                if loading {
                    self.begin(true);
                }
            }
            Some(bundle::Loading::Failed(e)) => {
                self.waiting = None;
                if loading {
                    self.status = Status::NotLoaded(e);
                }
            }
            None => self.waiting = None,
        }
    }

    /// Whether a worker runs (or would): for tests.
    pub fn queue(&self) -> Option<&Queue> {
        self.queue.as_ref()
    }
}

impl Speaker {
    /// What the voice info (ⓘ) says: why it does not work, that the step
    /// has no recording yet, else which AI voice reads it.
    pub fn info(&self) -> String {
        match &self.status {
            Status::Failed(e) => return trf(Msg::Failed, &[e]),
            Status::Unrecorded => return tr(Msg::Unrecorded).to_owned(),
            Status::Loading { done, total } => {
                let pct = (done * 100).checked_div(*total).unwrap_or(0).min(100);
                return trf(Msg::Loading, &[&pct.to_string()]);
            }
            Status::NotLoaded(e) => return trf(Msg::NotLoaded, &[e]),
            _ => {}
        }
        if let Some(e) = self.play_error() {
            return trf(Msg::Failed, &[&e]);
        }
        if let Some(Section {
            speech: Speech::Remote { label, .. },
            ..
        }) = &self.current
        {
            return trf(Msg::AiRemote, &[label]);
        }
        let recorded = self
            .current
            .as_ref()
            .is_some_and(|s| s.speech == Speech::Recorded);
        tr(match self.current.as_ref().map(|s| s.engine) {
            Some(Engine::Kokoro) if recorded => Msg::AiKokoroRecorded,
            Some(Engine::Kokoro) => Msg::AiKokoro,
            Some(Engine::ElevenLabs) => Msg::AiEleven,
            _ => Msg::AiFish,
        })
        .to_owned()
    }
}

opencad_ui::messages! {
    pub(crate) enum Msg {
        Failed: "Reading aloud does not work: {0}", "Vorlesen geht gerade nicht: {0}";
        AiFish: "AI voice (Fish Audio S2 Pro, non-commercial)", "KI-Stimme (Fish Audio S2 Pro, nicht kommerziell)";
        AiKokoro: "AI voice (Kokoro)", "KI-Stimme (Kokoro)";
        AiRemote: "AI voice ({0})", "KI-Stimme ({0})";
        AiKokoroRecorded: "AI voice (Kokoro, from the recording)", "KI-Stimme (Kokoro, aus der Aufnahme)";
        Loading: "Downloading the voice for this lesson … {0} %", "Die Stimme für diese Lektion wird geladen … {0} %";
        NotLoaded: "The voice for this lesson could not be downloaded ({0}). It will try again the next time you open the lesson.", "Die Stimme für diese Lektion konnte nicht geladen werden ({0}). Beim nächsten Öffnen der Lektion wird es nochmal versucht.";
        AiEleven: "AI voice (ElevenLabs, from the recording)", "KI-Stimme (ElevenLabs, aus der Aufnahme)";
        Unrecorded: "There is no recording for this step yet.", "Für diesen Schritt gibt es noch keine Aufnahme.";
    }
}
