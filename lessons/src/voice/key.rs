//! The keys the recorder's caches keep a spoken section by:
//! `fishspeech.Voice.key`, `elevenlabs.Voice.key` and `narrate.key` of
//! egui-screencast's `scripts/`, to the letter, so that what a recording
//! spoke is played as it is. The text given is as said (after the app's
//! `spoken()` and its pronunciation list). `tests/fixtures/keys.json`,
//! written by `scripts/lesson_fixture.py` with the recorder's own code, is
//! the test.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::Lang;

/// S2 Pro as mlx-speech runs it (`fishspeech.py`).
pub const FISH_MODEL: &str = "appautomaton/fishaudio-s2-pro-8bit-mlx";
const FISH_BACKEND: &str = "fish-s2-pro";
const FISH_ENGINE: &str = "mlx-speech";
const FISH_FORMAT: u32 = 1;
const FISH_OUTPUT: &str = "wav_24000";
/// The seed of every section (`fishspeech.SETTINGS`).
pub const FISH_SEED: u32 = 1;
/// Kokoro's weights at the runner's revision (`narrate.py`).
pub const KOKORO_REPO: &str = "hexgrad/Kokoro-82M";
pub const KOKORO_REVISION: &str = "f3ff3571791e39611d31c381e3a41a3af07b4987";
/// Kokoro's voice when the script names none of its own.
pub const KOKORO_VOICE: &str = "am_michael";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"\[[^\[\]]*\]"));

/// Python's `str.isspace`: Unicode's white space and the four separators
/// `\x1c` to `\x1f`.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `" ".join(text.split())`.
pub fn single_spaced(text: &str) -> String {
    text.split(py_space)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text without its delivery tags (`align.untagged`): what is shown,
/// and what Kokoro gets.
pub fn untagged(text: &str) -> String {
    single_spaced(&TAG.replace_all(text, " "))
}

/// SHA-256 in hex.
pub fn sha256(bytes: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, bytes);
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// A string as Python's `json.dumps(…, ensure_ascii=False)` writes it.
pub fn py_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A voice of S2 Pro as the key needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct FishKey {
    /// SHA-256 of the reference clip's bytes.
    pub reference: String,
    pub transcript: String,
    /// What S2 Pro is told: none for English, else the language's name.
    pub language: Option<String>,
    pub seed: u32,
}

impl FishKey {
    /// The section's key (`fishspeech.Voice.key`): the sorted JSON of what
    /// makes it, hashed, its first 24 hex digits. `text` is as spoken.
    pub fn key(&self, text: &str) -> String {
        let language = self.language.as_deref().map_or("null".into(), py_json);
        let material = format!(
            "{{\"backend\": {}, \"engine\": {}, \"format\": {FISH_FORMAT}, \"language\": {language}, \
             \"model\": {}, \"output_format\": {}, \"reference\": {}, \"settings\": {{\"seed\": {}}}, \
             \"text\": {}, \"transcript\": {}}}",
            py_json(FISH_BACKEND),
            py_json(FISH_ENGINE),
            py_json(FISH_MODEL),
            py_json(FISH_OUTPUT),
            py_json(&self.reference),
            self.seed,
            py_json(&single_spaced(text)),
            py_json(&single_spaced(&self.transcript)),
        );
        sha256(material.as_bytes())[..24].to_owned()
    }
}

/// What S2 Pro is told of a voice's language (`Voice.generation_language`).
pub fn generation_language(language: Option<&str>) -> Option<String> {
    language
        .filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("english"))
        .map(str::to_owned)
}

/// Python's `repr` of a float, as `str(speed)` writes it: `1.0`, `1.1`.
fn py_float(x: f64) -> String {
    format!("{x:?}")
}

/// A Kokoro section's key (`narrate.key`): `text` untagged.
pub fn kokoro_key(text: &str, voice: &str, speed: f64) -> String {
    let material = format!("{KOKORO_REVISION}\n{voice}\n{}\n{text}", py_float(speed));
    sha256(material.as_bytes())[..16].to_owned()
}

