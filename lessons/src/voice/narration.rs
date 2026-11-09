//! What each step of a script says, as the recorder says it, and its
//! sections for each voice (OpenSolve's ADR 0056): the step's
//! `say_lesson`, `say` or text in the language, through the pronunciation
//! list ([`Lists`]), keyed as the recording keyed it. [`Scripts`] is the
//! [`Narrate`] of an app's embedded courses.

use std::collections::HashMap;

use serde_json::Value;

use super::discover::{FishVoice, Found};
use super::{ElevenRun, Engine, Narrate, Prefer, Section, Speech, key};
use crate::Lang;
use crate::pronounce::Lists;
use crate::script::{Embedded, localized, only_in, text};
use crate::spoken::spoken;

/// A course's narration in a language: each step's text to say, and
/// Kokoro's voice and speed of the script.
#[derive(Debug, Clone, PartialEq)]
pub struct Narration {
    pub steps: Vec<String>,
    pub voice: String,
    pub speed: f64,
    /// Each step's text as ElevenLabs says it, its key's: `say_lesson` in
    /// the language (made by `scripts/lesson_voices.py`, never by a
    /// recording), else as the recorder sent it to its voice (`say`, else
    /// `text`, of the script in the language); none where there is
    /// nothing to say.
    pub recorded: Vec<Option<String>>,
    /// The voice settings, model and language a recording's ElevenLabs
    /// had by the script: its speed (`speed_de` in German),
    /// `voice_settings`, `voice_model`, `voice_language`.
    pub eleven_speed: f64,
    pub eleven_settings: serde_json::Map<String, Value>,
    pub eleven_model: Option<String>,
    pub eleven_language: Option<String>,
    /// The language of the narration (`--lang`).
    pub lang: Lang,
}

impl Narration {
    /// The key of `voice_id` saying a step's recorded text, as the
    /// recorder made it with `run`: its model and language over the
    /// script's.
    pub fn eleven_key(&self, voice_id: &str, run: ElevenRun<'_>) -> key::ElevenKey {
        let mut k = key::ElevenKey::new(voice_id, self.eleven_speed);
        for (name, v) in &self.eleven_settings {
            k.settings.insert(name.clone(), v.clone());
        }
        if let Some(m) = run.model.or(self.eleven_model.as_deref()) {
            m.clone_into(&mut k.model_id);
        }
        k.language_code =
            key::eleven_language(run.language, self.eleven_language.as_deref(), self.lang);
        k
    }
}

/// A step's narration in a lesson in `lang` only: `say_lesson` (what a
/// lesson says where the video says otherwise: "Thanks for following
/// along" for "Thanks for watching"), else `say`. The recorder never
/// reads `say_lesson`.
fn say_in(step: &Value, lang: Lang) -> Option<String> {
    only_in(step, "say_lesson", lang).or_else(|| only_in(step, "say", lang))
}

/// The narration of a script: each step's `say_lesson` or `say` in
/// `lang`, else its text. `None`: it is no YAML or has no steps.
pub fn narration(script: &str, lang: Lang) -> Option<Narration> {
    let v: Value = serde_yaml::from_str(script).ok()?;
    let steps = v["steps"]
        .as_array()?
        .iter()
        .map(|s| say_in(s, lang).unwrap_or_else(|| text(s, "text", lang)))
        .collect();
    let voice = v["voice"]
        .as_str()
        .filter(|n| {
            let b = n.as_bytes();
            n.len() > 3 && b[0].is_ascii_lowercase() && matches!(b[1], b'f' | b'm') && b[2] == b'_'
        })
        .unwrap_or(key::KOKORO_VOICE)
        .to_owned();
    let speed = v["speed"].as_f64().unwrap_or(1.0);
    // As the recorder reads it: the script in the language.
    let local = localized(&v, lang);
    let recorded = v["steps"]
        .as_array()?
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if let Some(lesson) = only_in(s, "say_lesson", lang) {
                return Some(lesson);
            }
            let step = &local["steps"][i];
            let text = (step["say"].as_str().filter(|t| !t.is_empty()))
                .or_else(|| step["text"].as_str())?;
            Some(text.to_owned()).filter(|t| !t.trim().is_empty())
        })
        .collect();
    Some(Narration {
        steps,
        voice,
        speed,
        recorded,
        eleven_speed: local["speed"].as_f64().unwrap_or(1.0),
        eleven_settings: local["voice_settings"]
            .as_object()
            .cloned()
            .unwrap_or_default(),
        eleven_model: local["voice_model"].as_str().map(str::to_owned),
        eleven_language: local["voice_language"].as_str().map(str::to_owned),
        lang,
    })
}

