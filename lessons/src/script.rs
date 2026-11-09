//! A tutorial's script read as a course (OpenSolve's ADR 0054): its steps
//! with their texts in a language, their actions as the recorder plays
//! them, their cues checked against what they say, and the app's own keys
//! of a step (`check:`, `intent:` …) as its [`Extra`]. `FORMAT.md` beside
//! this crate says every key.

use std::path::Path;

use serde_json::Value;

use crate::Lang;
use crate::spoken::spoken;
use crate::voice::key::untagged;

/// A script built into the app (its `build.rs`): the folder's name, the
/// script, and the files its `open:` actions read, by the path written.
#[derive(Debug)]
pub struct Embedded {
    pub name: &'static str,
    pub script: &'static str,
    pub files: &'static [(&'static str, &'static [u8])],
}

/// A step's own keys that only the app reads (`check:`, `intent:`, …),
/// parsed once with the course. `()` for an app without them.
pub trait Extra: Clone + std::fmt::Debug + Send + Sync + 'static {
    /// The app's keys of a step: `raw` the step as written, `localized`
    /// with its values in `lang` ([`localized`]).
    ///
    /// # Errors
    ///
    /// What is wrong with them, plainly; the course does not read.
    fn parse(raw: &Value, localized: &Value, lang: Lang) -> Result<Self, String>;
}

impl Extra for () {
    fn parse(_: &Value, _: &Value, _: Lang) -> Result<(), String> {
        Ok(())
    }
}

/// The key of a hint's target that is the app's inline editor, a value
/// typed in where a click opened it ([`Step::targets`]).
pub const EDITOR: &str = "editor";

/// Whether a hint's target is the inline editor.
pub fn is_editor(target: &Value) -> bool {
    target.get(EDITOR).is_some()
}

/// A course: a tutorial's script in a language.
#[derive(Debug, Clone)]
pub struct Course<E> {
    /// Its folder among the tutorials.
    pub name: String,
    pub title: String,
    pub summary: String,
    pub steps: Vec<Step<E>>,
    /// A hash of the script: what an app caches by what it made of it.
    pub version: u64,
    pub files: &'static [(&'static str, &'static [u8])],
    /// Made by the app (an assistant's lesson), not built in: the app sets
    /// it; a parsed course is built in.
    pub made: bool,
    /// The language it was read in: its texts', its actions' and its
    /// extra's values' (`{en: …, de: …}`).
    pub lang: Lang,
}

/// A step of a course.
#[derive(Debug, Clone)]
pub struct Step<E> {
    pub name: String,
    pub title: String,
    /// Markdown, as on the page: `text_lesson`, should the lesson say
    /// otherwise than the page and the video, else `text`.
    pub text: String,
    /// Where the interface has it.
    pub ui: Option<String>,
    /// A card's points, for the intro and the ending.
    pub points: Vec<String>,
    /// Its actions with their values in the course's language.
    pub actions: Vec<Value>,
    pub hint: Option<String>,
    /// The app's own keys of the step.
    pub extra: E,
}

/// The app's keys are read as the step's own (`step.checks` for
/// `step.extra.checks`).
impl<E> std::ops::Deref for Step<E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.extra
    }
}

impl<E> std::ops::DerefMut for Step<E> {
    fn deref_mut(&mut self) -> &mut E {
        &mut self.extra
    }
}

impl<E> Step<E> {
    /// Whether it does something in the app: not only waits, as a card.
    pub fn acts(&self) -> bool {
        self.actions.iter().any(|a| {
            !["wait", "cue", "shot"]
                .iter()
                .any(|k| a.get(k).is_some() && a.as_object().is_some_and(|o| o.len() == 1))
        })
    }

    /// The targets of its clicks, in order, each with the text typed
    /// into it should it be a field (`click: {field: …}` and a `type:`
    /// before the next click): what the pointer hints show. A `type:`
    /// after a click on anything else (a line to dimension, Fillet) is
    /// typed into the inline editor that click opens: a target of its own
    /// ([`EDITOR`]).
    pub fn targets(&self) -> Vec<(Value, Option<String>)> {
        (self.targets_escaped().into_iter())
            .map(|(t, typed, _)| (t, typed))
            .collect()
    }

