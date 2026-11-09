"""The lessons' sections in ElevenLabs that no recording makes (OpenSolve's
ADR 0056; egui-lessons reads them).

A lesson reads each step aloud: its `say_lesson` in the language, else the
step's `say`, else its text, as the recorder reads them (the script in the
language). Where the language has an ElevenLabs voice and a course has
recorded sections, the app plays only what is in the voice cache; a step
that is not is silent. The recordings never make the `say_lesson` texts
("Danke fürs Mitmachen") nor a step changed since; this tool does.

For each course and language it lists exactly the sections the app reads,
keyed as the app and the recorder key them (the recorder's own code beside
this file: `tutorial.localized`, `tutorial.said_text` with the
pronunciation list of the language, `tutorial.speaker_of`,
`elevenlabs.Voice.key`), looks for each in the voice cache and says per
course how many are missing and how many characters they are. Nothing is
sent unless `--speak` is given: then the missing ones are spoken by
ElevenLabs (`elevenlabs.Narrator`, ELEVENLABS_API_KEY in the environment)
into the same cache, laid out as the recordings' (`<key>/audio.wav`,
`words.json`, `meta.json`). A dry run only reads the cache.

English is read by the recordings' Kokoro voice (`--lang en` without
`--voice`): a section counts as there in the voice cache or where the
recordings keep Kokoro's (`--work-audio`, `<work>/audio/<key>.wav`);
`--speak` has the venv's Kokoro say the missing ones into the voice cache
(local, nothing sent; HF_HOME as the recordings had it).

With the tutorials' venv (PyYAML; Kokoro for English), from the app's
repository (OpenSolve: `make lesson-voices`, `make lesson-voices-en`):

    python <egui-screencast>/scripts/lesson_voices.py --tutorials docs/tutorials \
        --voice-cache "$HOME/Library/Application Support/<App>/tutorial-voices" --lang de \
        --voice elevenlabs:<voice id> --voice-model eleven_v3 --voice-language de \
        [--courses a,b] [--list] [--speak]
"""

import argparse
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
# What the recorder would read besides its arguments, and the app never.
ENVIRONMENT = ("SCREENCAST_VOICE", "SCREENCAST_ENGINE", "ELEVENLABS_MODEL", "ELEVENLABS_LANGUAGE")


def use_screencast(tutorials: Path):
    """The recorder's `tutorial` and `elevenlabs` (beside this file), set to
    read the tutorials (`<tutorials>/<name>/script.yaml`) and their
    pronunciation lists (`<tutorials>/pronunciation.<lang>.yaml`)."""
    if str(HERE) not in sys.path:
        sys.path.insert(0, str(HERE))
    import elevenlabs  # noqa: PLC0415
    import tutorial  # noqa: PLC0415

    tutorial.DOCS = Path(tutorials)
    tutorial.PRONUNCIATIONS = None
    for k in ENVIRONMENT:
        os.environ.pop(k, None)
    return tutorial, elevenlabs


@dataclass
class Section:
    """A step's section as the app reads it in ElevenLabs."""

    course: str
    step: int  # from 1
    # Said by the lesson only (`say_lesson`): no recording makes it.
    lesson: bool
    # As written, before the pronunciation list.
    text: str
    # As ElevenLabs is given it (`tutorial.said_text`).
    said: str
    key: str
    # What ElevenLabs bills (`Narrator.say`): the text sent.
    characters: int


def lesson_only(step: dict, key: str, lang: str) -> str | None:
    """A step's `key` in `lang` only, as the app reads `say_lesson`
    (egui-lessons' `script::only_in`): `key_de` or `key: {de: …}`; in English `key` or
    `key: {en: …}`. No other language's text stands in."""
    value = None
    suffixed = step.get(f"{key}_{lang}") if lang != "en" else None
    if isinstance(suffixed, str):
        value = suffixed
    if value is None and isinstance(step.get(key), dict):
        entry = step[key].get(lang)
        value = entry if isinstance(entry, str) else None
    if value is None and lang == "en" and isinstance(step.get(key), str):
        value = step[key]
    value = value.strip() if value is not None else ""
    return value or None


