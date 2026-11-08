#!/usr/bin/env python3
"""Builds an egui app's tutorials from their scripts, with egui-screencast.

A tutorial is `<tutorials>/<name>/script.yaml`: a title, a summary and
steps, each with its text, what it does in the app and its picture. This
script, for each tutorial named:

1. speaks each step's text, in the run's language (`--lang en|de`), with
   Fish Audio S2 Pro run locally by mlx-speech (`fishspeech.py`, the
   default), Kokoro (`narrate.py`, `--fast`) or ElevenLabs
   (`elevenlabs.py`, `--voice elevenlabs:<voice id>`), each section kept in
   a cache and spoken again only when it changed, or, with `--no-voice`,
   guesses how long it would take; first, over the first step's card, the
   voice says that it is an AI voice and why (`voice_intro`);
2. writes the plan for the app: the steps' actions, pictures and seconds;
3. starts the app (`--app`, with its `--app-arg`s and `--env`s) with
   `--play plan.json --record dir --headless`: the app plays the steps
   without a window, drawn at 1920 × 1080 on the GPU, and takes its pictures
   itself (its `egui_screencast::Player`);
4. copies the pictures to `<tutorials>/<name>/images/` and writes
   `<tutorials>/<name>/README.md` (`README.de.md` in German);
5. unless `--no-video`, makes `<tutorials>/<name>/<name>.mp4` with ffmpeg
   (`<name>.de.mp4` in German):
   the frames, the narration under them and the text as subtitles.

    python tutorial.py --app target/release/app [--app-arg ARG]… [--env K=V]…
        [--tutorials docs/tutorials] [--work target/tutorials]
        [--no-voice] [--no-video] [--window] [NAME…]
        [--voice elevenlabs:<voice id>] [--voice-model M] [--voice-setting K=V]…
        [--voice-cache DIR] [--dry-run] [--prune-voice-cache DAYS]
        [--app-name NAME] [--voice-intro TEXT | --no-voice-intro]
        [--lang en|de|en,de] [--fast] [--engine E] [--voices DIR] [--fish-python PY]
        [--remix] [--out DIR]

What is said and what is seen go together: an action `cue: "Open it"`
waits until the narration says those words (the times of its words, from
Kokoro or ElevenLabs, or found by forced alignment for S2 Pro),
and should the actions before it take longer, the words from there on wait
instead. `shot: true` takes the step's picture there rather than at the
step's end. The subtitles are the words spoken, when they are spoken. The
voice is said to be synthetic on the cards, in a corner of the video, on
the page and in the video's metadata (`voice_notice` in another language).

In an `--app-arg` or `--env`, `{lang}` is the run's language. In an
`--app-arg`, `{work}` is the tutorial's work folder (an app's own
settings folder, say). Paths are taken from the folder it is run in. Without
names, every tutorial is built. Work files are kept in `<work>/<name>/`.

Where a cue's words wait, the narration is cut at the quiet just before
the cue's word (its time from the aligner comes a little late), each piece
faded in and out. `--remix` makes the narration, the subtitles and the
video again from a recording already made (`<work>/<name>/recording`, its
`plan.json`) and the cached sections, without the app and without speaking
anything. `--out DIR` writes the video and its narration and subtitles to
`DIR/<name>[.<lang>]/` instead.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import elevenlabs  # noqa: E402  (the standard library only)

# Set from the command line in `main`.
ROOT = Path.cwd()
DOCS = ROOT / "docs" / "tutorials"
WORK = ROOT / "target" / "tutorials"
APP = Path()
APP_ARGS: list[str] = []
APP_ENV: dict[str, str] = {}
# Played in a window instead of without one (`--window`).
WINDOW = False

FPS = 30
# Seconds of quiet after a step's narration, before the next step.
PAUSE = 0.8
# Words a second, when no narration is spoken (`--no-voice`).
WORDS_PER_SECOND = 2.6
# The video's size: the window (1280 × 800 points) at 1.5 pixels a point.
VIDEO_WIDTH = 1920
SHOT_WIDTH = 1600
# That the voice is synthetic, for whoever watches (EU AI Act, Art. 50): on
# the cards and in a corner of the video, on the page, and in the video's
# metadata. A script in another language says it in its own
# (`voice_notice`).
VOICE_NOTICE = "Voice: AI-generated (Kokoro)"
# The language of this run (`--lang`): what is said and written, and the
# voice. A script's text in another language is `say: {en: …, de: …}` or
# `say_de:` beside `say:`; a plain one is English.
LANGS = ("en", "de")
LANG = "en"
# What the runner itself writes and says, in each language.
STRINGS = {
    "en": {
        "notice": "Voice: AI-generated ({voice})",
        "intro": "This video is narrated by an AI voice, so it can always show the current version of {app}.",
        "the_app": "the app",
        "needs": "You need",
        "screencast": "**Screencast:** `{video}` beside this page, with narration, is built by "
        "`make tutorial NAME={name}` (not in the repository, see [the index](../README.md)).",
        "page_notice": "The screencast's narration is synthetic speech, made by {made_by} from the text "
        "of this page; no person speaks it. The video says so on its first card and in a corner "
        "throughout, its voice says so at its start, and its metadata says so too.",
        "in_app": "In the app",
        "pending": "For now through the assistant's tools.",
        "next": "Next",
        "subtitles": "eng",
        "language": None,
    },
    "de": {
        "notice": "Stimme: KI-generiert ({voice})",
        "intro": "Dieses Video wird von einer KI-Stimme gesprochen, damit es immer die aktuelle Version von "
        "{app} zeigen kann.",
        "the_app": "der App",
        "needs": "Du brauchst",
        "screencast": "**Screencast:** `{video}` neben dieser Seite, mit Sprecherstimme, baut "
        "`make tutorial NAME={name}` (nicht im Repository, siehe [die Übersicht](../README.md)).",
        "page_notice": "Die Stimme des Screencasts ist synthetisch, erzeugt von {made_by} aus dem Text "
        "dieser Seite; kein Mensch spricht sie. Das Video sagt es auf seiner ersten Karte und die "
        "ganze Zeit in einer Ecke, seine Stimme sagt es zu Beginn, und seine Metadaten sagen es auch.",
        "in_app": "In der App",
        "pending": "Vorerst über die Werkzeuge des Assistenten.",
        "next": "Weiter",
        "subtitles": "deu",
        "language": "German",
    },
}
# The voices of Fish Audio S2 Pro by language (its voices.yaml), the default.
FISH_VOICES = {"en": "en-gb-female", "de": "de-male"}
# A Kokoro voice's name (am_michael, bf_emma): used by Kokoro only.
KOKORO_NAME = re.compile(r"^[a-z][fm]_[a-z]+$")
# Kokoro (`--fast`) instead of the engine chosen.
FAST = False
# The voice (`--voice`, else $SCREENCAST_VOICE, else the script's), the
# ElevenLabs model and settings over the script's, and where ElevenLabs'
# sections are kept: set from the command line in `main`.
KOKORO_VOICE = "am_michael"
# The names of Fish Audio S2 Pro, run locally (`fishspeech.py`).
FISH_NAMES = ("fish-s2-pro", "fishaudio-s2-pro", "fishaudio")
# The Python of mlx-speech (`--fish-python`, $FISH_SPEECH_PYTHON, else the
# `mlx-speech` command's).
FISH_PYTHON: Path | None = None
# Fish S2 Pro's voices: a folder with a voices.yaml (`--voices`,
# $SCREENCAST_VOICES, a script's `voices:`); the engine of a bare voice
# (`--engine`, $SCREENCAST_ENGINE, a script's `engine:`).
VOICES: str | None = None
ENGINE: str | None = None
# The user's own Hugging Face cache, before the runner sets its own.
USER_HF_HOME = os.environ.get("HF_HOME")
VOICE: str | None = None
VOICE_MODEL: str | None = None
VOICE_SETTINGS: dict = {}
VOICE_CACHE = WORK / "voices"
# What the voice says first, over the first step's card: that it is an AI
# voice, and why (a script's `voice_intro`, `--voice-intro`; off only by
# `voice_intro: false` or `--no-voice-intro`). `{app}` is the app's name.
VOICE_INTRO_DEFAULT = "This video is narrated by an AI voice, so it can always show the current version of {app}."
# None: the default of the run's language.
VOICE_INTRO: str | bool | None = None
APP_NAME: str | None = None
# Seconds of quiet between that line and the first step's words.
INTRO_PAUSE = 0.6
# A subtitle ends at a sentence's end, at a cue, or at a comma once it is
# this long.
SUBTITLE_CHARS = 80
# Where the words after a cue wait, the cut is at the quietest point of the
# narration from this long before the cue's word to this long after it
# (forced alignment finds a word some 70 ms late), in frames this long.
CUT_BEFORE = 0.30
CUT_AFTER = 0.03
CUT_FRAME = 0.010
# Each piece of a cut narration is faded in and out over this long.
FADE = 0.006
# Made again from a recording and the cached sections (`--remix`).
REMIX = False
# The made files' folder (`--out`): else the video beside the script, its
# narration and subtitles in the work folder.
OUT: Path | None = None

def fail(message: str) -> None:
    sys.exit(f"tutorials: {message}")


def load(name: str) -> dict:
    path = DOCS / name / "script.yaml"
    try:
        import yaml
    except ImportError:
        fail("PyYAML is missing: make tutorial-venv, or pip install -r the requirements.txt beside this script")
    with open(path, encoding="utf-8") as f:
        script = yaml.safe_load(f)
    for key in ("title", "steps"):
        if key not in script:
            fail(f"{path}: no {key}")
    return script


def localized(value, lang: str):
    """`value` in `lang`: a mapping of languages (`{en: …, de: …}`) its
    entry (English when it has none for `lang`), `key_<lang>` beside `key`
    in its place; the rest as it is."""
    if isinstance(value, dict):
        if value and all(k in LANGS for k in value):
            return localized(value.get(lang, value.get("en")), lang)
        out = {}
        for k, v in value.items():
            base, _, suffix = str(k).rpartition("_")
            if base and suffix in LANGS:
                continue
            out[k] = localized(value.get(f"{k}_{lang}", v), lang)
        for k, v in value.items():
            base, _, suffix = str(k).rpartition("_")
            if base and suffix == lang and base not in value:
                out[base] = localized(v, lang)
        return out
    if isinstance(value, list):
        return [localized(v, lang) for v in value]
    return value


def missing_text(script: dict, lang: str) -> list[int]:
    """The steps with narration but none in `lang` (plain text is English)."""
    out = []
    for i, step in enumerate(script.get("steps", [])):
        said = False
        given = False
        for key in ("say", "text"):
            v = step.get(key)
            if v is None and step.get(f"{key}_{lang}") is None:
                continue
            given = True
            if step.get(f"{key}_{lang}") is not None:
                said = True
            elif isinstance(v, dict):
                said = said or lang in v
            elif lang == "en":
                said = True
        if given and not said:
            out.append(i + 1)
    return out


def text(key: str, **kw) -> str:
    """What the runner writes or says, in the run's language."""
    return STRINGS[LANG][key].format(**kw)


