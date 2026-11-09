//! The pronunciation lists (OpenSolve's ADR 0056): what a voice is given
//! instead of what a script writes, as egui-screencast's runner does it
//! (`tutorial.pronounce`), so that a section's key is the recording's.
//! An app's `pronunciation.<lang>.yaml` beside its tutorials, embedded by
//! it and given as [`Lists`]: `all`, then an engine's own section
//! (`elevenlabs`, `fish`, `kokoro`), each a list of `{from: …, to: …}` (a
//! word or phrase as written, whole words, its case) or `{regex: …, to: …}`
//! (`\1`, `\g<name>` in `to`; `words: [2]` says group 2's digits as German
//! number words, in a German list only: [`number_words`]), applied in order
//! to the text as written (its Markdown gone), before units and signs are
//! read out ([`crate::spoken::read_out`]).

use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde_yaml::Value;

use crate::Lang;
use crate::spoken;
use crate::voice::Engine;

/// Each language's list as the app embeds it (`("de", its YAML)`), read
/// once, when first asked.
#[derive(Debug)]
pub struct Lists {
    table: &'static [(&'static str, &'static str)],
    read: OnceLock<Vec<(Lang, List)>>,
}

impl Lists {
    /// The lists of `table`: `pronunciation.<lang>.yaml` of each language,
    /// as (`"de"`, its text). An empty table: every text as spoken.
    pub const fn new(table: &'static [(&'static str, &'static str)]) -> Lists {
        Lists {
            table,
            read: OnceLock::new(),
        }
    }

    /// The list of `lang`, if it has one that reads (one that does not is
    /// said on stderr once and left out).
    pub fn get(&self, lang: Lang) -> Option<&List> {
        let read = self.read.get_or_init(|| {
            (self.table.iter())
                .filter_map(|(key, yaml)| {
                    let lang = Lang::from_key(key)?;
                    match List::parse(yaml, lang) {
                        Ok(list) => Some((lang, list)),
                        Err(e) => {
                            eprintln!("pronunciation.{key}.yaml: {e}");
                            None
                        }
                    }
                })
                .collect()
        });
        read.iter().find(|(l, _)| *l == lang).map(|(_, list)| list)
    }

    /// A step's text as `engine` is given it in `lang`
    /// (`tutorial.pronounce`): as text, the language's list applied, units
    /// and signs read out. What its cache key is made of, and what a local
    /// voice is sent.
    pub fn said(&self, text: &str, lang: Lang, engine: Engine) -> String {
        said_with(text, lang, self.get(lang), Some(engine))
    }

    /// Each list of the table, read anew, and whether it reads: for an
    /// app's test of its lists.
    pub fn each(&self) -> Vec<(&'static str, Result<List, String>)> {
        (self.table.iter())
            .map(|(l, y)| {
                let list = Lang::from_key(l)
                    .ok_or_else(|| format!("no language {l}"))
                    .and_then(|lang| List::parse(y, lang));
                (*l, list)
            })
            .collect()
    }
}

/// The sections of a list, in the order they apply.
const SECTIONS: [&str; 4] = ["all", "elevenlabs", "fish", "kokoro"];

/// A part of what a rule says: text, or a group of what it found.
#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    Index(usize),
    Name(String),
}

#[derive(Debug, Clone)]
enum Find {
    /// A word or phrase as written: whole words, its case.
    Phrase(String),
    Pattern(Regex),
}

/// One entry of a list.
#[derive(Debug, Clone)]
pub struct Rule {
    find: Find,
    to: Vec<Part>,
    /// The groups whose digits are said as number words (`words:`).
    words: Vec<usize>,
    lang: Lang,
}

/// A character of a word, for a phrase's whole-word match (Python's
/// `isalnum()`, or `_`).
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

static TEMPLATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\(?:(\d{1,2})|g<([^<>]*)>|(\\))").expect("a valid pattern"));

