"""The fixture of the lessons' own sections (egui-lessons'
`voice::narration`; OpenSolve's ADR 0056): the sections `lesson_voices.py`
finds in a script of the awkward cases (`say_lesson` in one language or
both, suffixed keys, a language without its own `say`, tags, units,
ElevenLabs' settings, model and language), and their keys, made by the
tool's own code (`tutorial` and `elevenlabs` underneath), which a lesson
must make alike. The pronunciation lists are those of the tutorials
folder given.

    python scripts/lesson_voices_fixture.py --tutorials lessons/tests/tutorials \\
        > lessons/tests/fixtures/lesson_voices.json

An app writes its own with its tutorials' lists (OpenSolve:
`--tutorials docs/tutorials`). PyYAML is needed. `--check FILE` compares
with FILE instead of writing.
"""

import argparse
import json
import sys
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lesson_voices  # noqa: E402

SCRIPT = r"""
title: T
speed: 1.0
speed_de: 1.1
voice_settings: {stability: 0.4}
voice_model: eleven_turbo_v2_5
voice_language: de
steps:
  - name: thanks
    text: Done.
    say: {en: Thanks for watching, de: Danke fürs Zuschauen}
    say_lesson:
      en: Thanks for following along.
      de: |
        OpenSolve, die Beispiele und weitere Tutorials findest du auf GitHub.
        Danke fürs Mitmachen.
  - name: suffixed
    text: x
    say: Watch.
    say_de: Schau.
    say_lesson: Follow along.
    say_lesson_de: Mach mit, die Platte ist schon geöffnet.
  - name: lesson in English only
    text: x
    say: Watch the plate.
    say_de: Schau dir die Platte an.
    say_lesson: {en: Only in the lesson.}
  - name: say in English only
    text: {en: x, de: y}
    say: Watch.
  - name: text only
    text: {en: "A 10 mm plate, an M3 screw, ISO 4762 and 3.4 mm.", de: "Eine Platte von 10 mm, eine M3-Schraube nach ISO 4762, z. B. 3,4 mm und 90°."}
  - name: empty say
    say: ""
    text: Only text.
    text_de: Nur **Text**, mit einem [Link](https://example.org).
  - name: nothing
    text: ""
  - name: tagged
    text: x
    say: {en: "Now [pause] the next part.", de: "Jetzt [pause] der nächste Teil › weiter."}
"""

ELEVEN_DE = "HzBCRGsOskP44taMg2CT"
# Each run as the app makes it: the voice, its model and language over the
# script's (none: the script's).
CASES = [
    ("German recordings", "de", ELEVEN_DE, "eleven_v3", "de"),
    ("the script's model and language", "de", ELEVEN_DE, None, None),
    ("English, the language told by auto", "en", "voice-en", None, "auto"),
    ("English, not told", "en", "voice-en", "eleven_multilingual_v2", "off"),
]


def fixture(tutorials: Path) -> dict:
    tutorial, elevenlabs = lesson_voices.use_screencast(tutorials)
    script = yaml.safe_load(SCRIPT)
    out = {"cases": []}
    for name, lang, voice, model, language in CASES:
        keyer = lesson_voices.Keyer(tutorial, f"elevenlabs:{voice}", model, language)
        sections, _ = lesson_voices.sections_of(tutorial, elevenlabs, keyer, "t", script, lang)
        out["cases"].append(
            {
                "name": name,
                "lang": lang,
                "voice": voice,
                "model": model,
                "language": language,
                "script": SCRIPT,
                "sections": [
                    {"step": s.step, "lesson": s.lesson, "said": s.said, "key": s.key} for s in sections
                ],
            }
        )
    return out


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--tutorials", type=Path, required=True, help="the tutorials folder (its lists)")
    p.add_argument("--check", type=Path, help="compare with this fixture instead of writing")
    a = p.parse_args()
    text = json.dumps(fixture(a.tutorials.resolve()), ensure_ascii=False, indent=1) + "\n"
    if a.check:
        if a.check.read_text(encoding="utf-8") != text:
            sys.exit(f"{a.check} is not what lesson_voices.py makes: write it again ({Path(__file__).name})")
        print(f"{a.check.name}: lesson_voices.py's")
        return
    sys.stdout.write(text)


if __name__ == "__main__":
    main()