def suffix() -> str:
    """Of the run's files: none for English, `.de` for German."""
    return "" if LANG == "en" else f".{LANG}"


def work_dir(name: str) -> Path:
    return WORK / f"{name}{suffix()}"


def made_dir(name: str) -> Path:
    """Where the narration, subtitles and joined sections go: the work
    folder, or `--out`'s `<name>[.<lang>]`."""
    return OUT / f"{name}{suffix()}" if OUT else work_dir(name)


def shown(path: Path) -> Path:
    """`path` from the folder run in, when it is below it."""
    try:
        return path.relative_to(ROOT)
    except ValueError:
        return path


def plain(markdown: str) -> str:
    """Markdown as text: links, emphasis and code marks gone."""
    t = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", markdown)
    t = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", t)
    t = re.sub(r"[*_`]+", "", t)
    t = re.sub(r"^\s*[-#>]+\s*", "", t, flags=re.M)
    return " ".join(t.split())


def spoken(text: str) -> str:
    """Text as a voice should read it: units and sizes spelled out."""
    t = plain(text)
    if LANG == "de":
        t = re.sub(r"(\d)\s*mm\b", r"\1 Millimeter", t)
        t = re.sub(r"(\d)\s*°", r"\1 Grad", t)
        t = re.sub(r"\bM(\d)", r"M \1", t)
        t = t.replace("×", "mal").replace("›", ",").replace("→", "zu")
        return t.replace("JLCCNC", "JLC CNC")
    t = re.sub(r"(\d)\s*mm\b", r"\1 millimeters", t)
    t = re.sub(r"(\d)\s*°", r"\1 degrees", t)
    t = re.sub(r"\bM(\d)", r"M \1", t)
    t = t.replace("×", "by").replace("›", ",").replace("→", "to")
    t = t.replace("JLCCNC", "JLC CNC").replace("aluminium", "aluminum")
    return t


