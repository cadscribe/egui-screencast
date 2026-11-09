//! The keys of `voice::key` against the recorder's own:
//! `tests/fixtures/keys.json`, written by `scripts/lesson_fixture.py` with
//! the recorder's Python (`fishspeech`, `elevenlabs`, `narrate`, `align`,
//! `tutorial`). A key that differs is a section a lesson does not find.

use egui_lessons::Lang;
use egui_lessons::voice::key::{self, ElevenKey, FishKey};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/keys.json")).expect("the fixture")
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_else(|| panic!("{k} in {v}"))
}

#[test]
fn the_keys_are_the_recorders() {
    let f = fixture();
    let reference = key::sha256(s(&f, "reference").as_bytes());
    let transcript = s(&f, "transcript");
    let sections = f["sections"].as_array().expect("sections");
    assert!(sections.len() >= 10);
    for section in sections {
        let text = s(section, "text");
        assert_eq!(key::untagged(text), s(section, "untagged"), "{text:?}");
        for (name, language) in [
            ("None", None),
            ("English", Some("English")),
            ("German", Some("German")),
        ] {
            let told = key::generation_language(language);
            assert_eq!(
                told.as_deref(),
                section["fish_language"][name].as_str(),
                "{name}"
            );
            let fish = FishKey {
                reference: reference.clone(),
                transcript: transcript.to_owned(),
                language: told,
                seed: key::FISH_SEED,
            };
            assert_eq!(
                fish.key(text),
                s(&section["fish"], name),
                "fish {name} {text:?}"
            );
        }
        for k in section["kokoro"].as_array().expect("kokoro") {
            let speed = k["speed"].as_f64().expect("a speed");
            assert_eq!(
                key::kokoro_key(&key::untagged(text), s(k, "voice"), speed),
                s(k, "key"),
                "kokoro {k} {text:?}"
            );
        }
        for e in section["elevenlabs"].as_array().expect("elevenlabs") {
            let mut eleven = ElevenKey::new(s(e, "voice"), e["speed"].as_f64().expect("a speed"));
            for (name, v) in e["settings"].as_object().expect("settings") {
                eleven.settings.insert(name.clone(), v.clone());
            }
            if let Some(m) = e["model"].as_str() {
                m.clone_into(&mut eleven.model_id);
            }
            eleven.language_code = e["language"].as_str().map(str::to_owned);
            assert_eq!(eleven.key(text), s(e, "key"), "elevenlabs {e} {text:?}");
        }
    }
}

#[test]
fn the_language_told_is_the_recorders() {
    let f = fixture();
    for l in f["languages"].as_array().expect("languages") {
        let lang = Lang::from_key(s(l, "lang")).expect("a language");
        assert_eq!(
            key::eleven_language(l["choice"].as_str(), l["script"].as_str(), lang).as_deref(),
            l["code"].as_str(),
            "{l}"
        );
    }
}
