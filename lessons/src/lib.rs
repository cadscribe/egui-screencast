//! Courses in an egui app (OpenSolve's ADR 0062): the lessons of
//! [egui-screencast](https://github.com/cadscribe/egui-screencast)'s
//! tutorial scripts, played in the app itself.
//!
//! It holds the script format ([`script`]: a course and its steps in a
//! language, cues checked against what they say, the app's own keys of a
//! step as its [`script::Extra`]; `FORMAT.md` says every key), the text as
//! the voice says it ([`spoken`], [`pronounce`]) and the narration: a
//! lesson's step read aloud by the voice its recordings spoke
//! ([`voice::Speaker`]), what each step says ([`voice::narration`]), the
//! keys the recorder keeps a spoken section by ([`voice::key`]), the
//! caches and voices found on the machine, a worker that speaks ahead with
//! a local voice (or the app's own remote one), the course's sections
//! downloaded once as a bundle, the player with its progress bar and the
//! speaker's level, and the voice's row in the lesson pane (feature `ui`).
//! The hints and the pane follow in later releases.
//!
//! The app gives its names and places once, as a [`Config`]:
//!
//! ```
//! use egui_lessons::{Config, VoiceConfig};
//!
//! static CONFIG: Config = Config {
//!     app: "MyApp",
//!     voice: VoiceConfig {
//!         work_env: Some("MYAPP_TUTORIALS_WORK"),
//!         ..VoiceConfig::NONE
//!     },
//! };
//! assert_eq!(CONFIG.lower(), "myapp");
//! ```

pub mod pronounce;
pub mod script;
pub mod spoken;
pub mod voice;

pub use script::{Course, Embedded, Extra, Step};

/// The language the lessons speak and show: opencad-ui's, so that the app
/// and the lessons share one language state (`lang()`, `set_lang`).
pub use opencad_ui::i18n::{Lang, lang};

/// What the app gives the lessons once: its name and the voice's places
/// and data.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// The app's name ("OpenSolve"): the folders of the voice (the lasting
    /// cache `~/Library/Application Support/OpenSolve/tutorial-voices`, the
    /// app's own `<cache dir>/opensolve/voices`), its threads and its log
    /// lines (`opensolve: audio: …`).
    pub app: &'static str,
    pub voice: VoiceConfig,
}

impl Config {
    /// The app's name in lower case: its folders where the system's
    /// folders are in lower case, its threads, its log lines.
    pub fn lower(&self) -> String {
        self.app.to_lowercase()
    }
}

/// The voice's places beyond the user's, and what the app knows of its
/// recordings.
#[derive(Debug, Clone, Copy)]
pub struct VoiceConfig {
    /// The environment variable naming the tutorials' work folder (the
    /// recorder's `--work`: its venv, Kokoro's sections, `voices/`), as
    /// `OPENSOLVE_TUTORIALS_WORK`.
    pub work_env: Option<&'static str>,
    /// The work folder when the variable is not set, where it exists: the
    /// repository's `target/tutorials` of a development build.
    pub work_dir: Option<&'static str>,
    /// The environment variable that, set, turns the voice off: nothing is
    /// looked for or downloaded (`OPENSOLVE_VOICE_OFF`).
    pub off_env: Option<&'static str>,
    /// The environment variable whose URL replaces the bundles' base
    /// (`OPENSOLVE_VOICE_BUNDLES`).
    pub bundles_env: Option<&'static str>,
    /// The bundles' manifest, embedded by the app (`bundles.json`, written
    /// by its build of the bundles); empty: none.
    pub bundles: &'static str,
    /// The ElevenLabs voice the recordings of a language used, when the
    /// settings say none.
    pub recorded: &'static [Recorded],
    /// Whether the voice looks on this machine. `false` in an app's tests:
    /// no voice is found and nothing is downloaded unless a test says where.
    pub machine: bool,
}

impl VoiceConfig {
    /// No places, no bundles, no recordings' voice; looking on the machine.
    pub const NONE: VoiceConfig = VoiceConfig {
        work_env: None,
        work_dir: None,
        off_env: None,
        bundles_env: None,
        bundles: "",
        recorded: &[],
        machine: true,
    };

    /// Whether to look on this machine now: [`VoiceConfig::machine`] and
    /// the off variable not set.
    pub fn here(&self) -> bool {
        self.machine && self.off_env.is_none_or(|k| std::env::var_os(k).is_none())
    }
}

/// The ElevenLabs voice the recordings of `lang` used, its model and the
/// language it was told (`--voice elevenlabs:<id> --voice-model …
/// --voice-language …`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded {
    pub lang: Lang,
    pub voice: &'static str,
    pub model: &'static str,
    pub language: &'static str,
}