def shot_name(i: int, step: dict) -> str | None:
    shot = step.get("shot", True)
    if shot is False:
        return None
    if isinstance(shot, str):
        return shot
    return f"{i + 1:02d}-{step['name']}.png"


def guessed_words(text: str) -> list[dict]:
    """The words of `text` as if spoken at `WORDS_PER_SECOND`, when it is
    not (`--no-voice`): for cues and subtitles all the same."""
    words = []
    for i, w in enumerate(text.split()):
        words.append(
            {
                "text": w,
                "start": round(i / WORDS_PER_SECOND, 3),
                "end": round((i + 1) / WORDS_PER_SECOND, 3),
                "space": True,
            }
        )
    return words


def voice_choice(script: dict) -> tuple[str, str]:
    """The engine and the voice: `--voice`, else `$SCREENCAST_VOICE`, else
    the script's `voice`, else Kokoro's am_michael. `elevenlabs:<voice id>`
    is ElevenLabs, `fish-s2-pro:<voice>` Fish Audio S2 Pro (run locally),
    `kokoro:<voice>` Kokoro; a bare voice is of `--engine`, else
    `$SCREENCAST_ENGINE`, else the script's `engine`, else Kokoro."""
    choice = str(VOICE or os.environ.get("SCREENCAST_VOICE") or script.get("voice") or "").strip()
    backend, sep, rest = choice.partition(":")
    backend = backend.lower()
    if sep and backend in ("elevenlabs", "kokoro", *FISH_NAMES):
        if not rest.strip():
            fail(f"voice {choice!r}: no voice after {backend}:")
        engine, choice = ("fish-s2-pro" if backend in FISH_NAMES else backend), rest.strip()
    else:
        engine = str(ENGINE or os.environ.get("SCREENCAST_ENGINE") or script.get("engine") or "fish-s2-pro")
        engine = engine.strip().lower()
        engine = "fish-s2-pro" if engine in FISH_NAMES else engine
    if FAST:
        engine = "kokoro"
    if engine == "kokoro":
        return "kokoro", choice if KOKORO_NAME.match(choice) else KOKORO_VOICE
    if engine == "fish-s2-pro":
        # A Kokoro voice in a script (voice: am_michael) is Kokoro's only.
        return "fish-s2-pro", choice if choice and not KOKORO_NAME.match(choice) else FISH_VOICES[LANG]
    if engine == "elevenlabs":
        if not choice:
            fail("ElevenLabs needs a voice: elevenlabs:<voice id>")
        return "elevenlabs", choice
    fail(f"engine {engine!r}: fish-s2-pro, kokoro or elevenlabs")


class Kokoro:
    """Kokoro's narration, a section at a time, kept in `<work>/audio` by
    the hash of its text, voice and speed (as before)."""

    def __init__(self, voice: str, speed: float, dry_run: bool = False):
        sys.path.insert(0, str(Path(__file__).parent))
        import narrate

        self.kokoro = narrate
        self.voice = voice
        self.speed = speed
        self.dry_run = dry_run
        self.cached = 0
        self.spoken = 0
        self.characters = 0

    def say(self, text: str) -> tuple[Path | None, float, list[dict]]:
        # Kokoro would read delivery tags aloud.
        text = elevenlabs.untagged(text)
        wav = WORK / "audio" / f"{self.kokoro.key(text, self.voice, self.speed)}.wav"
        words = self.kokoro.words(wav) if wav.exists() else None
        if words is not None:
            self.cached += 1
            return wav, self.kokoro.duration(wav), words
        self.spoken += 1
        self.characters += len(text)
        if self.dry_run:
            return None, 0.0, []
        # Spoken again when the words of an older run were not kept.
        print(f"  narration: {text[:60]}…", flush=True)
        seconds = self.kokoro.speak(text, wav, self.voice, self.speed)
        return wav, seconds, self.kokoro.words(wav) or []

    def report(self) -> str:
        total = self.cached + self.spoken
        verb = "to speak" if self.dry_run else "spoken"
        return f"Kokoro: {total} sections, {self.cached} from the cache, {self.spoken} {verb}"


def speaker_of(script: dict, dry_run: bool = False, opener=None) -> dict:
    """Who speaks: the backend that says each section (`narrator`) and how
    the notices name the voice."""
    backend, voice = voice_choice(script)
    speed = float(script.get("speed", 1.0))
    if backend == "kokoro":
        if LANG != "en":
            fail(f"Kokoro speaks no {LANG}: fish-s2-pro speaks it (without --fast)")
        return {
            "backend": "kokoro",
            "voice": voice,
            "label": "Kokoro",
            "made_by": f"the Kokoro-82M text-to-speech model (voice {voice})",
            "made_by_md": f"the Kokoro-82M text-to-speech model (voice `{voice}`)",
            "track": f"Kokoro-82M, {voice}",
            "narrator": lambda: Kokoro(voice, speed, dry_run),
        }
    if backend == "fish-s2-pro":
        import fishspeech

        settings = dict(fishspeech.SETTINGS)
        settings.update(script.get("voice_settings") or {})
        settings.update(VOICE_SETTINGS)
        settings.setdefault("language", script.get("voice_language") or STRINGS[LANG]["language"])
        voices = VOICES or os.environ.get("SCREENCAST_VOICES") or script.get("voices")
        voices_dir = (ROOT / Path(str(voices)).expanduser()).resolve() if voices else None
        the_voice = fishspeech.voice_of(voice, voices_dir, ROOT, settings)
        name = str(script.get("voice_name") or the_voice.name).strip()
        python = FISH_PYTHON or fishspeech.mlx_python()
        if not dry_run and (python is None or not Path(python).exists()):
            fail(
                "Fish Audio S2 Pro speaks by mlx-speech, which is not installed: "
                "uv tool install --python 3.13 mlx-speech (and its model fish-s2-pro, by "
                "mlx-speech tts --model fish-s2-pro …); or Kokoro, fast: --fast"
            )
        # Its model is in the user's own Hugging Face cache, not the runner's.
        hf_home = {"HF_HOME": USER_HF_HOME}
        return {
            "backend": "fish-s2-pro",
            "voice": voice,
            "label": f"Fish Audio S2 Pro, {name}",
            "made_by": f"Fish Audio S2 Pro, run locally with mlx-speech from its open weights "
            f"({fishspeech.LICENCE}), voice {name} (cloned from a synthetic reference clip)",
            "made_by_md": f"Fish Audio S2 Pro, run locally with mlx-speech from its open weights (licence: "
            f"{fishspeech.LICENCE}; commercial use needs a licence from Fish Audio), voice `{name}`",
            "track": f"Fish Audio S2 Pro, {name}; {fishspeech.LICENCE}",
            "narrator": lambda: fishspeech.Narrator(
                the_voice, VOICE_CACHE, fishspeech.Worker(python, hf_home), dry_run
            ),
        }
    import urllib.request

    opener = opener or urllib.request.urlopen
    settings = dict(elevenlabs.SETTINGS)
    settings["speed"] = speed
    settings.update(script.get("voice_settings") or {})
    settings.update(VOICE_SETTINGS)
    model = VOICE_MODEL or os.environ.get("ELEVENLABS_MODEL") or script.get("voice_model") or elevenlabs.MODEL
    the_voice = elevenlabs.Voice(voice, str(model), settings)
    name = str(script.get("voice_name") or "").strip()
    if not name:
        # Asked of ElevenLabs once (no characters), kept in the cache; the
        # id when there is no key (a dry run).
        name = voice if dry_run and not os.environ.get(elevenlabs.KEY_ENV) else (
            elevenlabs.voice_name(voice, VOICE_CACHE, opener)
        )
    return {
        "backend": "elevenlabs",
        "voice": voice,
        "label": f"ElevenLabs, {name}",
        "made_by": f"ElevenLabs' text-to-speech (model {model}, voice {name})",
        "made_by_md": f"ElevenLabs' text-to-speech (model `{model}`, voice {name}, id `{voice}`)",
        "track": f"ElevenLabs {model}, {name}",
        "narrator": lambda: elevenlabs.Narrator(the_voice, VOICE_CACHE, dry_run, opener),
    }