/// A step's section as ElevenLabs `voice_id` says it, keyed as the
/// recorder and `scripts/lesson_voices.py` key it: `recorded` is the
/// step's text before its pronunciation list ([`Narration::recorded`]);
/// none when there is nothing to say.
pub fn eleven_section(
    lists: &Lists,
    voice_id: &str,
    run: ElevenRun<'_>,
    lang: Lang,
    recorded: &str,
    n: &Narration,
) -> Option<Section> {
    let said = lists.said(recorded, lang, Engine::ElevenLabs);
    let text = key::untagged(&said);
    if text.is_empty() {
        return None;
    }
    Some(Section {
        engine: Engine::ElevenLabs,
        key: n.eleven_key(voice_id, run).key(&said),
        text,
        lang,
        voice: voice_id.to_owned(),
        speech: Speech::Recorded,
    })
}

/// A step's section as the English recordings' Kokoro said it (the
/// recorder's `--fast`), keyed as the recorder and `scripts/lesson_voices.py
/// --lang en` key it: the script's Kokoro voice and speed, the text through
/// the pronunciation list, untagged. Only played, from the recordings or a
/// downloaded bundle; none when there is nothing to say.
pub fn kokoro_recorded(
    lists: &Lists,
    lang: Lang,
    recorded: &str,
    n: &Narration,
) -> Option<Section> {
    let text = key::untagged(&lists.said(recorded, lang, Engine::Kokoro));
    if text.is_empty() {
        return None;
    }
    Some(Section {
        engine: Engine::Kokoro,
        key: key::kokoro_key(&text, &n.voice, n.speed),
        text,
        lang,
        voice: n.voice.clone(),
        speech: Speech::Recorded,
    })
}

/// A step's section as the voice found says it in `lang` (S2 Pro, or
/// Kokoro in English; `prefer` where both do), its pronunciation list
/// applied for that voice; none when there is nothing to say or no voice
/// for it.
pub fn section(
    lists: &Lists,
    found: &Found,
    prefer: Prefer,
    lang: Lang,
    text: &str,
    n: &Narration,
) -> Option<Section> {
    if key::untagged(&spoken(text, lang)).is_empty() {
        return None;
    }
    let fish = found
        .fish
        .as_ref()
        .and_then(|f| f.voices.iter().find(|v| v.lang == lang));
    let kokoro = found.kokoro.as_ref().filter(|_| lang == Lang::En);
    let fish_section = |v: &FishVoice| {
        let said = lists.said(text, lang, Engine::Fish);
        let k = key::FishKey {
            reference: v.hash.clone(),
            transcript: v.transcript.clone(),
            language: v.language.clone(),
            seed: key::FISH_SEED,
        };
        Section {
            engine: Engine::Fish,
            key: k.key(&said),
            text: key::single_spaced(&said),
            lang,
            voice: v.name.clone(),
            speech: Speech::Fish {
                reference: v.reference.clone(),
                transcript: v.transcript.clone(),
                language: v.language.clone(),
                seed: key::FISH_SEED,
            },
        }
    };
    let kokoro_section = || {
        let text = key::untagged(&lists.said(text, lang, Engine::Kokoro));
        Section {
            engine: Engine::Kokoro,
            key: key::kokoro_key(&text, &n.voice, n.speed),
            text,
            lang,
            voice: n.voice.clone(),
            speech: Speech::Kokoro {
                voice: n.voice.clone(),
                speed: n.speed,
            },
        }
    };
    match (prefer, fish, kokoro) {
        (Prefer::Kokoro, _, Some(_)) | (Prefer::Fish, None, Some(_)) => Some(kokoro_section()),
        (_, Some(v), _) => Some(fish_section(v)),
        _ => None,
    }
}