    /// For each of `targets`, whether the script presses Escape before it
    /// (after the click before): the tool at hand is put away first, the
    /// click is on something already there.
    pub fn escapes(&self) -> Vec<bool> {
        (self.targets_escaped().into_iter())
            .map(|(.., escaped)| escaped)
            .collect()
    }

    /// Whether the script presses Escape after the step's last click: the
    /// next step starts with no tool at hand.
    pub fn ends_escaped(&self) -> bool {
        let last_click = self.actions.iter().rposition(|a| a.get("click").is_some());
        (self.actions.iter().skip(last_click.map_or(0, |i| i + 1)))
            .any(|a| a.get("key").and_then(Value::as_str) == Some("Escape"))
    }

    fn targets_escaped(&self) -> Vec<(Value, Option<String>, bool)> {
        let mut out: Vec<(Value, Option<String>, bool)> = Vec::new();
        let mut escaped = false;
        for a in &self.actions {
            if let Some(t) = a.get("click") {
                out.push((t.clone(), None, escaped));
                escaped = false;
            } else if a.get("key").and_then(Value::as_str) == Some("Escape") {
                escaped = true;
            } else if let Some(text) = a.get("type").and_then(Value::as_str)
                && let Some(last) = out.last_mut()
                && last.1.is_none()
            {
                if last.0.get("field").is_some() {
                    last.1 = Some(text.to_owned());
                } else {
                    let editor = serde_json::json!({ EDITOR: true });
                    out.push((editor, Some(text.to_owned()), false));
                }
            }
        }
        out
    }
}

/// A text of the script in `lang`: `key_de` or `key: {de: …}` in German,
/// `key` or `key: {en: …}` in English; English where there is no German.
pub fn text(v: &Value, key: &str, lang: Lang) -> String {
    let in_lang = |l: Lang| -> Option<&str> {
        let suffixed = (l != Lang::En).then(|| v.get(format!("{key}_{}", l.key()))?.as_str());
        suffixed
            .flatten()
            .or_else(|| v.get(key)?.get(l.key())?.as_str())
            .or_else(|| (l == Lang::En).then(|| v.get(key)?.as_str()).flatten())
    };
    in_lang(lang)
        .or_else(|| in_lang(Lang::En))
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// A text of a step as the lesson shows it in `lang`: `key_lesson`
/// (`key_lesson_de`, `key_lesson: {en: …, de: …}`) where the lesson says
/// otherwise than the page and the video (a file it opens itself instead
/// of File › Open), else `key`. The recorder never reads `key_lesson`.
pub fn lesson_text(v: &Value, key: &str, lang: Lang) -> String {
    let lesson = text(v, &format!("{key}_lesson"), lang);
    if lesson.is_empty() {
        text(v, key, lang)
    } else {
        lesson
    }
}

/// A step's `key` in `lang` only, no other language standing in: `key_de`,
/// `key: {de: …}`; for English `key` or `key: {en: …}`. Trimmed; none when
/// empty. How a lesson reads `say_lesson`.
pub fn only_in(step: &Value, key: &str, lang: Lang) -> Option<String> {
    let suffixed = (lang != Lang::En)
        .then(|| step.get(format!("{key}_{}", lang.key()))?.as_str())
        .flatten();
    let said = suffixed
        .or_else(|| step.get(key)?.get(lang.key())?.as_str())
        .or_else(|| {
            (lang == Lang::En)
                .then(|| step.get(key)?.as_str())
                .flatten()
        })?;
    Some(said.trim().to_owned()).filter(|s| !s.is_empty())
}

/// `value` in `lang`, as the recorder's `localized()` makes a script: a
/// map whose keys are all languages (`{en: …, de: …}`) its entry in
/// `lang` (the English one where it has none), `key_de` beside `key` in
/// its place in German; inside maps and lists alike, the rest as it is.
pub fn localized(value: &Value, lang: Lang) -> Value {
    let is_lang = |k: &str| Lang::ALL.iter().any(|l| l.key() == k);
    // `width_de`: `width` in German, never a key of its own.
    let suffixed = |k: &str| -> Option<(String, String)> {
        let (base, suffix) = k.rsplit_once('_')?;
        (!base.is_empty() && is_lang(suffix)).then(|| (base.to_owned(), suffix.to_owned()))
    };
    match value {
        Value::Object(o) if !o.is_empty() && o.keys().all(|k| is_lang(k)) => {
            let v = o.get(lang.key()).or_else(|| o.get(Lang::En.key()));
            v.map_or(Value::Null, |v| localized(v, lang))
        }
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, v) in o {
                if suffixed(k).is_some() {
                    continue;
                }
                let v = o.get(&format!("{k}_{}", lang.key())).unwrap_or(v);
                out.insert(k.clone(), localized(v, lang));
            }
            for (k, v) in o {
                if let Some((base, suffix)) = suffixed(k)
                    && suffix == lang.key()
                    && !o.contains_key(&base)
                {
                    out.insert(base, localized(v, lang));
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| localized(v, lang)).collect()),
        v => v.clone(),
    }
}