KOKORO_SPEAKER = {
    "backend": "kokoro",
    "voice": "am_michael",
    "label": "Kokoro",
    "made_by": "the Kokoro-82M text-to-speech model (voice am_michael)",
    "made_by_md": "the Kokoro-82M text-to-speech model (voice `am_michael`)",
    "track": "Kokoro-82M, am_michael",
}


def voice_intro(script: dict) -> str | None:
    """What the voice says first, over the first step's card: that it is
    an AI voice, and why. The script's `voice_intro` (its own words, in its
    own language; `{app}` the app's name), else `--voice-intro`, else the
    English default with the script's `app` or `--app-name`; none only when
    turned off (`voice_intro: false`, or for every script
    `--no-voice-intro`)."""
    said = False if VOICE_INTRO is False else script.get("voice_intro", VOICE_INTRO)
    if said is False:
        return None
    if said is None or said is True:
        said = VOICE_INTRO or text("intro", app="{app}")
    app = str(script.get("app") or APP_NAME or text("the_app")).strip()
    return spoken(str(said).replace("{app}", app)) or None


def join_wavs(parts: list[tuple[Path, float]], out: Path) -> None:
    """16-bit mono WAVs (each with the seconds of quiet after it) as one."""
    import wave

    out.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(out), "wb") as w:
        rate = None
        for wav, quiet in parts:
            with wave.open(str(wav), "rb") as r:
                if rate is None:
                    rate = r.getframerate()
                    w.setnchannels(r.getnchannels())
                    w.setsampwidth(r.getsampwidth())
                    w.setframerate(rate)
                w.writeframes(r.readframes(r.getnframes()))
            w.writeframes(b"\0\0" * round(quiet * rate))


def narrate(name: str, script: dict, voice: bool, speaker: dict | None = None, narrator=None) -> list[dict]:
    """Each step's narration: its audio file (`wav`, none without a voice),
    its `seconds` and its `words`, each with when it starts and ends. The
    first step's starts with the AI voice's own notice (`voice_intro`):
    `lead` is how many of its words are that notice's, which cues skip."""
    out = []
    if voice and narrator is None:
        narrator = speaker["narrator"]()
    for i, step in enumerate(script["steps"]):
        text = spoken(step.get("say") or step.get("text", ""))
        if not text:
            out.append({"wav": None, "seconds": 0.0, "words": [], "lead": 0})
            continue
        if not voice:
            out.append(
                {
                    "wav": None,
                    "seconds": len(elevenlabs.untagged(text).split()) / WORDS_PER_SECOND,
                    "words": guessed_words(elevenlabs.untagged(text)),
                    "lead": 0,
                }
            )
            continue
        wav, seconds, words = narrator.say(text)
        out.append({"wav": wav, "seconds": seconds, "words": words, "lead": 0})
    intro = voice_intro(script) if voice and out else None
    if intro:
        wav, seconds, words = narrator.say(intro)
        first = out[0]
        shift = seconds + INTRO_PAUSE
        joined = made_dir(name) / "first-step.wav"
        if wav is not None and first["wav"] is not None:
            join_wavs([(wav, INTRO_PAUSE), (first["wav"], 0.0)], joined)
        elif wav is not None:
            join_wavs([(wav, 0.0)], joined)
        out[0] = {
            "wav": joined if wav is not None else None,
            "seconds": (shift + first["seconds"]) if first["words"] else seconds,
            "words": words
            + [dict(w, start=round(w["start"] + shift, 3), end=round(w["end"] + shift, 3)) for w in first["words"]],
            "lead": len(words),
        }
    if voice:
        print(f"  {narrator.report()}", flush=True)
        close = getattr(narrator, "close", None)
        if close:
            close()
    return out


def normal(text: str) -> list[str]:
    """The words of `text` to compare: small letters and digits."""
    return re.findall(r"[a-z0-9]+(?:'[a-z0-9]+)*", text.lower())


def find_words(words: list[dict], phrase: str, after: int) -> tuple[int, int] | None:
    """Where `phrase` is said in `words`, from the word `after` on: the
    index of its first word and of the word after its last."""
    want = normal(spoken(phrase))
    if not want:
        return None
    # Each compared word with the index of the spoken word it is part of.
    flat = [(w, i) for i, word in enumerate(words) for w in normal(word["text"])]
    for k in range(len(flat) - len(want) + 1):
        if flat[k][1] < after:
            continue
        if [w for w, _ in flat[k : k + len(want)]] == want:
            last = flat[k + len(want) - 1][1]
            return flat[k][1], last + 1
    return None


