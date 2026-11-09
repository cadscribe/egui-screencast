//! A step's text as the recorder's runner speaks it (`tutorial.spoken`),
//! to the letter: Markdown as text, then units, sizes and signs read out.
//! With a pronunciation list the list comes between the two
//! ([`crate::pronounce::said_with`]).

use std::sync::LazyLock;

use regex::Regex;

use crate::Lang;
use crate::voice::key::single_spaced;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static PICTURE: LazyLock<Regex> = LazyLock::new(|| re(r"!\[[^\]]*\]\([^)]*\)"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[([^\]]+)\]\([^)]*\)"));
static MARKS: LazyLock<Regex> = LazyLock::new(|| re(r"[*_`]+"));
static LEADS: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)^\s*[-#>]+\s*"));
static MM: LazyLock<Regex> = LazyLock::new(|| re(r"(\d)\s*mm\b"));
static DEGREES: LazyLock<Regex> = LazyLock::new(|| re(r"(\d)\s*°"));
static THREAD: LazyLock<Regex> = LazyLock::new(|| re(r"\bM(\d)"));

/// Markdown as text (`tutorial.plain`): pictures, links, emphasis and
/// code marks gone, the spaces single.
pub fn plain(markdown: &str) -> String {
    let t = PICTURE.replace_all(markdown, "");
    let t = LINK.replace_all(&t, "$1");
    let t = MARKS.replace_all(&t, "");
    let t = LEADS.replace_all(&t, "");
    single_spaced(&t)
}

/// Text as the voice reads it without a pronunciation list
/// (`tutorial.spoken`): units and sizes spelled out in `lang`. With a
/// list: [`crate::pronounce::said_with`].
pub fn spoken(text: &str, lang: Lang) -> String {
    read_out(&plain(text), lang)
}

/// Units, sizes and signs read out in `lang` (`tutorial.READ_OUT`): after
/// the pronunciation list.
pub fn read_out(t: &str, lang: Lang) -> String {
    if lang == Lang::De {
        let t = MM.replace_all(t, "${1} Millimeter");
        let t = DEGREES.replace_all(&t, "${1} Grad");
        let t = THREAD.replace_all(&t, "M ${1}");
        let t = t.replace('×', "mal").replace('›', ",").replace('→', "zu");
        return t.replace("JLCCNC", "JLC CNC");
    }
    let t = MM.replace_all(t, "${1} millimeters");
    let t = DEGREES.replace_all(&t, "${1} degrees");
    let t = THREAD.replace_all(&t, "M ${1}");
    let t = t.replace('×', "by").replace('›', ",").replace('→', "to");
    t.replace("JLCCNC", "JLC CNC")
        .replace("aluminium", "aluminum")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::key::untagged;

    #[test]
    fn tags_go_to_the_voice_but_not_to_kokoro() {
        let said = spoken("Then [pause] extrude by **5 mm**.", Lang::En);
        assert_eq!(said, "Then [pause] extrude by 5 millimeters.");
        assert_eq!(untagged(&said), "Then extrude by 5 millimeters.");
    }

    #[test]
    fn markdown_is_read_as_text() {
        assert_eq!(
            spoken(
                "A `code` mark, _emphasis_, a [link](https://x.org) and ![a picture](x.png).\n- a point",
                Lang::En
            ),
            "A code mark, emphasis, a link and . a point"
        );
        assert_eq!(
            spoken("Datei › Neu, 20 × 30 → 90°, M3", Lang::De),
            "Datei , Neu, 20 mal 30 zu 90 Grad, M 3"
        );
    }
}