/// The words of `text` as the recorder compares them (`tutorial.normal`):
/// small letters and digits, an apostrophe within a word kept.
pub fn words(text: &str) -> Vec<String> {
    static WORD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[a-z0-9]+(?:'[a-z0-9]+)*").expect("a valid pattern")
    });
    let lower = text.to_lowercase();
    WORD.find_iter(&lower)
        .map(|m| m.as_str().to_owned())
        .collect()
}

/// A step's cues as the recorder needs them: each a text, or a map of
/// the languages with a text each (an unquoted comma in a flow map,
/// `{en: b, the face width, de: …}`, makes keys of its own); and each
/// cue's words in `lang` said in the step's narration in `lang` (`say`,
/// else `text`, as the recorder speaks it), in order, each after the one
/// before (`tutorial.cue_actions`). `raw` is the step as written, `l` in
/// `lang`.
///
/// # Errors
///
/// A cue is not a text, or not said where it stands.
pub fn cues(raw: &Value, l: &Value, lang: Lang) -> Result<(), String> {
    let is_lang = |k: &str| Lang::ALL.iter().any(|x| x.key() == k);
    for a in raw["actions"].as_array().into_iter().flatten() {
        let Some(c) = a.get("cue") else { continue };
        match c {
            Value::String(_) | Value::Number(_) => {}
            Value::Object(o) => {
                let bad: Vec<&str> = (o.iter())
                    .filter(|(k, v)| !is_lang(k) || !v.is_string())
                    .map(|(k, _)| k.as_str())
                    .collect();
                if !bad.is_empty() || o.is_empty() {
                    return Err(format!(
                        "the cue {c} is not a text or {{en: …, de: …}} with a text each: \
                         its keys {bad:?} (a comma in it must be quoted)"
                    ));
                }
            }
            _ => return Err(format!("the cue {c} is not a text")),
        }
    }
    // What the recorder speaks in `lang`: the step's `say`, else its text.
    let said = [l.get("say"), l.get("text")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|t| !t.trim().is_empty())
        .unwrap_or_default();
    let said = words(&untagged(&spoken(said, lang)));
    let mut after = 0;
    for a in l["actions"].as_array().into_iter().flatten() {
        let Some(cue) = a.get("cue").and_then(Value::as_str) else {
            continue;
        };
        let want = words(&spoken(cue, lang));
        let found = (!want.is_empty())
            .then(|| {
                (after..=said.len().saturating_sub(want.len()))
                    .find(|&k| said[k..].starts_with(&want))
            })
            .flatten();
        let Some(k) = found else {
            return Err(format!(
                "the cue {cue:?} is not in what it says in {}, or not after the cue before it",
                lang.key()
            ));
        };
        after = k + want.len();
    }
    Ok(())
}

/// A course from an embedded script.
///
/// # Errors
///
/// It is no YAML, has no title or steps, a cue is wrong, or the app's keys
/// of a step ([`Extra::parse`]).
pub fn parse<E: Extra>(e: &Embedded, lang: Lang) -> Result<Course<E>, String> {
    parse_text(e.name, e.script, e.files, lang)
}