impl Rule {
    fn parse(entry: &Value, lang: Lang) -> Result<Rule, String> {
        let shape =
            || format!("an entry is {{from: …, to: …}} or {{regex: …, to: …}}, not {entry:?}");
        let map = entry
            .as_mapping()
            .filter(|m| m.len() == 2 + usize::from(m.contains_key("words")))
            .ok_or_else(shape)?;
        let to = map.get("to").and_then(Value::as_str).ok_or_else(shape)?;
        let source = |k: &str| map.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        let find = if let Some(phrase) = source("from") {
            Find::Phrase(phrase.to_owned())
        } else if let Some(pattern) = source("regex") {
            Find::Pattern(Regex::new(pattern).map_err(|e| format!("regex {pattern:?}: {e}"))?)
        } else {
            return Err(shape());
        };
        let words = match (map.get("words"), &find) {
            (None, _) => Vec::new(),
            (Some(_), Find::Phrase(_)) => return Err("words: only in a regex's entry".into()),
            (Some(_), Find::Pattern(_)) if lang != Lang::De => {
                return Err(format!(
                    "words: numbers are said as words in German only (pronunciation.de.yaml), not in {}",
                    lang.key()
                ));
            }
            (Some(w), Find::Pattern(p)) => {
                let groups = p.captures_len() - 1;
                let bad = || {
                    format!("words: a list of the regex's group numbers (1 to {groups}), not {w:?}")
                };
                let list = w.as_sequence().ok_or_else(bad)?;
                list.iter()
                    .map(|g| {
                        g.as_u64()
                            .and_then(|g| usize::try_from(g).ok())
                            .filter(|g| (1..=groups).contains(g))
                            .ok_or_else(bad)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        let to = Rule::template(to, &find)?;
        Ok(Rule {
            find,
            to,
            words,
            lang,
        })
    }

    /// `to` as its parts, its groups checked against what it replaces.
    fn template(to: &str, find: &Find) -> Result<Vec<Part>, String> {
        let bad = || format!("to {to:?}: only \\1, \\g<name> and \\\\ after a backslash");
        let mut parts = Vec::new();
        let mut at = 0;
        for c in TEMPLATE.captures_iter(to) {
            let m = c.get(0).expect("the match");
            let text = &to[at..m.start()];
            if text.contains('\\') {
                return Err(bad());
            }
            parts.push(Part::Text(text.to_owned()));
            if c.get(3).is_some() {
                parts.push(Part::Text("\\".into()));
            } else {
                let group = c.get(1).or_else(|| c.get(2)).map_or("", |g| g.as_str());
                let part = match group.parse::<usize>() {
                    Ok(n) if group.bytes().all(|b| b.is_ascii_digit()) => Part::Index(n),
                    _ => Part::Name(group.to_owned()),
                };
                let known = match (find, &part) {
                    (Find::Pattern(p), Part::Index(n)) => *n < p.captures_len(),
                    (Find::Pattern(p), Part::Name(n)) => {
                        p.capture_names().flatten().any(|x| x == n)
                    }
                    _ => false,
                };
                if !known {
                    return Err(format!("to {to:?}: no group {group} in what it replaces"));
                }
                parts.push(part);
            }
            at = m.end();
        }
        if to[at..].contains('\\') {
            return Err(bad());
        }
        parts.push(Part::Text(to[at..].to_owned()));
        Ok(parts)
    }

    /// `text` with this rule's replacements (`Rule.spans`, applied).
    fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        match &self.find {
            Find::Phrase(phrase) => {
                let first = phrase.chars().next().is_some_and(word_char);
                let last = phrase.chars().next_back().is_some_and(word_char);
                let mut i = 0;
                while let Some(k) = text[i..].find(phrase.as_str()).map(|k| k + i) {
                    let e = k + phrase.len();
                    let before = text[..k].chars().next_back();
                    let after = text[e..].chars().next();
                    if (!first || !before.is_some_and(word_char))
                        && (!last || !after.is_some_and(word_char))
                    {
                        out.push_str(&text[at..k]);
                        self.said(&mut out, None);
                        at = e;
                        i = e;
                    } else {
                        i = k + text[k..].chars().next().map_or(1, char::len_utf8);
                    }
                }
            }
            Find::Pattern(p) => {
                for c in p.captures_iter(text) {
                    let m = c.get(0).expect("the match");
                    if m.is_empty() {
                        continue;
                    }
                    out.push_str(&text[at..m.start()]);
                    self.said(&mut out, Some(&c));
                    at = m.end();
                }
            }
        }
        out.push_str(&text[at..]);
        out
    }

    fn said(&self, out: &mut String, found: Option<&regex::Captures<'_>>) {
        for part in &self.to {
            match (part, found) {
                (Part::Text(t), _) => out.push_str(t),
                (Part::Index(n), Some(c)) => {
                    self.group(out, *n, c.get(*n).map_or("", |g| g.as_str()));
                }
                (Part::Name(n), Some(c)) => {
                    let index = match &self.find {
                        Find::Pattern(p) => p.capture_names().position(|x| x == Some(n.as_str())),
                        Find::Phrase(_) => None,
                    };
                    let value = c.name(n).map_or("", |g| g.as_str());
                    match index {
                        Some(k) => self.group(out, k, value),
                        None => out.push_str(value),
                    }
                }
                _ => {}
            }
        }
    }

    /// Group `n`'s text as said (`Rule.group`): its digits as words when
    /// `words` has it.
    fn group(&self, out: &mut String, n: usize, value: &str) {
        if self.words.contains(&n) {
            out.push_str(&number_words(value, self.lang));
        } else {
            out.push_str(value);
        }
    }
}

const GERMAN_ONES: [&str; 10] = [
    "", "ein", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht", "neun",
];
const GERMAN_TEENS: [&str; 10] = [
    "zehn",
    "elf",
    "zwölf",
    "dreizehn",
    "vierzehn",
    "fünfzehn",
    "sechzehn",
    "siebzehn",
    "achtzehn",
    "neunzehn",
];
const GERMAN_TENS: [&str; 10] = [
    "", "", "zwanzig", "dreißig", "vierzig", "fünfzig", "sechzig", "siebzig", "achtzig", "neunzig",
];
/// The largest number said as words (`tutorial.GERMAN_MAX`); a larger
/// one stays digits.
const GERMAN_MAX: u64 = 999_999_999_999;

/// A digit's (or a count's below 10) entry of a table.
fn digit(table: &[&'static str; 10], k: u64) -> &'static str {
    usize::try_from(k)
        .ok()
        .and_then(|k| table.get(k))
        .copied()
        .unwrap_or_default()
}

/// 1 ≤ `n` < 1000 as one German word (`tutorial.german_below_thousand`):
/// a final 1 is "eins", one before more words "ein".
fn german_below_thousand(n: u64, last: bool) -> String {
    let mut out = String::new();
    let (hundreds, rest) = (n / 100, n % 100);
    if hundreds > 0 {
        out.push_str(digit(&GERMAN_ONES, hundreds));
        out.push_str("hundert");
    }
    match rest {
        0 => {}
        1 => out.push_str(if last { "eins" } else { "ein" }),
        2..=9 => out.push_str(digit(&GERMAN_ONES, rest)),
        10..=19 => out.push_str(digit(&GERMAN_TEENS, rest - 10)),
        _ => {
            let (tens, ones) = (rest / 10, rest % 10);
            if ones > 0 {
                out.push_str(digit(&GERMAN_ONES, ones));
                out.push_str("und");
            }
            out.push_str(digit(&GERMAN_TENS, tens));
        }
    }
    out
}

/// 0 ≤ `n` ≤ 999 999 999 999 as German number words
/// (`tutorial.german_number`): one word below a million
/// ("dreiundzwanzig", "eintausendeins"), "Million(en)" and
/// "Milliarde(n)" as words of their own ("eine Million zweihundert").
pub fn german_number(n: u64) -> String {
    if n == 0 {
        return "null".into();
    }
    let mut parts = Vec::new();
    let (billions, n) = (n / 1_000_000_000, n % 1_000_000_000);
    let (millions, n) = (n / 1_000_000, n % 1_000_000);
    for (count, one, many) in [
        (billions, "eine Milliarde", "Milliarden"),
        (millions, "eine Million", "Millionen"),
    ] {
        match count {
            0 => {}
            1 => parts.push(one.to_owned()),
            _ => parts.push(format!("{} {many}", german_below_thousand(count, false))),
        }
    }
    let (thousands, n) = (n / 1000, n % 1000);
    let mut word = String::new();
    if thousands > 0 {
        word.push_str(&german_below_thousand(thousands, false));
        word.push_str("tausend");
    }
    if n > 0 {
        word.push_str(&german_below_thousand(n, true));
    }
    if !word.is_empty() {
        parts.push(word);
    }
    parts.join(" ")
}

/// `digits` (ASCII digits only) as number words in `lang`
/// (`tutorial.number_words`); anything else, or a number too large, as
/// it is.
pub fn number_words(digits: &str, lang: Lang) -> String {
    if lang != Lang::De || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return digits.to_owned();
    }
    match digits.parse::<u64>() {
        Ok(n) if n <= GERMAN_MAX => german_number(n),
        _ => digits.to_owned(),
    }
}

/// A language's pronunciation list.
#[derive(Debug, Clone, Default)]
pub struct List {
    /// Each section's rules, by [`SECTIONS`].
    sections: [Vec<Rule>; 4],
}

impl List {
    /// A list as `pronunciation.<lang>.yaml` writes it, of `lang` (whose
    /// numbers `words:` says).
    pub fn parse(yaml: &str, lang: Lang) -> Result<List, String> {
        let v: Value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        let mut list = List::default();
        if v.is_null() {
            return Ok(list);
        }
        let map = v
            .as_mapping()
            .ok_or("sections all, elevenlabs, fish, kokoro, each a list")?;
        for (name, entries) in map {
            let name = name.as_str().unwrap_or_default();
            let k = (SECTIONS.iter().position(|s| *s == name))
                .ok_or_else(|| format!("section {name:?}: one of {}", SECTIONS.join(", ")))?;
            if entries.is_null() {
                continue;
            }
            let entries = entries
                .as_sequence()
                .ok_or_else(|| format!("{name}: a list"))?;
            for (i, e) in entries.iter().enumerate() {
                let rule = Rule::parse(e, lang).map_err(|e| format!("{name} {}: {e}", i + 1))?;
                list.sections[k].push(rule);
            }
        }
        Ok(list)
    }

    /// The rules for `engine`: `all`, then its own; only `all` for none.
    fn rules(&self, engine: Option<Engine>) -> impl Iterator<Item = &Rule> {
        // A remote voice has no section of its own: `all` only.
        let own = engine.and_then(|e| match e {
            Engine::ElevenLabs => Some(1),
            Engine::Fish => Some(2),
            Engine::Kokoro => Some(3),
            Engine::Remote => None,
        });
        self.sections[0]
            .iter()
            .chain(own.into_iter().flat_map(|k| self.sections[k].iter()))
    }

    /// Plain `text` (no Markdown) as `engine` is given it, before units
    /// are read out.
    pub fn apply(&self, text: &str, engine: Option<Engine>) -> String {
        self.rules(engine)
            .fold(text.to_owned(), |t, rule| rule.apply(&t))
    }
}

/// Plain `text` as `engine` is given it in `lang` with `list` (none:
/// as spoken; no engine: the list's `all` only): as text, the list applied,
/// units and signs read out.
pub fn said_with(text: &str, lang: Lang, list: Option<&List>, engine: Option<Engine>) -> String {
    let t = spoken::plain(text);
    let t = list.map_or(t.clone(), |l| l.apply(&t, engine));
    spoken::read_out(&t, lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A list as the recorder reads one: words whole and in their case, a
    /// regex's groups, `all` then the engine's own, before the units.
    #[test]
    fn a_pronunciation_list_is_the_recorders() {
        let list = List::parse(
            r"
all:
  - {regex: '\b(ISO|DIN) (?P<n>\d+)\b', to: '\1 \g<n>', words: [2]}
  - {from: ISO, to: Iso}
  - {from: XY-Ebene, to: Iks-Ypsilon-Ebene}
  - {from: 'z. B.', to: zum Beispiel}
  - {regex: '\bM(\d+),(\d+)\b', to: 'Em \1 Komma \2'}
  - {regex: '(?P<n>\d+) Stück', to: '\g<n> Teile \\'}
fish:
  - {from: Ebene, to: Ebehne}
kokoro: []
elevenlabs:
",
            Lang::De,
        )
        .expect("it reads");
        let said = |t: &str, e: Option<Engine>| said_with(t, Lang::De, Some(&list), e);
        assert_eq!(
            said(
                "Die **XY-Ebene**, z. B. mit M2,5 und 8 mm.",
                Some(Engine::Kokoro)
            ),
            "Die Iks-Ypsilon-Ebene, zum Beispiel mit Em 2 Komma 5 und 8 Millimeter."
        );
        assert_eq!(
            said("XY-Ebenen, xy-ebene, AXY-Ebene", None),
            "XY-Ebenen, xy-ebene, AXY-Ebene"
        );
        assert_eq!(
            said("die XY-Ebene", Some(Engine::Fish)),
            "die Iks-Ypsilon-Ebehne"
        );
        assert_eq!(said("die Ebene", Some(Engine::ElevenLabs)), "die Ebene");
        // A remote voice: `all` only.
        assert_eq!(
            said("die XY-Ebene", Some(Engine::Remote)),
            "die Iks-Ypsilon-Ebene"
        );
        assert_eq!(said("3 Stück", None), "3 Teile \\");
        assert_eq!(
            said("ISO 4762 und DIN 780, ISO 1000001", None),
            "Iso viertausendsiebenhundertzweiundsechzig und DIN siebenhundertachtzig, Iso eine Million eins"
        );
        for bad in [
            "fishaudio: []",
            "all: [{from: a}]",
            "all: [{from: a, regex: b, to: c}]",
            "all: [{regex: '(', to: c}]",
            r"all: [{regex: 'a(b)', to: '\2'}]",
            r"all: [{from: a, to: '\1'}]",
            r"all: [{regex: 'a', to: '\n'}]",
            "all: [{regex: '(?<=a)b', to: c}]",
            "all: [{from: a, to: b, words: [1]}]",
            r"all: [{regex: 'a(1)', to: '\1', words: [2]}]",
            r"all: [{regex: 'a(1)', to: '\1', words: 1}]",
            r"all: [{regex: 'a(1)', to: '\1', words: [true]}]",
        ] {
            assert!(List::parse(bad, Lang::De).is_err(), "{bad}");
        }
        let english = r"all: [{regex: 'a(1)', to: '\1', words: [1]}]";
        assert!(List::parse(english, Lang::En).is_err(), "words in English");
    }

    #[test]
    fn german_numbers_are_words() {
        assert_eq!(
            german_number(4762),
            "viertausendsiebenhundertzweiundsechzig"
        );
        assert_eq!(german_number(101), "einhunderteins");
        assert_eq!(german_number(1_000_000), "eine Million");
        assert_eq!(number_words("12", Lang::En), "12");
        assert_eq!(number_words("12a", Lang::De), "12a");
    }

    /// The lists of a table: read once, each the language's own, one that
    /// does not read left out.
    #[test]
    fn the_lists_of_a_table() {
        static TABLE: &[(&str, &str)] = &[
            ("de", "all: [{from: z. B., to: zum Beispiel}]"),
            ("en", "all: [{from: a}]"),
            ("xx", "all: []"),
        ];
        let lists = Lists::new(TABLE);
        assert_eq!(
            lists.said("z. B. 5 mm", Lang::De, Engine::Fish),
            "zum Beispiel 5 Millimeter"
        );
        assert!(lists.get(Lang::En).is_none());
        assert_eq!(
            lists.said("5 mm", Lang::En, Engine::Kokoro),
            "5 millimeters"
        );
        let each = lists.each();
        assert!(each[0].1.is_ok() && each[1].1.is_err() && each[2].1.is_err());
    }
}