/// ElevenLabs as the recorder asks it (`elevenlabs.py`): its model when
/// none is given.
pub const ELEVEN_MODEL: &str = "eleven_multilingual_v2";
const ELEVEN_BACKEND: &str = "elevenlabs";
const ELEVEN_FORMAT: u32 = 1;
const ELEVEN_OUTPUT: &str = "pcm_24000";

/// A JSON value as Python's `json.dumps(…, sort_keys=True,
/// ensure_ascii=False)` writes it: `1.0` for a float, `1` for an integer.
fn py_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) if n.is_f64() => n.as_f64().map_or_else(|| n.to_string(), py_float),
        Value::Number(n) => n.to_string(),
        Value::String(s) => py_json(s),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().map(py_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(o) => {
            let sorted: BTreeMap<&String, &Value> = o.iter().collect();
            let items: Vec<String> = sorted
                .into_iter()
                .map(|(k, v)| format!("{}: {}", py_json(k), py_value(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// A voice of ElevenLabs as the key needs it (`elevenlabs.Voice`).
#[derive(Debug, Clone, PartialEq)]
pub struct ElevenKey {
    pub voice_id: String,
    pub model_id: String,
    /// The voice settings sent: `elevenlabs.SETTINGS` with the script's
    /// speed, then the script's `voice_settings`.
    pub settings: serde_json::Map<String, Value>,
    /// The language the model was told (`language_code`, "de"); none: not
    /// told, and not in the key.
    pub language_code: Option<String>,
}

/// The `language_code` the recorder sends for a choice
/// (`tutorial.elevenlabs_language`): `choice` (`--voice-language`), else
/// the script's `voice_language`; `auto` is `lang`, `off` or `none` none,
/// "German" (Fish's name of it) `de`, else a code of two or three letters.
pub fn eleven_language(choice: Option<&str>, script: Option<&str>, lang: Lang) -> Option<String> {
    let choice = choice
        .filter(|c| !c.is_empty())
        .or(script.filter(|c| !c.is_empty()))?
        .trim()
        .to_lowercase();
    match choice.as_str() {
        "" | "off" | "none" => None,
        "auto" => Some(lang.key().to_owned()),
        "german" => Some("de".to_owned()),
        c if (2..=3).contains(&c.chars().count()) && c.chars().all(|x| x.is_ascii_alphabetic()) => {
            Some(c.to_owned())
        }
        _ => None,
    }
}

impl ElevenKey {
    /// The voice `voice_id` as the recorder makes it for a script of
    /// `speed` (`tutorial.speaker_of`): ElevenLabs' default model and
    /// settings.
    pub fn new(voice_id: &str, speed: f64) -> ElevenKey {
        let mut settings = serde_json::Map::new();
        settings.insert("stability".into(), Value::from(0.5));
        settings.insert("similarity_boost".into(), Value::from(0.75));
        settings.insert("style".into(), Value::from(0.0));
        settings.insert("use_speaker_boost".into(), Value::Bool(true));
        settings.insert("speed".into(), Value::from(speed));
        ElevenKey {
            voice_id: voice_id.to_owned(),
            model_id: ELEVEN_MODEL.to_owned(),
            settings,
            language_code: None,
        }
    }

    /// The section's key (`elevenlabs.Voice.key`): the sorted JSON of what
    /// makes it, hashed, its first 24 hex digits. `text` is as spoken; its
    /// delivery tags go (`elevenlabs.normalized`). The language is in it
    /// only when one was told, so that the sections recorded without one
    /// keep their keys.
    pub fn key(&self, text: &str) -> String {
        let language = (self.language_code.as_deref())
            .filter(|l| !l.is_empty())
            .map(|l| format!("\"language_code\": {}, ", py_json(l)))
            .unwrap_or_default();
        let material = format!(
            "{{\"backend\": {}, \"format\": {ELEVEN_FORMAT}, {language}\"model_id\": {}, \
             \"output_format\": {}, \"settings\": {}, \"text\": {}, \"voice_id\": {}}}",
            py_json(ELEVEN_BACKEND),
            py_json(&self.model_id),
            py_json(ELEVEN_OUTPUT),
            py_value(&Value::Object(self.settings.clone())),
            py_json(&untagged(text)),
            py_json(&self.voice_id),
        );
        sha256(material.as_bytes())[..24].to_owned()
    }
}