def step_text(raw: dict, local: dict, lang: str) -> tuple[str | None, bool]:
    """What the lesson reads of a step (`raw` as written, `local` in the
    language), and whether only the lesson says it: `say_lesson` in the
    language, else `say`, else the text, as the recorder reads them."""
    said = lesson_only(raw, "say_lesson", lang)
    if said is not None:
        return said, True
    say, text = local.get("say"), local.get("text")
    chosen = say if isinstance(say, str) and say else text if isinstance(text, str) else None
    return (chosen if chosen and chosen.strip() else None), False


class Keyer:
    """The ElevenLabs voice of a script as the recorder makes it, from the
    run's choices (`--voice`, `--voice-model`, `--voice-language`)."""

    def __init__(self, tutorial, voice: str, model: str | None, language: str | None):
        self.tutorial = tutorial
        tutorial.VOICE = voice
        tutorial.VOICE_MODEL = model
        tutorial.VOICE_LANGUAGE = language
        tutorial.VOICE_SETTINGS = {}

    def voice(self, local: dict, lang: str):
        """The `elevenlabs.Voice` of the script `local` (in `lang`)."""
        self.tutorial.LANG = lang
        # A name given: the recorder asks ElevenLabs for none.
        speaker = self.tutorial.speaker_of(dict(local, voice_name="-"), dry_run=True)
        if speaker["backend"] != "elevenlabs":
            sys.exit("lesson-voices: --voice elevenlabs:<voice id>")
        return speaker["narrator"]().voice


def sections_of(tutorial, elevenlabs, keyer: Keyer, course: str, script: dict, lang: str):
    """The sections of `script` the app reads in `lang`, and its voice."""
    tutorial.LANG = lang
    local = tutorial.localized(script, lang)
    voice = keyer.voice(local, lang)
    out = []
    for i, raw in enumerate(script.get("steps") or []):
        text, lesson = step_text(raw, local["steps"][i], lang)
        if text is None:
            continue
        said = tutorial.said_text(text, "elevenlabs", lang)
        sent = elevenlabs.normalized(said)
        if not sent:
            continue
        out.append(Section(course, i + 1, lesson, text, said, voice.key(said), len(sent)))
    return out, voice


def kokoro_sections(tutorial, course: str, script: dict) -> tuple[list[Section], str, float]:
    """The sections of `script` an English lesson reads by the recordings'
    Kokoro (`make tutorial … --fast`), keyed as `narrate.key` of the text
    untagged, the script's Kokoro voice and its speed; and that voice and
    speed."""
    import elevenlabs  # noqa: PLC0415
    import narrate  # noqa: PLC0415

    tutorial.LANG = "en"
    local = tutorial.localized(script, "en")
    # As `--fast` chooses it: the script's own Kokoro voice, not the run's.
    before = getattr(tutorial, "FAST", False), getattr(tutorial, "VOICE", None)
    tutorial.FAST, tutorial.VOICE = True, None
    try:
        _, voice = tutorial.voice_choice(local)
    finally:
        tutorial.FAST, tutorial.VOICE = before
    speed = float(local.get("speed", 1.0))
    out = []
    for i, raw in enumerate(script.get("steps") or []):
        text, lesson = step_text(raw, local["steps"][i], "en")
        if text is None:
            continue
        said = elevenlabs.untagged(tutorial.said_text(text, "kokoro", "en"))
        if not said:
            continue
        out.append(Section(course, i + 1, lesson, text, said, narrate.key(said, voice, speed), len(said)))
    return out, voice, speed


def kokoro_has(key: str, cache: Path, work_audio: Path | None) -> bool:
    """Whether a Kokoro section is spoken: in the voice cache's layout, or
    as the recordings keep it (`<work>/audio/<key>.wav`)."""
    if (cache / key / "audio.wav").is_file():
        return True
    return work_audio is not None and (work_audio / f"{key}.wav").is_file()