/// Each step of `n` as the recordings' voice said it: ElevenLabs `eleven`
/// where the language has one, else in English the recordings' Kokoro.
pub fn recorded(
    lists: &Lists,
    n: &Narration,
    eleven: Option<(&str, ElevenRun<'_>)>,
) -> Vec<Option<Section>> {
    let lang = n.lang;
    (n.recorded.iter())
        .map(|r| {
            let r = r.as_deref()?;
            match eleven {
                Some((v, run)) => eleven_section(lists, v, run, lang, r, n),
                None if lang == Lang::En => kokoro_recorded(lists, lang, r, n),
                None => None,
            }
        })
        .collect()
}

/// The narration of an app's embedded courses: each course's read once
/// per language, through the app's pronunciation lists. An app with
/// lessons of its own besides (an assistant's) answers for those and asks
/// this for the rest.
#[derive(Debug)]
pub struct Scripts {
    courses: &'static [Embedded],
    lists: &'static Lists,
    narrations: HashMap<(String, Lang), Option<Narration>>,
}

impl Scripts {
    pub fn new(courses: &'static [Embedded], lists: &'static Lists) -> Scripts {
        Scripts {
            courses,
            lists,
            narrations: HashMap::new(),
        }
    }

    /// Whether `course` is one of them.
    pub fn has(&self, course: &str) -> bool {
        self.courses.iter().any(|e| e.name == course)
    }

    /// The pronunciation lists.
    pub fn lists(&self) -> &'static Lists {
        self.lists
    }

    /// `n` as the narration of `course` in its language, embedded or not:
    /// a course of the app's own, or a test's.
    pub fn set(&mut self, course: &str, n: Narration) {
        self.narrations.insert((course.to_owned(), n.lang), Some(n));
    }

    /// The narration of `course` in `lang`, read once.
    pub fn narration(&mut self, course: &str, lang: Lang) -> Option<Narration> {
        let courses = self.courses;
        self.narrations
            .entry((course.to_owned(), lang))
            .or_insert_with(|| {
                let e = courses.iter().find(|e| e.name == course)?;
                narration(e.script, lang)
            })
            .clone()
    }
}

