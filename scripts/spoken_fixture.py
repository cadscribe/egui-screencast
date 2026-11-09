"""The fixture of what a lesson's voice is given (egui-lessons' `spoken`,
`pronounce` and `voice::key`; OpenSolve's ADR 0056): each step's narration
of a tutorials folder as the runner says it (`tutorial.spoken`), as each
engine is given it with the pronunciation list of its language
(`tutorial.pronounce`, `<tutorials>/pronunciation.<lang>.yaml`), and the
keys the caches keep the sections by (`fishspeech.Voice.key`,
`narrate.key`, `elevenlabs.Voice.key` with the German voice the
recordings use, without a language and as the German recordings make it:
`eleven_v3` told the language), and German number words
(`tutorial.number_words`), all by the recorder's own code beside this file.
A few extra texts with lists of their own cover every kind of entry.

    python scripts/spoken_fixture.py --tutorials lessons/tests/tutorials > lessons/tests/fixtures/spoken.json

egui-lessons' fixture is of its test tutorials (`lessons/tests/tutorials`);
an app writes its own of its tutorials, its reference named after it
(OpenSolve: `--tutorials docs/tutorials --name opensolve`), whenever a
script's narration or a list changes. PyYAML is needed. `--check FILE`
compares with FILE instead of writing.
"""

import argparse
import json
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import elevenlabs  # noqa: E402
import fishspeech  # noqa: E402
import narrate  # noqa: E402
import tutorial  # noqa: E402
from align import untagged  # noqa: E402

TRANSCRIPT = "A fixture's  transcript,\n with its spaces."
# The ElevenLabs voice of the German recordings, and the model they use
# with the language told (`--voice-model eleven_v3 --voice-language de`).
ELEVEN_VOICE = "HzBCRGsOskP44taMg2CT"
ELEVEN_MODEL = "eleven_v3"
# The numbers said as German words: every one to 2000, then samples.
NUMBERS = [str(n) for n in range(2001)] + [
    str(n)
    for n in (
        2021, 3960, 4762, 7089, 9999, 10000, 10001, 11011, 21771, 99999, 100000, 100001, 101101, 110000,
        999999, 1_000_000, 1_000_001, 1_001_000, 1_100_000, 2_000_000, 2_000_001, 21_500_001,
        101_000_001, 999_999_999, 1_000_000_000, 1_000_000_001, 2_001_001_001, 999_999_999_999,
        1_000_000_000_000, 10**20, 10**25,
    )
] + ["007", "0000", "00000000000000000000001", "", "12a", "١٢"]
# Texts the scripts do not have yet: tags, units, marks, Markdown.
EXTRA = {
    "en": [
        "Extrude by **10 mm**, [pause] then turn it 90° and use an M3 screw: 20 × 30 → done.",
        "A `code` mark, _emphasis_, a [link](https://example.org) and ![a picture](x.png).\n\n- a point\n# a heading",
        "JLCCNC cuts aluminium; [excited] it’s 3.5 mm  thick.",
        "Ümlaut, “quotes” and an em dash — kept.",
        "Export the 3MF of the XY plane; a CAD file, M2.5 screws, an M3-screw and C++.",
    ],
    "de": [
        "Extrudiere um **10 mm**, [pause] dann dreh es um 90° mit einer M3-Schraube: 20 × 30 → fertig.",
        "Öffne Datei › Neu; JLCCNC fräst Aluminium.",
        "Zeichne auf der XY-Ebene, z. B. eine Skizze mit M2,5 und 1,25 mm; die XY-Ebenen nicht, xy-ebene nicht.",
        "Die Skizze, die Ebene und ein Wert z2: z oder Z, Öl und Größe Ö.",
        "Schrauben nach ISO 4762, DIN 780 und ISO 21771; 0 Teile, 1000001 Teile und 007 Teile.",
    ],
}
# The pronunciation lists of the extra texts: every kind of entry, every
# engine's section.
EXTRA_LISTS = {
    "en": r"""
all:
  - {from: "3MF", to: "three M F"}
  - {from: "XY plane", to: "X Y plane"}
  - {from: "CAD", to: "C A D"}
  - {regex: 'A D file', to: 'A D data'}
  - {regex: '\bM(?P<size>\d+)\.(\d+)\b', to: 'M \g<size> point \2'}
  - {regex: '\bM(\d+)-', to: 'M \1 '}
  - {from: "C++", to: "C plus plus"}
  - {from: "aluminium", to: "aluminum"}
fish:
  - {from: "screws", to: "skrews"}
kokoro:
  - {from: "plane", to: "plain"}
elevenlabs: []
""",
    "de": r"""
all:
  - {from: "XY-Ebene", to: "Iks-Ypsilon-Ebene"}
  - {from: "z. B.", to: "zum Beispiel"}
  - {regex: '\bM(\d+),(\d+)\b', to: 'Em \1 Komma \2'}
  - {regex: '\bM(\d+)-', to: 'Em \1 '}
  - {regex: '\b(\d+),(\d)(\d)\b', to: '\1 Komma \2 \3'}
  - {from: "z", to: "Zett"}
  - {from: "JLCCNC", to: "J L C C N C"}
  - {from: "Ö", to: "Oe"}
  - {regex: '\b(ISO|DIN) (?P<n>\d+)\b', to: '\1 \g<n>', words: [2]}
  - {regex: '\b(\d+) Teile', to: '\1 Teile', words: [1]}
  - {from: "ISO", to: "Iso"}
fish:
  - {from: "Ebene", to: "Ebehne"}
elevenlabs:
  - {from: "Skizze", to: "Skitze"}
kokoro:
""",
}
# Each engine as the runner names it, and as the fixture does.
BACKENDS = {"fish": "fish-s2-pro", "kokoro": "kokoro", "elevenlabs": "elevenlabs"}