def kokoro_speak(s: Section, voice: str, speed: float, cache: Path) -> None:
    """Kokoro (the tutorial venv's, local) says `s` into the voice cache,
    laid out as the recordings' sections: `<key>/audio.wav`, `words.json`,
    `meta.json`, written into a folder of its own and renamed into place."""
    import narrate  # noqa: PLC0415

    tmp = cache / f".{s.key}.{os.getpid()}.tmp"
    tmp.mkdir(parents=True, exist_ok=True)
    narrate.speak(s.said, tmp / "audio.wav", voice, speed)
    narrate.words_file(tmp / "audio.wav").replace(tmp / "words.json")
    meta = {"text": s.said, "backend": "kokoro", "model": narrate.REPO, "revision": narrate.REVISION,
            "voice": voice, "speed": speed, "characters": len(s.said)}
    (tmp / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
    tmp.replace(cache / s.key)


def english(a, tutorial, names: list[str], speak=None) -> int:
    """`--lang en`: the sections of the recordings' Kokoro voice; with
    `--speak` the missing ones spoken by the venv's Kokoro (local, nothing
    sent). `speak` stands in for `kokoro_speak` in the tests."""
    missing: dict[str, tuple] = {}
    rows = []
    for name in names:
        sections, voice, speed = kokoro_sections(tutorial, name, tutorial.load(name))
        gone = [s for s in sections if not kokoro_has(s.key, a.voice_cache, a.work_audio)]
        for s in gone:
            missing.setdefault(s.key, (s, voice, speed))
        rows.append((name, len(sections), len(gone), sum(s.lesson for s in gone)))
        if a.list:
            for s in gone:
                kind = "say_lesson" if s.lesson else "step"
                print(f"  {name} {s.step} ({kind}, {s.key}): {' '.join(s.text.split())}")
    print("Kokoro (the recordings' English voice), en")
    print(f"voice cache: {a.voice_cache}; recordings: {a.work_audio}")
    width = max([len(r[0]) for r in rows] + [len("course")])
    print(f"  {'course':<{width}}  sections  missing  say_lesson")
    for name, total, gone, lesson in rows:
        print(f"  {name:<{width}}  {total:>8}  {gone:>7}  {lesson:>10}")
    print(f"  {'all':<{width}}  {sum(r[1] for r in rows):>8}  {len(missing):>7}  (each text once)")
    if not a.speak:
        if missing:
            print("dry run: --speak speaks them (Kokoro, local; OpenSolve: "
                  "make lesson-voices-en LESSON_VOICES_ARGS=--speak)")
        return 0
    for s, voice, speed in missing.values():
        (speak or kokoro_speak)(s, voice, speed, a.voice_cache)
        print(f"  {s.course} {s.step}: {s.key}", flush=True)
    print(f"Kokoro: {len(missing)} sections spoken")
    return 0


def courses(tutorials: Path, names: list[str]) -> list[str]:
    """The courses: every tutorial with a script (as the app embeds them),
    or those named."""
    every = sorted(d.name for d in Path(tutorials).iterdir() if (d / "script.yaml").is_file())
    unknown = [n for n in names if n not in every]
    if unknown:
        sys.exit(f"lesson-voices: no course {', '.join(unknown)} (of {', '.join(every)})")
    return [n for n in every if not names or n in names]


def main(argv: list[str] | None = None, opener=None, speak=None) -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--tutorials", type=Path, required=True,
                   help="the tutorials: <name>/script.yaml and pronunciation.<lang>.yaml")
    p.add_argument("--lang", default="de", help="the language of the lessons: de or en (de)")
    p.add_argument("--voice", help="the ElevenLabs voice the app plays in that language: elevenlabs:<voice id>")
    p.add_argument("--voice-model", help="the model the recordings use (eleven_v3)")
    p.add_argument("--voice-language", help="the language ElevenLabs is told (de, auto, off)")
    p.add_argument("--voice-cache", type=Path, required=True,
                   help="the voice cache: the app's lasting one (<App>/tutorial-voices)")
    p.add_argument("--work-audio", type=Path,
                   help="English: Kokoro's sections as the recordings keep them (<work>/audio)")
    p.add_argument("--courses", default="", help="the courses, comma-separated (all)")
    p.add_argument("--list", action="store_true", help="each missing section's text")
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", default=True, help="only count (the default)")
    mode.add_argument("--speak", action="store_true", help="speak the missing sections: by ElevenLabs (paid); "
                      "in English without --voice by Kokoro (local)")
    a = p.parse_args(argv)
    names = courses(a.tutorials, [n.strip() for n in a.courses.split(",") if n.strip()])
    if a.lang == "en" and not a.voice:
        tutorial, _ = use_screencast(a.tutorials)
        return english(a, tutorial, names, speak)
    if not a.voice:
        sys.exit(f"lesson-voices: no ElevenLabs voice for {a.lang}: --voice elevenlabs:<voice id>")
    tutorial, elevenlabs = use_screencast(a.tutorials)
    keyer = Keyer(tutorial, a.voice, a.voice_model, a.voice_language)
    cache = elevenlabs.Cache(a.voice_cache)

    missing: dict[str, tuple] = {}  # key: (section, voice), each once
    rows = []
    voice = None
    for name in names:
        script = tutorial.load(name)
        sections, voice = sections_of(tutorial, elevenlabs, keyer, name, script, a.lang)
        # Read only: `Cache.has`, not `Cache.get`, which marks an entry used.
        gone = [s for s in sections if not cache.has(s.key)]
        for s in gone:
            missing.setdefault(s.key, (s, voice))
        rows.append((name, len(sections), len(gone), sum(s.lesson for s in gone), sum(s.characters for s in gone)))
        if a.list:
            for s in gone:
                kind = "say_lesson" if s.lesson else "step"
                print(f"  {name} {s.step} ({kind}, {s.key}): {' '.join(s.text.split())}")

    model = voice.model_id if voice else "-"
    told = voice.language_code if voice else None
    print(f"ElevenLabs {a.voice.partition(':')[2] or a.voice} ({model}, language {told or 'not told'}), {a.lang}")
    print(f"voice cache: {a.voice_cache}")
    width = max([len(r[0]) for r in rows] + [len("course")])
    print(f"  {'course':<{width}}  sections  missing  say_lesson  characters")
    for name, total, gone, lesson, chars in rows:
        print(f"  {name:<{width}}  {total:>8}  {gone:>7}  {lesson:>10}  {chars:>10}")
    chars = sum(s.characters for s, _ in missing.values())
    print(f"  {'all':<{width}}  {sum(r[1] for r in rows):>8}  {len(missing):>7}  "
          f"{sum(s.lesson for s, _ in missing.values()):>10}  {chars:>10}  (each text once)")
    if not a.speak:
        if missing:
            print("dry run: nothing sent; --speak speaks them (ELEVENLABS_API_KEY in the environment; "
                  'OpenSolve: make lesson-voices LESSON_VOICES_ARGS="--speak")')
        return 0

    spoken = 0
    sent = 0
    for s, v in missing.values():
        narrator = elevenlabs.Narrator(v, a.voice_cache, dry_run=False, **({"opener": opener} if opener else {}))
        try:
            narrator.say(s.said)
        except elevenlabs.SpeechError as e:
            print(f"lesson-voices: {s.course} {s.step}: {e}", file=sys.stderr)
            print(f"{spoken} spoken, {sent} characters sent; the rest when run again", file=sys.stderr)
            return 1
        spoken += narrator.spoken
        sent += narrator.characters
        print(f"  {s.course} {s.step}: {s.key} ({s.characters} characters)", flush=True)
    print(f"ElevenLabs: {spoken} sections spoken, {sent} characters sent")
    return 0


if __name__ == "__main__":
    sys.exit(main())