def cue_actions(name: str, i: int, step: dict, words: list[dict], lead: int = 0) -> list:
    """The step's actions with each cue's words (`cue: "Open it"`) made the
    seconds into its narration when they are said (`{"cue": 3.2}`): the
    actions after it wait for them. Cues follow the narration's order and
    are looked for after the first `lead` words (the voice's notice)."""
    out = []
    after = lead
    for a in step.get("actions", []):
        if isinstance(a, dict) and isinstance(a.get("cue"), str):
            found = find_words(words, a["cue"], after)
            if found is None:
                fail(
                    f"{name}: step {i + 1} ({step['name']}): the cue {a['cue']!r} is not in "
                    "what it says, or not after the cue before it"
                )
            first, after = found
            out.append({"cue": words[first]["start"], "words": a["cue"]})
        else:
            if isinstance(a, dict) and a.get("shot") is True and step.get("shot", True) is False:
                fail(f"{name}: step {i + 1} ({step['name']}): shot: true in a step without a picture")
            out.append(a)
    return out


def absolute(value, base: Path, work: Path):
    """`open` paths of actions made absolute, `{work}` in a string the
    tutorial's work folder (for what it exports)."""
    if isinstance(value, dict):
        return {
            k: str((base / v).resolve()) if k == "open" and isinstance(v, str) else absolute(v, base, work)
            for k, v in value.items()
        }
    if isinstance(value, list):
        return [absolute(v, base, work) for v in value]
    if isinstance(value, str):
        return value.replace("{work}", str(work))
    return value


def voice_notice(script: dict, speaker: dict | None = None) -> str:
    """The line that says the voice is synthetic, in the script's language,
    naming the voice: "Voice: AI-generated (Kokoro)", "Voice: AI-generated
    (ElevenLabs, Rachel)". In a script's own `voice_notice`, `{voice}` is
    that name."""
    label = (speaker or KOKORO_SPEAKER)["label"]
    own = script.get("voice_notice")
    if own:
        return str(own).replace("{voice}", label).strip()
    return text("notice", voice=label)


def plan(name: str, script: dict, audio, video: bool, voice: bool, speaker: dict | None = None) -> dict:
    steps = []
    for i, step in enumerate(script["steps"]):
        actions = cue_actions(name, i, step, audio[i]["words"], audio[i].get("lead", 0))
        steps.append(
            {
                "name": step["name"],
                "actions": absolute(actions, ROOT, work_dir(name)),
                "shot": shot_name(i, step),
                "seconds": round(audio[i]["seconds"] + PAUSE, 3),
            }
        )
        if step.get("card"):
            card = dict(step["card"])
            if card.get("logo"):
                card["logo"] = str((ROOT / card["logo"]).resolve())
            steps[-1]["card"] = card
    the_plan = {"fps": FPS, "video": video, "shot_width": SHOT_WIDTH, "steps": steps}
    if voice:
        the_plan["caption"] = voice_notice(script, speaker)
    return the_plan


def play(name: str, the_plan: dict) -> dict:
    work = work_dir(name)
    config = work / "config"
    shutil.rmtree(config, ignore_errors=True)
    config.mkdir(parents=True)
    path = work / "plan.json"
    path.write_text(json.dumps(the_plan, indent=2), encoding="utf-8")
    if not APP.exists():
        fail(f"{APP} is missing: build the app first")
    record = work / "recording"
    print(f"  playing {len(the_plan['steps'])} steps in the app", flush=True)
    env = dict(os.environ, **APP_ENV)
    args = [a.replace("{work}", str(work)).replace("{lang}", LANG) for a in APP_ARGS]
    env.update({k: v.replace("{lang}", LANG) for k, v in APP_ENV.items()})
    run = subprocess.run(
        [APP, *args, "--play", path, "--record", record]
        + ([] if WINDOW else ["--headless"]),
        env=env,
        timeout=1800,
    )
    timeline = record / "timeline.json"
    if run.returncode != 0 or not timeline.exists():
        fail(f"the app ended with {run.returncode} and no {timeline}")
    result = json.loads(timeline.read_text(encoding="utf-8"))
    if result["errors"]:
        fail(f"{name}: " + "; ".join(result["errors"]))
    return result


def images_dir(script: dict) -> str:
    """The pictures' folder: `images`, or `images.de` for a script whose app
    is in that language too (`ui_languages: [en, de]`)."""
    return f"images{suffix()}" if LANG in (script.get("ui_languages") or []) and LANG != "en" else "images"


def copy_shots(name: str, the_plan: dict, script: dict | None = None) -> None:
    images = DOCS / name / images_dir(script or {})
    shutil.rmtree(images, ignore_errors=True)
    images.mkdir(parents=True)
    for step in the_plan["steps"]:
        if step["shot"]:
            shutil.copy(work_dir(name) / "recording" / "shots" / step["shot"], images / step["shot"])


def srt_time(seconds: float) -> str:
    ms = round(seconds * 1000)
    return f"{ms // 3_600_000:02d}:{ms // 60_000 % 60:02d}:{ms // 1000 % 60:02d},{ms % 1000:03d}"


def shifts(planned: list[dict], logged: dict, fps: float) -> list[tuple[float, float]]:
    """Each cue of a step as (seconds into its narration, seconds the
    words from there on are later than planned): the actions before it
    took longer than the words, and the player waited for them."""
    cues = [a["cue"] for a in planned if isinstance(a, dict) and isinstance(a.get("cue"), (int, float))]
    out = []
    late = 0.0
    for k, at in enumerate(cues):
        reached = logged.get("cues", [])
        if k < len(reached):
            late = max(late, (reached[k] - round(at * fps)) / fps)
        out.append((at, late))
    return out


def late_at(cues: list[tuple[float, float]], t: float) -> float:
    """How much later than planned what is said at `t` is."""
    late = 0.0
    for at, shift in cues:
        if t + 1e-6 >= at:
            late = shift
    return late