def fixture(tutorials: Path, name: str) -> dict:
    reference = f"RIFF {name} fixture reference".encode()
    tutorial.DOCS = tutorials
    with tempfile.TemporaryDirectory() as d:
        ref = Path(d) / "reference.wav"
        ref.write_bytes(reference)
        extra_lists = Path(d) / "lists"
        extra_lists.mkdir()
        no_lists = Path(d) / "none"
        no_lists.mkdir()
        for lang, text in EXTRA_LISTS.items():
            (extra_lists / f"pronunciation.{lang}.yaml").write_text(text, encoding="utf-8")
        voices = {
            "en": fishspeech.Voice("fixture", ref, TRANSCRIPT, "English", {"seed": 1}),
            "de": fishspeech.Voice("fixture", ref, TRANSCRIPT, "German", {"seed": 1}),
        }
        out = {
            "reference": reference.decode(),
            "transcript": TRANSCRIPT,
            "elevenlabs_voice": ELEVEN_VOICE,
            "elevenlabs_model": ELEVEN_MODEL,
            "extra_lists": EXTRA_LISTS,
            "numbers": [[n, tutorial.number_words(n, "de")] for n in NUMBERS],
            "sections": [],
        }

        def section(lang: str, raw: str, voice: str, speed: float, where: str, lists: str) -> None:
            tutorial.LANG = lang
            # The tutorials' lists beside them; the extra texts' their own, or none.
            tutorial.PRONUNCIATIONS = {"docs": None, "extra": extra_lists, "none": no_lists}[lists]
            said = {e: tutorial.said_text(raw, b, lang) for e, b in BACKENDS.items()}
            out["sections"].append(
                {
                    "where": where,
                    "lang": lang,
                    "raw": raw,
                    "list": lists,
                    "spoken": tutorial.spoken(raw),
                    "said": said,
                    "fish": voices[lang].key(said["fish"]),
                    "kokoro_voice": voice,
                    "kokoro_speed": speed,
                    "kokoro": narrate.key(untagged(said["kokoro"]), voice, speed),
                    # As `tutorial.speaker_of` makes the voice: the speed is the script's.
                    "elevenlabs": elevenlabs.Voice(
                        ELEVEN_VOICE, settings=dict(elevenlabs.SETTINGS, speed=speed)
                    ).key(said["elevenlabs"]),
                    # As the German recordings make it: the model told the
                    # language (`--voice-language auto`).
                    "elevenlabs_told": elevenlabs.Voice(
                        ELEVEN_VOICE,
                        model_id=ELEVEN_MODEL,
                        settings=dict(elevenlabs.SETTINGS, speed=speed),
                        language_code=lang,
                    ).key(said["elevenlabs"]),
                }
            )

        for script in sorted(tutorials.glob("*/script.yaml")):
            course = script.parent.name
            original = tutorial.load(course)
            for lang in tutorial.LANGS:
                if tutorial.missing_text(original, lang):
                    continue
                s = tutorial.localized(original, lang)
                voice = s.get("voice") or "am_michael"
                speed = float(s.get("speed", 1.0))
                for i, step in enumerate(s["steps"]):
                    raw = step.get("say") or step.get("text", "")
                    if raw:
                        section(lang, raw, voice, speed, f"{course}/{i + 1}", "docs")
        for lang, texts in EXTRA.items():
            for i, raw in enumerate(texts):
                section(lang, raw, "am_michael", 1.0, f"extra/{i + 1}", "none")
                section(lang, raw, "bf_emma", 1.1, f"extra/{i + 1}/fast", "none")
                section(lang, raw, "am_michael", 1.0, f"extra/{i + 1}/listed", "extra")
    return out


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--tutorials", type=Path, required=True, help="the tutorials folder")
    p.add_argument("--name", default="egui-lessons", help="the reference's name (egui-lessons)")
    p.add_argument("--check", type=Path, help="compare with this fixture instead of writing")
    a = p.parse_args()
    text = json.dumps(fixture(a.tutorials.resolve(), a.name), ensure_ascii=False, indent=1) + "\n"
    if a.check:
        if a.check.read_text(encoding="utf-8") != text:
            sys.exit(f"{a.check} is not what the recorder makes: write it again ({Path(__file__).name})")
        print(f"{a.check.name}: the recorder's")
        return
    sys.stdout.write(text)


if __name__ == "__main__":
    main()
