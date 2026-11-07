#!/usr/bin/env python3
"""Builds an egui app's tutorials from their scripts, with egui-screencast.

A tutorial is `<tutorials>/<name>/script.yaml`: a title, a summary and
steps, each with its text, what it does in the app and its picture. This
script, for each tutorial named:

1. speaks each step's text with Kokoro (`narrate.py`) or ElevenLabs
   (`elevenlabs.py`, `--voice elevenlabs:<voice id>`) or Fish Audio S2 Pro
   run locally (`fishspeech.py`, `--voice fish-s2-pro:<reference.wav>`),
   each section kept in
   a cache and spoken again only when it changed, or, with `--no-voice`,
   guesses how long it would take; first, over the first step's card, the
   voice says that it is an AI voice and why (`voice_intro`);
2. writes the plan for the app: the steps' actions, pictures and seconds;
3. starts the app (`--app`, with its `--app-arg`s and `--env`s) with
   `--play plan.json --record dir --headless`: the app plays the steps
   without a window, drawn at 1920 × 1080 on the GPU, and takes its pictures
   itself (its `egui_screencast::Player`);
4. copies the pictures to `<tutorials>/<name>/images/` and writes
   `<tutorials>/<name>/README.md`;
5. unless `--no-video`, makes `<tutorials>/<name>/<name>.mp4` with ffmpeg:
   the frames, the narration under them and the text as subtitles.

    python tutorial.py --app target/release/app [--app-arg ARG]… [--env K=V]…
        [--tutorials docs/tutorials] [--work target/tutorials]
        [--no-voice] [--no-video] [--window] [NAME…]
        [--voice elevenlabs:<voice id>] [--voice-model M] [--voice-setting K=V]…
        [--voice-cache DIR] [--dry-run] [--prune-voice-cache DAYS]
        [--app-name NAME] [--voice-intro TEXT | --no-voice-intro]

What is said and what is seen go together: an action `cue: "Open it"`
waits until the narration says those words (Kokoro's times of its words),
and should the actions before it take longer, the words from there on wait
instead. `shot: true` takes the step's picture there rather than at the
step's end. The subtitles are the words spoken, when they are spoken. The
voice is said to be synthetic on the cards, in a corner of the video, on
the page and in the video's metadata (`voice_notice` in another language).

In an `--app-arg`, `{work}` is the tutorial's work folder (an app's own
settings folder, say). Paths are taken from the folder it is run in. Without
names, every tutorial is built. Work files are kept in `<work>/<name>/`.
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
# The voice (`--voice`, else $SCREENCAST_VOICE, else the script's), the
# ElevenLabs model and settings over the script's, and where ElevenLabs'
# sections are kept: set from the command line in `main`.
KOKORO_VOICE = "am_michael"
# The names of Fish Audio S2 Pro, run locally (`fishspeech.py`).
FISH_NAMES = ("fish-s2-pro", "fishaudio-s2-pro", "fishaudio")
# The Python of the Fish venv (`--fish-python`, $FISH_SPEECH_PYTHON, else
# <work>/fish-venv/bin/python).
FISH_PYTHON: Path | None = None
VOICE: str | None = None
VOICE_MODEL: str | None = None
VOICE_SETTINGS: dict = {}
VOICE_CACHE = WORK / "voices"
# What the voice says first, over the first step's card: that it is an AI
# voice, and why (a script's `voice_intro`, `--voice-intro`; off only by
# `voice_intro: false` or `--no-voice-intro`). `{app}` is the app's name.
VOICE_INTRO_DEFAULT = "This video is narrated by an AI voice, so it can always show the current version of {app}."
VOICE_INTRO: str | bool = VOICE_INTRO_DEFAULT
APP_NAME: str | None = None
# Seconds of quiet between that line and the first step's words.
INTRO_PAUSE = 0.6
# A subtitle ends at a sentence's end, at a cue, or at a comma once it is
# this long.
SUBTITLE_CHARS = 80

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


def plain(markdown: str) -> str:
    """Markdown as text: links, emphasis and code marks gone."""
    t = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", markdown)
    t = re.sub(r"\[([^\]]+)\]\([^)]*\)", r"\1", t)
    t = re.sub(r"[*_`]+", "", t)
    t = re.sub(r"^\s*[-#>]+\s*", "", t, flags=re.M)
    return " ".join(t.split())


def spoken(text: str) -> str:
    """Text as Kokoro should read it: units and sizes spelled out."""
    t = plain(text)
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
    """The backend and the voice: `--voice`, else `$SCREENCAST_VOICE`, else
    the script's `voice`, else Kokoro's am_michael. `elevenlabs:<voice id>`
    is ElevenLabs, `fish-s2-pro:<reference.wav>` Fish Audio S2 Pro (run
    locally), `kokoro:<voice>` or a bare name is Kokoro."""
    choice = str(VOICE or os.environ.get("SCREENCAST_VOICE") or script.get("voice") or KOKORO_VOICE).strip()
    backend, sep, rest = choice.partition(":")
    backend = backend.lower()
    if sep and backend in ("elevenlabs", "kokoro", *FISH_NAMES):
        if not rest.strip():
            fail(f"voice {choice!r}: no voice after {backend}:")
        return ("fish-s2-pro" if backend in FISH_NAMES else backend), rest.strip()
    return "kokoro", choice


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
        the_voice = fishspeech.Voice((ROOT / voice).resolve(), str(script.get("voice_reference_text") or ""), settings)
        name = str(script.get("voice_name") or the_voice.name).strip()
        python = FISH_PYTHON or WORK / "fish-venv" / "bin" / "python"
        hf_home = os.environ.get("HF_HOME", str(WORK / "hf"))
        return {
            "backend": "fish-s2-pro",
            "voice": voice,
            "label": f"Fish Audio S2 Pro, {name}",
            "made_by": f"Fish Audio S2 Pro, run locally from its open weights ({fishspeech.LICENCE}), "
            f"in the voice of the reference recording {name}",
            "made_by_md": f"Fish Audio S2 Pro, run locally from its open weights (licence: {fishspeech.LICENCE}; "
            f"commercial use needs a licence from Fish Audio), in the voice of the reference recording {name}",
            "track": f"Fish Audio S2 Pro, {name}; {fishspeech.LICENCE}",
            "narrator": lambda: fishspeech.Narrator(
                the_voice, VOICE_CACHE, fishspeech.Worker(python, {"HF_HOME": hf_home}), dry_run
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
    if said is False or said is None:
        return None
    if said is True:
        said = VOICE_INTRO or VOICE_INTRO_DEFAULT
    app = str(script.get("app") or APP_NAME or "the app").strip()
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
                    "seconds": len(text.split()) / WORDS_PER_SECOND,
                    "words": guessed_words(text),
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
        joined = WORK / name / "first-step.wav"
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
    return VOICE_NOTICE.replace("Kokoro", label)


def plan(name: str, script: dict, audio, video: bool, voice: bool, speaker: dict | None = None) -> dict:
    steps = []
    for i, step in enumerate(script["steps"]):
        actions = cue_actions(name, i, step, audio[i]["words"], audio[i].get("lead", 0))
        steps.append(
            {
                "name": step["name"],
                "actions": absolute(actions, ROOT, WORK / name),
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
    work = WORK / name
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
    args = [a.replace("{work}", str(work)) for a in APP_ARGS]
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


def copy_shots(name: str, the_plan: dict) -> None:
    images = DOCS / name / "images"
    shutil.rmtree(images, ignore_errors=True)
    images.mkdir(parents=True)
    for step in the_plan["steps"]:
        if step["shot"]:
            shutil.copy(WORK / name / "recording" / "shots" / step["shot"], images / step["shot"])


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
    work = WORK / name
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
            cuts = [0] + [min(len(data), round(at * rate)) for at, _ in cues] + [len(data)]
            lates = [0.0] + [late for _, late in cues]
            for k, late in enumerate(lates):
                piece = data[cuts[k] : cuts[k + 1]]
                at = round((start + late) * rate) + cuts[k]
                piece = piece[: max(0, len(track) - at)]
                track[at : at + len(piece)] = piece
        for s, e, text in subtitles_of(audio[i]["words"], cues):
            srt.append(f"{len(srt) + 1}\n{srt_time(start + s)} --> {srt_time(start + e)}\n{text}\n")
    narration = work / "narration.wav"
    sf.write(narration, track, rate, subtype="PCM_16")
    subtitles = work / "subtitles.srt"
    subtitles.write_text("\n".join(srt), encoding="utf-8")
    out = DOCS / name / f"{name}.mp4"
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
        "-c:s", "mov_text", "-metadata:s:s:0", "language=eng",
        "-metadata", f"title={script['title']}",
        *disclosure,
        "-movflags", "+faststart",
        "-t", f"{timeline['frames'] / fps:.3f}",
        out,
    ]
    print(f"  encoding {out.relative_to(ROOT)}", flush=True)
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
        lines += [f"**You need:** {script['needs'].strip()}", ""]
    lines += [
        f"**Screencast:** `{name}.mp4` beside this page, with narration, is built by "
        f"`make tutorial NAME={name}` (not in the repository, see [the index](../README.md)).",
        "",
        f"**{voice_notice(script, speaker)}.** The screencast's narration is synthetic speech, made by "
        f"{speaker['made_by_md']} "
        "from the text of this page; no person speaks it. The video says so on its first "
        "card and in a corner throughout, and in its metadata.",
        "",
    ]
    for i, (step, planned) in enumerate(zip(script["steps"], the_plan["steps"])):
        lines += [f"## {i + 1}. {step['title']}", "", step.get("text", "").strip(), ""]
        if step.get("ui"):
            lines += [
                f"> In the app: {step['ui'].strip()}",
                "",
            ]
        if step.get("pending_ui"):
            lines += [
                f"> **For now through the assistant's tools.** {step['pending_ui'].strip()}",
                "",
            ]
        if planned["shot"]:
            lines += [f"![{step['title']}](images/{planned['shot']})", ""]
    if script.get("next"):
        lines += ["## Next", "", script["next"].strip(), ""]
    (DOCS / name / "README.md").write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")


def build(name: str, voice: bool, with_video: bool, dry_run: bool = False) -> None:
    print(f"{name}:", flush=True)
    script = load(name)
    try:
        speaker = speaker_of(script, dry_run) if voice else None
        audio = narrate(name, script, voice, speaker)
    except elevenlabs.SpeechError as e:
        fail(f"{name}: {e}")
    if dry_run:
        return
    the_plan = plan(name, script, audio, with_video, voice, speaker)
    timeline = play(name, the_plan)
    copy_shots(name, the_plan)
    readme(name, script, the_plan, speaker)
    if with_video:
        out = video(name, script, audio, the_plan, timeline, speaker)
        seconds = timeline["frames"] / timeline["fps"]
        print(f"  {out.relative_to(ROOT)}: {seconds:.1f} s", flush=True)
    print(f"  {(DOCS / name / 'README.md').relative_to(ROOT)}, {sum(1 for s in the_plan['steps'] if s['shot'])} pictures")


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
        "fish-s2-pro:<reference.wav> (local, non-commercial licence)",
    )
    p.add_argument("--voice-model", help="ElevenLabs' model (else $ELEVENLABS_MODEL, the script's voice_model, "
                   f"{'eleven_multilingual_v2'})")
    p.add_argument("--voice-setting", action="append", default=[], metavar="K=V",
                   help="a voice setting of ElevenLabs (stability=0.4, speed=1.05, …) or Fish (seed=7, …)")
    p.add_argument("--voice-cache", help="the cache of ElevenLabs' and Fish S2 Pro's sections (else "
                   "$SCREENCAST_VOICE_CACHE, <work>/voices)")
    p.add_argument("--fish-python", help="the Python of the Fish venv (else $FISH_SPEECH_PYTHON, "
                   "<work>/fish-venv/bin/python)")
    p.add_argument("--dry-run", action="store_true",
                   help="only count the sections not in the cache and the characters they would send")
    p.add_argument("--prune-voice-cache", type=float, metavar="DAYS",
                   help="remove cached sections (ElevenLabs, Fish) not used for DAYS days, then end")
    p.add_argument("--app-name", help="the app's name in the voice's own notice (a script's app: wins)")
    p.add_argument("--voice-intro", help="what the voice says first (a script's voice_intro: wins; {app} its name)")
    p.add_argument("--no-voice-intro", action="store_true", help="the voice does not say first that it is AI")
    a = p.parse_args()
    global WINDOW, DOCS, WORK, APP, APP_ARGS, APP_ENV, VOICE, VOICE_MODEL, VOICE_SETTINGS, VOICE_CACHE
    global APP_NAME, VOICE_INTRO, FISH_PYTHON
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
    APP_NAME = a.app_name
    VOICE_INTRO = False if a.no_voice_intro else (a.voice_intro or VOICE_INTRO_DEFAULT)
    if a.prune_voice_cache is not None:
        removed, size = elevenlabs.Cache(VOICE_CACHE).prune(a.prune_voice_cache)
        print(f"{VOICE_CACHE}: {removed} entries removed, {size / 1e6:.1f} MB")
        return
    if not a.dry_run:
        if not a.app:
            fail("--app names the app's executable")
        APP = (ROOT / a.app).resolve()
    os.environ.setdefault("HF_HOME", str(WORK / "hf"))
    names = a.names or sorted(d.name for d in DOCS.iterdir() if (d / "script.yaml").exists())
    if not names:
        fail(f"no tutorials in {DOCS}")
    for name in names:
        build(name, voice=not a.no_voice, with_video=not a.no_video, dry_run=a.dry_run)


if __name__ == "__main__":
    main()