def energy(data, rate: int, frame: float = CUT_FRAME):
    """The short-time RMS of `data` in frames `frame` long, one centred on
    each sample (at the ends, over what there is)."""
    import numpy as np

    n = max(1, round(frame * rate))
    power = np.concatenate([[0.0], np.cumsum(np.asarray(data, dtype=np.float64) ** 2)])
    idx = np.arange(len(data))
    lo = np.clip(idx - n // 2, 0, len(data))
    hi = np.clip(idx - n // 2 + n, 0, len(data))
    return np.sqrt((power[hi] - power[lo]) / np.maximum(hi - lo, 1))


def quiet_cut(data, rate: int, at: float, floor: int = 0) -> int:
    """The sample to cut `data` at for a cue whose word starts at `at`
    seconds, by the aligner: the quietest point from `CUT_BEFORE` before to
    `CUT_AFTER` after it, not before `floor`. Of the quiet run there, its
    start (a frame into it): the pause before the word, not the word's own
    first sound."""
    import numpy as np

    end = len(data)
    lo = min(end, max(floor, round((at - CUT_BEFORE) * rate)))
    hi = min(end, max(lo, round((at + CUT_AFTER) * rate)))
    if hi - lo < 2:
        return min(end, max(floor, round(at * rate)))
    rms = energy(data[lo:hi], rate)
    least = float(rms.min())
    # Quiet: within a little of the least (6 dB, or -66 dBFS of noise).
    quiet = rms <= max(2 * least, least + 5e-4)
    k = int(np.argmin(rms))
    start = k
    while start > 0 and quiet[start - 1]:
        start -= 1
    stop = k
    while stop + 1 < len(quiet) and quiet[stop + 1]:
        stop += 1
    # A frame into the run, or its middle when it is shorter.
    into = min(round(CUT_FRAME * rate), (stop - start) // 2)
    return lo + start + into


def cut_points(data, rate: int, cues: list[tuple[float, float]]) -> list[int]:
    """Where a step's narration is cut for its cues: at the quiet before
    each cue's word, in order, none before the one before it."""
    points = []
    floor = 0
    for at, _ in cues:
        cut = quiet_cut(data, rate, at, floor)
        points.append(cut)
        floor = cut
    return points


def faded(piece, rate: int, fade: float = FADE):
    """`piece` faded in at its start and out at its end, so a cut never
    clicks."""
    import numpy as np

    piece = np.array(piece, dtype=np.float32)
    n = min(round(fade * rate), len(piece) // 2)
    if n > 0:
        ramp = np.linspace(0.0, 1.0, n, endpoint=False, dtype=np.float32)
        piece[:n] *= ramp
        piece[len(piece) - n :] *= ramp[::-1]
    return piece


def place_narration(track, data, rate: int, start: float, cues: list[tuple[float, float]]) -> list[float]:
    """A step's narration `data` added into `track` from `start` seconds,
    cut at the quiet before each cue's word and each piece from there on as
    much later as its cue came late, faded at its edges. The seconds into
    the narration where each cut is, for the subtitles."""
    points = cut_points(data, rate, cues)
    # Cut only where the words wait longer than before.
    edges, lates = [0], [0.0]
    for point, (_, late) in zip(points, cues):
        if late != lates[-1]:
            edges.append(point)
            lates.append(late)
    edges.append(len(data))
    for k, late in enumerate(lates):
        piece = faded(data[edges[k] : edges[k + 1]], rate)
        at = round((start + late) * rate) + edges[k]
        piece = piece[: max(0, len(track) - at)]
        # Added: the pieces of a step never overlap (each starts no earlier
        # than the one before it ends), a step's may run into the next one.
        track[at : at + len(piece)] += piece
    return [p / rate for p in points]


def subtitles_of(words: list[dict], cues: list[tuple[float, float]]) -> list[tuple[float, float, str]]:
    """A step's subtitles from its words, as (start, end, text) in seconds
    from the step's start: what is said, when it is said. One ends at a
    sentence's end, where a cue moves the words, or at a comma once long."""
    out = []
    group: list[dict] = []

    def close():
        if not group:
            return
        text = "".join(w["text"] + (" " if w["space"] else "") for w in group).strip()
        if text:
            first, last = group[0], group[-1]
            out.append(
                (
                    first["start"] + late_at(cues, first["start"]),
                    last["end"] + late_at(cues, last["start"]),
                    text,
                )
            )
        group.clear()

    for i, w in enumerate(words):
        if group and late_at(cues, w["start"]) != late_at(cues, group[0]["start"]):
            close()
        group.append(w)
        length = sum(len(x["text"]) + 1 for x in group)
        end = w["text"] in (".", "!", "?") or (length >= SUBTITLE_CHARS and w["text"] in (",", ";", ":"))
        if end or i + 1 == len(words):
            close()
    # Each shown until the next comes.
    return [
        (s, min(e, out[k + 1][0]) if k + 1 < len(out) else e, t) for k, (s, e, t) in enumerate(out)
    ]


def video(name: str, script: dict, audio, the_plan: dict, timeline: dict, speaker: dict | None = None) -> Path:
    if not shutil.which("ffmpeg"):
        fail("ffmpeg is missing (brew install ffmpeg), or build without video: --no-video")
    work = work_dir(name)
    fps = timeline["fps"]
    # The narration: each step's audio from the step's first frame, its
    # words after a cue as much later as the cue came late; quiet to its
    # last frame.
    import numpy as np
    import soundfile as sf

    rate = 24_000
    track = np.zeros(round(timeline["frames"] / fps * rate), dtype=np.float32)
    srt = []
    for i, (planned, logged) in enumerate(zip(the_plan["steps"], timeline["steps"])):
        start = logged["start"] / fps
        cues = shifts(planned["actions"], logged, fps)
        wav = audio[i]["wav"]
        if wav is not None:
            data, r = sf.read(wav, dtype="float32")
            assert r == rate, f"{wav}: {r} Hz"
            if data.ndim > 1:
                data = data.mean(axis=1)
            at_cuts = place_narration(track, data, rate, start, cues)
            # The words from each cut on move with their sound.
            cues = [(min(at, cut), late) for (at, late), cut in zip(cues, at_cuts)]
        for s, e, said in subtitles_of(audio[i]["words"], cues):
            srt.append(f"{len(srt) + 1}\n{srt_time(start + s)} --> {srt_time(start + e)}\n{said}\n")
    made = made_dir(name)
    made.mkdir(parents=True, exist_ok=True)
    narration = made / "narration.wav"
    sf.write(narration, np.clip(track, -1.0, 1.0), rate, subtype="PCM_16")
    subtitles = made / "subtitles.srt"
    subtitles.write_text("\n".join(srt), encoding="utf-8")
    out = (made if OUT else DOCS / name) / f"{name}{suffix()}.mp4"
    voiced = "caption" in the_plan
    speaker = speaker or KOKORO_SPEAKER
    notice = voice_notice(script, speaker)
    # That the voice is synthetic, readable by machines too (Art. 50(2)):
    # the file's comment and description, and the sound's own title.
    disclosure = (
        [
            "-metadata", f"comment={notice}. The narration is synthetic speech "
            f"made by {speaker['made_by']}.",
            "-metadata", f"description={' '.join(plain(script.get('summary', '')).split())} "
            f"[{notice}]",
            "-metadata:s:a:0", f"title=AI-generated voice ({speaker['track']})",
            "-metadata:s:a:0", f"handler_name=AI-generated voice ({speaker['track']})",
        ]
        if voiced
        else []
    )
    cmd = [
        "ffmpeg", "-y", "-loglevel", "error",
        "-f", "concat", "-safe", "0", "-i", work / "recording" / "frames.txt",
        "-i", narration,
        "-i", subtitles,
        "-map", "0:v", "-map", "1:a", "-map", "2:s",
        "-vf", f"fps={fps},scale={VIDEO_WIDTH}:-2:flags=lanczos,format=yuv420p",
        "-c:v", "libx264", "-preset", "slow", "-crf", "22", "-tune", "stillimage",
        "-c:a", "aac", "-b:a", "128k",
        "-c:s", "mov_text", "-metadata:s:s:0", f"language={text('subtitles')}",
        "-metadata:s:a:0", f"language={text('subtitles')}",
        "-metadata", f"title={script['title']}",
        *disclosure,
        "-movflags", "+faststart",
        "-t", f"{timeline['frames'] / fps:.3f}",
        out,
    ]
    print(f"  encoding {shown(out)}", flush=True)
    subprocess.run([str(c) for c in cmd], check=True)
    return out


def readme(name: str, script: dict, the_plan: dict, speaker: dict | None = None) -> None:
    speaker = speaker or KOKORO_SPEAKER
    lines = [
        f"# {script['title']}",
        "",
        "<!-- Generated by egui-screencast's tutorial.py from script.yaml; edit that. -->",
        "",
        script.get("summary", "").strip(),
        "",
    ]
    if script.get("needs"):
        lines += [f"**{text('needs')}:** {script['needs'].strip()}", ""]
    lines += [
        text("screencast", video=f"{name}{suffix()}.mp4", name=name),
        "",
        f"**{voice_notice(script, speaker)}.** " + text("page_notice", made_by=speaker["made_by_md"]),
        "",
    ]
    for i, (step, planned) in enumerate(zip(script["steps"], the_plan["steps"])):
        lines += [f"## {i + 1}. {step['title']}", "", step.get("text", "").strip(), ""]
        if step.get("ui"):
            lines += [
                f"> {text('in_app')}: {step['ui'].strip()}",
                "",
            ]
        if step.get("pending_ui"):
            lines += [
                f"> **{text('pending')}** {step['pending_ui'].strip()}",
                "",
            ]
        if planned["shot"]:
            lines += [f"![{step['title']}]({images_dir(script)}/{planned['shot']})", ""]
    if script.get("next"):
        lines += [f"## {text('next')}", "", script["next"].strip(), ""]
    (DOCS / name / f"README{suffix()}.md").write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")


class CachedOnly:
    """A narrator that only gives what is in the cache (`--remix`): a
    section not there stops the run, nothing is spoken."""

    def __init__(self, narrator, name: str):
        self.narrator = narrator
        self.name = name
        self.sections = 0

    def say(self, text: str):
        wav, seconds, words = self.narrator.say(text)
        if wav is None or not Path(wav).exists():
            fail(f"{self.name}: --remix: a section is not in the cache ({text[:60]!r}…); build it without --remix")
        self.sections += 1
        return wav, seconds, words

    def report(self) -> str:
        return f"{self.sections} sections from the cache"


def remix(name: str, script: dict, voice: bool, speaker: dict | None) -> None:
    """The narration, subtitles and video made again from the recording
    of an earlier run and the cached sections: the app is not started,
    nothing is spoken."""
    work = work_dir(name)
    record = work / "recording"
    needed = [work / "plan.json", record / "timeline.json", record / "frames.txt", record / "frames"]
    missing = [shown(p) for p in needed if not p.exists()]
    if missing:
        fail(f"{name}: --remix needs a recording: no {', '.join(map(str, missing))} (build it without --remix)")
    the_plan = json.loads((work / "plan.json").read_text(encoding="utf-8"))
    timeline = json.loads((record / "timeline.json").read_text(encoding="utf-8"))
    if [s["name"] for s in the_plan["steps"]] != [s["name"] for s in script["steps"]]:
        fail(f"{name}: --remix: the script's steps are not the recording's; build it without --remix")
    if len(timeline["steps"]) != len(the_plan["steps"]):
        fail(f"{name}: --remix: {record / 'timeline.json'} has not every step of the plan")
    narrator = CachedOnly(speaker["narrator"](), name) if voice else None
    audio = narrate(name, script, voice, speaker, narrator)
    out = video(name, script, audio, the_plan, timeline, speaker)
    print(f"  {shown(out)}: {timeline['frames'] / timeline['fps']:.1f} s", flush=True)


def build(name: str, voice: bool, with_video: bool, dry_run: bool = False, lang: str = "en") -> None:
    global LANG
    LANG = lang
    print(f"{name} ({lang}):", flush=True)
    original = load(name)
    missing = missing_text(original, lang)
    if missing:
        fail(f"{name}: no {lang} text in steps {', '.join(map(str, missing))} (say: {{en: …, {lang}: …}} or say_{lang}:)")
    script = localized(original, lang)
    if REMIX:
        try:
            remix(name, script, voice, speaker_of(script, dry_run=True) if voice else None)
        except elevenlabs.SpeechError as e:
            fail(f"{name}: {e}")
        return
    try:
        speaker = speaker_of(script, dry_run) if voice else None
        audio = narrate(name, script, voice, speaker)
    except elevenlabs.SpeechError as e:
        fail(f"{name}: {e}")
    if dry_run:
        return
    the_plan = plan(name, script, audio, with_video, voice, speaker)
    timeline = play(name, the_plan)
    copy_shots(name, the_plan, script)
    readme(name, script, the_plan, speaker)
    if with_video:
        out = video(name, script, audio, the_plan, timeline, speaker)
        seconds = timeline["frames"] / timeline["fps"]
        print(f"  {shown(out)}: {seconds:.1f} s", flush=True)
    page = DOCS / name / f"README{suffix()}.md"
    print(f"  {shown(page)}, {sum(1 for s in the_plan['steps'] if s['shot'])} pictures")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("names", nargs="*", help="tutorials (folders of --tutorials); all without")
    p.add_argument("--app", help="the app's executable (not needed for --dry-run, --prune-voice-cache)")
    p.add_argument("--app-arg", action="append", default=[], help="an argument for the app, before --play")
    p.add_argument("--env", action="append", default=[], help="K=V in the app's environment")
    p.add_argument("--tutorials", default="docs/tutorials", help="the folder of the tutorials")
    p.add_argument("--work", default="target/tutorials", help="the folder of work files")
    p.add_argument("--no-voice", action="store_true", help="no narration: a silent video, timed by word count")
    p.add_argument("--no-video", action="store_true", help="pictures and page only")
    p.add_argument(
        "--window",
        action="store_true",
        help="play in a window instead of without one (1920 × 1080 on the GPU)",
    )
    p.add_argument(
        "--voice",
        help="the voice, over the script's and $SCREENCAST_VOICE: a Kokoro voice (am_michael, "
        "kokoro:af_heart), elevenlabs:<voice id> (needs $ELEVENLABS_API_KEY) or "
        "fish-s2-pro:<voice of voices.yaml, or a WAV> (local, non-commercial licence)",
    )
    p.add_argument("--voice-model", help="ElevenLabs' model (else $ELEVENLABS_MODEL, the script's voice_model, "
                   f"{'eleven_multilingual_v2'})")
    p.add_argument("--voice-setting", action="append", default=[], metavar="K=V",
                   help="a voice setting of ElevenLabs (stability=0.4, speed=1.05, …) or Fish (seed=7, …)")
    p.add_argument("--voice-cache", help="the cache of ElevenLabs' and Fish S2 Pro's sections (else "
                   "$SCREENCAST_VOICE_CACHE, <work>/voices)")
    p.add_argument("--fish-python", help="the Python of mlx-speech (else $FISH_SPEECH_PYTHON, else the "
                   "mlx-speech command's)")
    p.add_argument("--voices", help="Fish S2 Pro's voices: a folder with a voices.yaml (else $SCREENCAST_VOICES, "
                   "the script's voices:)")
    p.add_argument("--engine", help="the engine of a bare voice: kokoro, elevenlabs, fish-s2-pro (else "
                   "$SCREENCAST_ENGINE, the script's engine:)")
    p.add_argument("--dry-run", action="store_true",
                   help="only count the sections not in the cache and the characters they would send")
    p.add_argument("--prune-voice-cache", type=float, metavar="DAYS",
                   help="remove cached sections (ElevenLabs, Fish) not used for DAYS days, then end")
    p.add_argument("--app-name", help="the app's name in the voice's own notice (a script's app: wins)")
    p.add_argument("--voice-intro", help="what the voice says first (a script's voice_intro: wins; {app} its name)")
    p.add_argument("--no-voice-intro", action="store_true", help="the voice does not say first that it is AI")
    p.add_argument("--lang", default="en", help="the language(s) of the narration and the page: en, de or en,de "
                   "(German: <name>.de.mp4, README.de.md)")
    p.add_argument("--fast", action="store_true", help="Kokoro instead of Fish Audio S2 Pro: fast, English only")
    p.add_argument("--remix", action="store_true",
                   help="the narration, subtitles and video again from <work>/<name>/recording and the cached "
                   "sections: no app, nothing spoken")
    p.add_argument("--out", help="the video, its narration and subtitles in OUT/<name>[.<lang>]/ (else the video "
                   "beside the script)")
    a = p.parse_args()
    global WINDOW, DOCS, WORK, APP, APP_ARGS, APP_ENV, VOICE, VOICE_MODEL, VOICE_SETTINGS, VOICE_CACHE
    global APP_NAME, VOICE_INTRO, FISH_PYTHON, VOICES, ENGINE, FAST, REMIX, OUT
    WINDOW = a.window
    DOCS = (ROOT / a.tutorials).resolve()
    WORK = (ROOT / a.work).resolve()
    APP_ARGS = a.app_arg
    APP_ENV = dict(e.split("=", 1) for e in a.env)
    VOICE = a.voice
    VOICE_MODEL = a.voice_model
    for kv in a.voice_setting:
        k, sep, v = kv.partition("=")
        if not sep:
            fail(f"--voice-setting {kv!r}: K=V")
        VOICE_SETTINGS[k.strip()] = elevenlabs.setting(v)
    cache = a.voice_cache or os.environ.get("SCREENCAST_VOICE_CACHE")
    VOICE_CACHE = (ROOT / cache).resolve() if cache else WORK / "voices"
    fish = a.fish_python or os.environ.get("FISH_SPEECH_PYTHON")
    FISH_PYTHON = (ROOT / fish).resolve() if fish else None
    VOICES = a.voices
    ENGINE = a.engine
    APP_NAME = a.app_name
    VOICE_INTRO = False if a.no_voice_intro else (a.voice_intro or None)
    FAST = a.fast
    REMIX = a.remix
    OUT = (ROOT / a.out).resolve() if a.out else None
    if REMIX and (a.dry_run or a.no_video):
        fail("--remix makes the video: not with --dry-run or --no-video")
    langs = [x.strip() for x in a.lang.split(",") if x.strip()]
    for lang in langs:
        if lang not in LANGS:
            fail(f"--lang {lang}: one of {', '.join(LANGS)}")
    if a.prune_voice_cache is not None:
        removed, size = elevenlabs.Cache(VOICE_CACHE).prune(a.prune_voice_cache)
        print(f"{VOICE_CACHE}: {removed} entries removed, {size / 1e6:.1f} MB")
        return
    if not a.dry_run and not REMIX:
        if not a.app:
            fail("--app names the app's executable")
        APP = (ROOT / a.app).resolve()
    os.environ.setdefault("HF_HOME", str(WORK / "hf"))
    names = a.names or sorted(d.name for d in DOCS.iterdir() if (d / "script.yaml").exists())
    if not names:
        fail(f"no tutorials in {DOCS}")
    for lang in langs:
        for name in names:
            build(name, voice=not a.no_voice, with_video=not a.no_video, dry_run=a.dry_run, lang=lang)


if __name__ == "__main__":
    main()
