//! What a voice is given and the keys of it against the recorder's own:
//! `tests/fixtures/spoken.json` (by `scripts/spoken_fixture.py`) of the
//! test tutorials in `tests/tutorials` and a few extra texts, and
//! `tests/fixtures/lesson_voices.json` (by `scripts/lesson_voices_fixture.py`)
//! of the lessons' own sections, both made by the recorder's Python. A
//! text or key that differs is a section a lesson does not find.

use egui_lessons::Lang;
use egui_lessons::pronounce::{self, List, Lists};
use egui_lessons::spoken::spoken;
use egui_lessons::voice::key::{self, ElevenKey, FishKey};
use egui_lessons::voice::narration::{self, narration};
use egui_lessons::voice::{ElevenRun, Engine};
use serde_json::Value;

/// The test tutorials' lists, as an app embeds its own.
static LISTS: Lists = Lists::new(&[
    ("de", include_str!("tutorials/pronunciation.de.yaml")),
    ("en", include_str!("tutorials/pronunciation.en.yaml")),
]);

fn lang_of(s: &str) -> Lang {
    Lang::from_key(s).expect("a language")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_else(|| panic!("{k} in {v}"))
}

#[test]
fn texts_and_keys_are_the_recorders() {
    let f: Value = serde_json::from_str(include_str!("fixtures/spoken.json")).expect("the fixture");
    let reference = key::sha256(s(&f, "reference").as_bytes());
    let transcript = s(&f, "transcript");
    let sections = f["sections"].as_array().expect("sections");
    assert!(sections.len() > 30);
    let extra =
        |lang: &str| List::parse(s(&f["extra_lists"], lang), lang_of(lang)).expect("it reads");
    let extra = [(Lang::En, extra("en")), (Lang::De, extra("de"))];
    let mut replaced = 0;
    let mut docs = 0;
    for x in sections {
        let lang = lang_of(s(x, "lang"));
        let where_ = s(x, "where");
        let raw = s(x, "raw");
        assert_eq!(spoken(raw, lang), s(x, "spoken"), "{where_}");
        let list = match x["list"].as_str() {
            Some("extra") => extra.iter().find(|(l, _)| *l == lang).map(|(_, l)| l),
            Some("docs") => {
                docs += 1;
                LISTS.get(lang)
            }
            _ => None,
        };
        let mut said = |engine: Engine, name: &str| {
            let said = pronounce::said_with(raw, lang, list, Some(engine));
            assert_eq!(said, s(&x["said"], name), "{where_} {name}");
            replaced += usize::from(said != s(x, "spoken"));
            said
        };
        let (fish_said, kokoro_said, eleven_said) = (
            said(Engine::Fish, "fish"),
            said(Engine::Kokoro, "kokoro"),
            said(Engine::ElevenLabs, "elevenlabs"),
        );
        let fish = FishKey {
            reference: reference.clone(),
            transcript: transcript.to_owned(),
            language: (lang == Lang::De).then(|| "German".to_owned()),
            seed: key::FISH_SEED,
        };
        assert_eq!(fish.key(&fish_said), s(x, "fish"), "{where_}");
        let speed = x["kokoro_speed"].as_f64().expect("a speed");
        let kokoro = key::kokoro_key(&key::untagged(&kokoro_said), s(x, "kokoro_voice"), speed);
        assert_eq!(kokoro, s(x, "kokoro"), "{where_}");
        let eleven = ElevenKey::new(s(&f, "elevenlabs_voice"), speed);
        assert_eq!(eleven.key(&eleven_said), s(x, "elevenlabs"), "{where_}");
        let told = ElevenKey {
            model_id: s(&f, "elevenlabs_model").to_owned(),
            language_code: Some(lang.key().to_owned()),
            ..eleven
        };
        assert_eq!(told.key(&eleven_said), s(x, "elevenlabs_told"), "{where_}");
    }
    assert!(docs >= 8, "the test tutorials' steps: {docs}");
    assert!(
        replaced >= 10,
        "the lists replace in the fixture: {replaced}"
    );
}

/// German number words as the recorder says them (`words:` of a list),
/// every number to 2000 and samples to beyond a billion, to the letter.
#[test]
fn number_words_are_the_recorders() {
    let f: Value = serde_json::from_str(include_str!("fixtures/spoken.json")).expect("the fixture");
    let numbers = f["numbers"].as_array().expect("numbers");
    assert!(numbers.len() > 2000);
    for pair in numbers {
        let digits = pair[0].as_str().expect("digits");
        let words = pair[1].as_str().expect("words");
        assert_eq!(pronounce::number_words(digits, Lang::De), words, "{digits}");
        assert_eq!(pronounce::number_words(digits, Lang::En), digits);
    }
}

/// The keys `scripts/lesson_voices.py` makes the lessons' sections by are
/// the lessons': a `say_lesson` in each of its forms, `say` and `text` in
/// the language or not, the script's speed, settings, model and language.
#[test]
fn the_lesson_voices_tools_keys_are_the_lessons() {
    let f: Value =
        serde_json::from_str(include_str!("fixtures/lesson_voices.json")).expect("the fixture");
    let mut checked = 0;
    let mut lessons = 0;
    for case in f["cases"].as_array().expect("cases") {
        let lang = lang_of(s(case, "lang"));
        let voice = s(case, "voice");
        let run = ElevenRun {
            model: case["model"].as_str(),
            language: case["language"].as_str(),
        };
        let n = narration(s(case, "script"), lang).expect("a narration");
        let want: Vec<(u64, String)> = (case["sections"].as_array().expect("sections").iter())
            .map(|x| {
                lessons += usize::from(x["lesson"].as_bool().expect("lesson"));
                (x["step"].as_u64().expect("a step"), s(x, "key").to_owned())
            })
            .collect();
        let got: Vec<(u64, String)> = (n.recorded.iter().enumerate())
            .filter_map(|(i, r)| {
                let x = narration::eleven_section(&LISTS, voice, run, lang, r.as_deref()?, &n)?;
                Some((i as u64 + 1, x.key))
            })
            .collect();
        assert_eq!(got, want, "{}", case["name"]);
        checked += got.len();
    }
    assert!(
        checked > 10 && lessons >= 4,
        "{checked} sections, {lessons} say_lesson"
    );
}

/// The test tutorials read as courses in both languages, their lists too.
#[test]
fn the_test_tutorials_read() {
    for lang in [Lang::En, Lang::De] {
        let c: egui_lessons::Course<()> = egui_lessons::script::parse_text(
            "tray",
            include_str!("tutorials/tray/script.yaml"),
            &[],
            lang,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.steps.len(), 6);
    }
    for (lang, list) in LISTS.each() {
        assert!(list.is_ok(), "pronunciation.{lang}.yaml: {:?}", list.err());
    }
}