/// A course from a script's text: `name` its folder's, or a made
/// lesson's.
///
/// # Errors
///
/// As [`parse`].
pub fn parse_text<E: Extra>(
    name: &str,
    script: &str,
    files: &'static [(&'static str, &'static [u8])],
    lang: Lang,
) -> Result<Course<E>, String> {
    let course = name;
    let v: Value = serde_yaml::from_str(script).map_err(|err| format!("{course}: {err}"))?;
    let text = |v: &Value, key: &str| text(v, key, lang);
    let title = text(&v, "title");
    if title.is_empty() {
        return Err(format!("{course}: no title"));
    }
    let mut steps = Vec::new();
    for (i, s) in v["steps"].as_array().into_iter().flatten().enumerate() {
        let name = s["name"]
            .as_str()
            .map_or_else(|| format!("{}", i + 1), str::to_owned);
        let at = |err: String| format!("{course}: step {} ({name}): {err}", i + 1);
        // Its actions and the app's keys with their values in `lang`; its
        // texts read by `text()` below.
        let l = localized(s, lang);
        cues(s, &l, lang).map_err(at)?;
        let extra = E::parse(s, &l, lang).map_err(at)?;
        let card = s.get("card");
        steps.push(Step {
            title: text(s, "title"),
            text: lesson_text(s, "text", lang),
            ui: Some(lesson_text(s, "ui", lang)).filter(|u| !u.is_empty()),
            points: card
                .and_then(|c| {
                    let de = (lang == Lang::De)
                        .then(|| c["points_de"].as_array())
                        .flatten();
                    de.or_else(|| c["points"].as_array())
                })
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            actions: l["actions"].as_array().cloned().unwrap_or_default(),
            hint: Some(lesson_text(s, "hint", lang)).filter(|h| !h.is_empty()),
            extra,
            name,
        });
    }
    if steps.is_empty() {
        return Err(format!("{course}: no steps"));
    }
    Ok(Course {
        name: course.to_owned(),
        title,
        summary: lesson_text(&v, "summary", lang),
        steps,
        version: {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            script.hash(&mut h);
            h.finish()
        },
        files,
        made: false,
        lang,
    })
}

/// `{work}` in every string the course's folder, as the recorder's runner
/// makes it.
pub fn with_work(v: &Value, work: &Path) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace("{work}", &work.display().to_string())),
        Value::Array(a) => Value::Array(a.iter().map(|x| with_work(x, work)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), with_work(x, work)))
                .collect(),
        ),
        v => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r"
title: {en: Tray, de: Schale}
summary: A tray.
summary_lesson_de: Eine Schale, schon offen.
steps:
  - name: open
    title: Open
    text: Open **the file**.
    text_de: Öffne **die Datei**.
    say: Open it, then extrude by 10 mm.
    say_de: Öffne sie, dann extrudiere um 10 mm.
    actions:
      - cue: {en: extrude by 10 mm, de: extrudiere}
      - click: {button: Extrude}
      - type: {en: depth, de: Tiefe}
      - click: {field: Depth}
      - type: '10'
      - key: Escape
      - click: {view: line}
      - type: '5'
  - text: Done.
    card: {points: [One, Two], points_de: [Eins, Zwei]}
    actions: [{wait: 1}]
