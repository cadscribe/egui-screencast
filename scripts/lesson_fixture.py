"""The fixture of egui-lessons' keys (`lessons/tests/fixtures/keys.json`):
the keys the recorder's caches keep a spoken section by, made by the
recorder's own code (`fishspeech.Voice.key`, `elevenlabs.Voice.key`,
`narrate.key`, `align.untagged`, `tutorial.elevenlabs_language`) for texts
with the awkward cases: delivery tags, quotes, backslashes, control
characters, Unicode spaces and separators, umlauts and marks. egui-lessons'
`voice::key` must make the same keys to the letter, so that a lesson plays
what a recording spoke.

    python3 scripts/lesson_fixture.py > lessons/tests/fixtures/keys.json

The standard library only. `--check` compares with the fixture instead of
writing it (CI: the fixture is the recorder's).
"""

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

FIXTURE = HERE.parent / "lessons" / "tests" / "fixtures" / "keys.json"
# A reference recording of no voice: only its bytes are hashed.
REFERENCE = b"RIFF egui-lessons fixture reference"
TRANSCRIPT = "A fixture's  transcript,\n with\tits spaces."
# Texts as said: as the app gives them to the keys, after its own
# `spoken()` and pronunciation list.
TEXTS = [
    "Extrude the sketch by ten millimeters.",
    "Say [pause] it, [excited] then go on.",
    "A [tag [nested] one] and a lone [ bracket.",
    'He said "quoted" and a back\\slash.',
    "Tabs\tand\nnew lines\r\nand  double  spaces.",
    "Control \x01 and \x1c separators \x1f here.",
    "Non breaking, ideographic　space, line separator.",
    "Zero​width stays.",
    "Ümlaute, ß und „Anführungszeichen“ — mit Gedankenstrich.",
    "Emoji 🙂 and math ∑ × → ›.",
    "  leading and trailing  ",
    "[pause]",
]
# Kokoro: its voices and speeds as `str(speed)` writes them.
KOKORO = [("am_michael", 1.0), ("bf_emma", 1.1), ("am_michael", 0.95), ("af_heart", 1.25), ("am_michael", 2.0)]
# ElevenLabs: the voice, its model, the script's speed and settings, and
# the language told.
ELEVEN = [
    ("HzBCRGsOskP44taMg2CT", None, 1.0, {}, None),
    ("HzBCRGsOskP44taMg2CT", "eleven_v3", 1.0, {}, "de"),
    ("abcDEF123", None, 1.1, {"stability": 0.4, "style": 1}, None),
    ("abcDEF123", "eleven_multilingual_v2", 0.9, {"use_speaker_boost": False, "extra": "x"}, "en"),
]
# What the recorder tells ElevenLabs: `--voice-language`, the script's
# `voice_language`, the run's language.
LANGUAGES = [
    (None, None, "en"),
    ("de", None, "en"),
    (None, "de", "en"),
    ("auto", None, "de"),
    ("AUTO", "en", "en"),
    ("off", "de", "de"),
    ("none", None, "de"),
    (None, "German", "en"),
    ("EN", None, "de"),
    ("fra", None, "de"),
    (" de ", None, "en"),
]


def fixture() -> dict:
    with tempfile.TemporaryDirectory() as d:
        ref = Path(d) / "reference.wav"
        ref.write_bytes(REFERENCE)
        fish_voices = {
            language: fishspeech.Voice("fixture", ref, TRANSCRIPT, language, {"seed": 1})
            for language in (None, "English", "German")
        }
        out = {
            "reference": REFERENCE.decode(),
            "transcript": TRANSCRIPT,
            "sections": [],
            "languages": [],
        }
        for text in TEXTS:
            out["sections"].append(
                {
                    "text": text,
                    "untagged": untagged(text),
                    "fish": {str(k): v.key(text) for k, v in fish_voices.items()},
                    "fish_language": {str(k): v.generation_language for k, v in fish_voices.items()},
                    "kokoro": [
                        {"voice": voice, "speed": speed, "key": narrate.key(untagged(text), voice, speed)}
                        for voice, speed in KOKORO
                    ],
                    "elevenlabs": [
                        {
                            "voice": voice,
                            "model": model,
                            "speed": speed,
                            "settings": settings,
                            "language": language,
                            "key": elevenlabs.Voice(
                                voice,
                                model_id=model or elevenlabs.MODEL,
                                settings=dict(elevenlabs.SETTINGS, speed=speed, **settings),
                                language_code=language,
                            ).key(text),
                        }
                        for voice, model, speed, settings, language in ELEVEN
                    ],
                }
            )
    for choice, script, lang in LANGUAGES:
        tutorial.VOICE_LANGUAGE = choice
        tutorial.LANG = lang
        script_dict = {"voice_language": script} if script else {}
        out["languages"].append(
            {"choice": choice, "script": script, "lang": lang, "code": tutorial.elevenlabs_language(script_dict)}
        )
    return out


def main() -> None:
    import os

    os.environ.pop("ELEVENLABS_LANGUAGE", None)
    text = json.dumps(fixture(), ensure_ascii=False, indent=1) + "\n"
    if sys.argv[1:] == ["--check"]:
        if FIXTURE.read_text(encoding="utf-8") != text:
            sys.exit(f"{FIXTURE} is not what the recorder makes: python3 scripts/lesson_fixture.py > {FIXTURE}")
        print(f"{FIXTURE.name}: the recorder's")
        return
    sys.stdout.write(text)


if __name__ == "__main__":
    main()