impl Narrate for Scripts {
    fn recorded(
        &mut self,
        course: &str,
        lang: Lang,
        eleven: Option<(&str, ElevenRun<'_>)>,
    ) -> Option<Vec<Option<Section>>> {
        let n = self.narration(course, lang)?;
        Some(recorded(self.lists, &n, eleven))
    }

    fn local(
        &mut self,
        course: &str,
        lang: Lang,
        step: usize,
        found: &Found,
        prefer: Prefer,
    ) -> Option<Section> {
        let n = self.narration(course, lang)?;
        let text = n.steps.get(step)?;
        section(self.lists, found, prefer, lang, text, &n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::discover::{Fish, Kokoro};

    const SCRIPT: &str = r"
title: T
voice: bf_emma
speed: 1.1
speed_de: 1.2
voice_model: eleven_turbo_v2_5
steps:
  - name: a
    text: The **text**.
    say: Said in English.
    say_de: Auf Deutsch gesagt, z. B. 10 mm.
  - name: b
    text: Only text, 10 mm.
    text_de: Nur Text, 10 mm.
  - name: c
    text: x
    say: {en: Mapped, de: Zugeordnet}
    say_lesson: {en: Thanks for following along, de: Danke fürs Mitmachen}
  - name: d
    text: ''
";

    static COURSES: &[Embedded] = &[Embedded {
        name: "t",
        script: SCRIPT,
        files: &[],
    }];
    static LISTS: Lists = Lists::new(&[("de", "all: [{from: z. B., to: zum Beispiel}]")]);

    #[test]
    fn the_narration_is_say_lesson_say_else_the_text() {
        let en = narration(SCRIPT, Lang::En).expect("a narration");
        assert_eq!(
            en.steps,
            [
                "Said in English.",
                "Only text, 10 mm.",
                "Thanks for following along",
                ""
            ]
        );
        assert_eq!((en.voice.as_str(), en.speed), ("bf_emma", 1.1));
        let de = narration(SCRIPT, Lang::De).expect("a narration");
        assert_eq!(de.steps[1], "Nur Text, 10 mm.");
        assert_eq!(de.recorded[2].as_deref(), Some("Danke fürs Mitmachen"));
        assert_eq!(de.recorded[3], None);
        assert!((de.eleven_speed - 1.2).abs() < 1e-9);
        let k = de.eleven_key("v", ElevenRun::default());
        assert_eq!(k.model_id, "eleven_turbo_v2_5");
        let k = de.eleven_key(
            "v",
            ElevenRun {
                model: Some("eleven_v3"),
                language: Some("auto"),
            },
        );
        assert_eq!(
            (k.model_id.as_str(), k.language_code.as_deref()),
            ("eleven_v3", Some("de"))
        );
    }

    #[test]
    fn the_scripts_narrate_their_courses() {
        let mut s = Scripts::new(COURSES, &LISTS);
        assert!(s.has("t") && !s.has("u"));
        assert!(s.recorded("u", Lang::En, None).is_none());
        // German: only with the recordings' ElevenLabs voice.
        let none = s.recorded("t", Lang::De, None).expect("the course");
        assert!(none.iter().all(Option::is_none));
        let de = s
            .recorded("t", Lang::De, Some(("v", ElevenRun::default())))
            .expect("the course");
        let first = de[0].clone().expect("said");
        assert_eq!(
            first.text,
            "Auf Deutsch gesagt, zum Beispiel 10 Millimeter."
        );
        assert_eq!(first.speech, Speech::Recorded);
        assert!(de[3].is_none());
        // English: the recordings' Kokoro.
        let en = s.recorded("t", Lang::En, None).expect("the course");
        let first = en[0].clone().expect("said");
        assert_eq!(first.engine, Engine::Kokoro);
        assert_eq!(
            first.key,
            key::kokoro_key("Said in English.", "bf_emma", 1.1)
        );

        let found = Found {
            fish: Some(Fish {
                python: "p".into(),
                voices: vec![FishVoice {
                    lang: Lang::De,
                    name: "de-male".into(),
                    reference: "r.wav".into(),
                    transcript: "Hallo.".into(),
                    language: Some("German".into()),
                    hash: "abc".into(),
                }],
            }),
            kokoro: Some(Kokoro {
                python: "k".into(),
                hf_home: "h".into(),
            }),
            ..Found::default()
        };
        let de = s
            .local("t", Lang::De, 0, &found, Prefer::Kokoro)
            .expect("S2 Pro in German");
        assert_eq!(de.engine, Engine::Fish);
        assert_eq!(de.text, "Auf Deutsch gesagt, zum Beispiel 10 Millimeter.");
        let en = s
            .local("t", Lang::En, 1, &found, Prefer::Fish)
            .expect("Kokoro: no English S2 Pro voice");
        assert_eq!(en.engine, Engine::Kokoro);
        assert_eq!(en.text, "Only text, 10 millimeters.");
        assert!(s.local("t", Lang::En, 3, &found, Prefer::Fish).is_none());
    }
}