";

    #[test]
    fn a_course_in_each_language() {
        let en: Course<()> = parse_text("tray", SCRIPT, &[], Lang::En).expect("it reads");
        assert_eq!(en.title, "Tray");
        assert_eq!(en.summary, "A tray.");
        assert_eq!(en.steps[0].text, "Open **the file**.");
        assert_eq!(en.steps[1].name, "2");
        assert_eq!(en.steps[1].points, ["One", "Two"]);
        assert!(en.steps[0].acts() && !en.steps[1].acts());
        let de: Course<()> = parse_text("tray", SCRIPT, &[], Lang::De).expect("it reads");
        assert_eq!(de.title, "Schale");
        assert_eq!(de.summary, "Eine Schale, schon offen.");
        assert_eq!(de.steps[0].actions[2]["type"], "Tiefe");
        assert_eq!(de.steps[1].points, ["Eins", "Zwei"]);
        assert_eq!(de.version, en.version);
    }

    #[test]
    fn the_targets_of_a_step() {
        let c: Course<()> = parse_text("tray", SCRIPT, &[], Lang::En).expect("it reads");
        let s = &c.steps[0];
        let targets = s.targets();
        assert_eq!(targets.len(), 5, "{targets:?}");
        assert_eq!(targets[0].1, None);
        assert!(
            is_editor(&targets[1].0),
            "a type after a button: the editor"
        );
        assert_eq!(targets[1].1.as_deref(), Some("depth"));
        assert_eq!(targets[2].1.as_deref(), Some("10"), "typed into the field");
        assert!(is_editor(&targets[4].0));
        assert_eq!(s.escapes(), [false, false, false, true, false]);
        assert!(!s.ends_escaped());
    }

    #[test]
    fn a_cue_must_be_said() {
        let bad = SCRIPT.replace("cue: {en: extrude by 10 mm", "cue: {en: revolve");
        let e = parse_text::<()>("tray", &bad, &[], Lang::En).expect_err("not said");
        assert!(e.starts_with("tray: step 1 (open): the cue"), "{e}");
        assert!(parse_text::<()>("tray", &bad, &[], Lang::De).is_ok());
        let comma = SCRIPT.replace("{en: extrude by 10 mm, de: extrudiere}", "{en: a, b}");
        assert!(parse_text::<()>("tray", &comma, &[], Lang::En).is_err());
        assert!(parse_text::<()>("x", "title: X\nsteps: []", &[], Lang::En).is_err());
        assert!(parse_text::<()>("x", "steps: [{text: a}]", &[], Lang::En).is_err());
    }

    /// The app's keys, parsed with the course; their error is the step's.
    #[test]
    fn the_apps_keys_are_its_extra() {
        #[derive(Debug, Clone)]
        struct Checks(Vec<String>);
        impl Extra for Checks {
            fn parse(raw: &Value, l: &Value, _: Lang) -> Result<Checks, String> {
                if raw.get("check") == Some(&Value::Bool(true)) {
                    return Err("check: a list".into());
                }
                Ok(Checks(
                    (l["check"].as_array().into_iter().flatten())
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect(),
                ))
            }
        }
        let script = "title: T\nsteps:\n  - text: a\n    check: [{en: x, de: y}]\n";
        let c: Course<Checks> = parse_text("t", script, &[], Lang::De).expect("it reads");
        assert_eq!(c.steps[0].0, ["y"], "read as the step's own");
        let bad = "title: T\nsteps:\n  - name: s\n    text: a\n    check: true\n";
        let e = parse_text::<Checks>("t", bad, &[], Lang::En).expect_err("wrong");
        assert_eq!(e, "t: step 1 (s): check: a list");
    }

    #[test]
    fn texts_in_a_language() {
        let v: Value = serde_json::json!({"say": {"en": "Hello", "de": "Hallo"}, "text": "Plain",
            "say_lesson_de": " Mach mit. ", "say_lesson": "Follow."});
        assert_eq!(text(&v, "say", Lang::De), "Hallo");
        assert_eq!(text(&v, "text", Lang::De), "Plain");
        assert_eq!(
            only_in(&v, "say_lesson", Lang::De).as_deref(),
            Some("Mach mit.")
        );
        assert_eq!(
            only_in(&v, "say_lesson", Lang::En).as_deref(),
            Some("Follow.")
        );
        assert_eq!(only_in(&v, "text", Lang::De), None);
        let l = localized(
            &serde_json::json!({"a": [{"w": 1, "w_de": 2}, {"en": "x"}]}),
            Lang::De,
        );
        assert_eq!(l, serde_json::json!({"a": [{"w": 2}, "x"]}));
        assert_eq!(words("It's 10 MM, ok?"), ["it's", "10", "mm", "ok"]);
        let w = with_work(&serde_json::json!({"open": "{work}/a"}), Path::new("/w"));
        assert_eq!(w["open"], "/w/a");
    }
}
