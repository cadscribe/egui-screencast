#!/usr/bin/env python3
"""Builds an egui app's tutorials from their scripts, with egui-screencast.

A tutorial is `<tutorials>/<name>/script.yaml`: a title, a summary and
steps, each with its text, what it does in the app and its picture. This
script, for each tutorial named:

1. speaks each step's text with Kokoro (`narrate.py`), or, with
   `--no-voice`, guesses how long it would take;
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


def narrate(name: str, script: dict, voice: bool) -> list[dict]:
    """Each step's narration: its audio file (`wav`, none without a voice),
    its `seconds` and its `words`, each with when it starts and ends."""
    out = []
    speaker = script.get("voice", "am_michael")
    speed = float(script.get("speed", 1.0))
    if voice:
        sys.path.insert(0, str(Path(__file__).parent))
        import narrate as kokoro
    for i, step in enumerate(script["steps"]):
        text = spoken(step.get("say") or step.get("text", ""))
        if not text:
            out.append({"wav": None, "seconds": 0.0, "words": []})
            continue
        if not voice:
            out.append(
                {"wav": None, "seconds": len(text.split()) / WORDS_PER_SECOND, "words": guessed_words(text)}
            )
            continue
        wav = WORK / "audio" / f"{kokoro.key(text, speaker, speed)}.wav"
        words = kokoro.words(wav) if wav.exists() else None
        if words is not None:
            seconds = kokoro.duration(wav)
        else:
            # Spoken again when the words of an older run were not kept.
            print(f"  narration {i + 1}: {text[:60]}…", flush=True)
            seconds = kokoro.speak(text, wav, speaker, speed)
            words = kokoro.words(wav) or []
        out.append({"wav": wav, "seconds": seconds, "words": words})
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


def cue_actions(name: str, i: int, step: dict, words: list[dict]) -> list:
    """The step's actions with each cue's words (`cue: "Open it"`) made the
    seconds into its narration when they are said (`{"cue": 3.2}`): the
    actions after it wait for them. Cues follow the narration's order."""
    out = []
    after = 0
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


def voice_notice(script: dict) -> str:
    """The line that says the voice is synthetic, in the script's language."""
    return str(script.get("voice_notice") or VOICE_NOTICE).strip()


def plan(name: str, script: dict, audio, video: bool, voice: bool) -> dict:
    steps = []
    for i, step in enumerate(script["steps"]):
        actions = cue_actions(name, i, step, audio[i]["words"])
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
        the_plan["caption"] = voice_notice(script)
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


def video(name: str, script: dict, audio, the_plan: dict, timeline: dict) -> Path:
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
    speaker = script.get("voice", "am_michael")
    # That the voice is synthetic, readable by machines too (Art. 50(2)):
    # the file's comment and description, and the sound's own title.
    disclosure = (
        [
            "-metadata", f"comment={voice_notice(script)}. The narration is synthetic speech "
            f"made by the Kokoro-82M text-to-speech model (voice {speaker}).",
            "-metadata", f"description={script.get('summary', '').strip()} "
            f"[{voice_notice(script)}]",
            "-metadata:s:a:0", f"title=AI-generated voice (Kokoro-82M, {speaker})",
            "-metadata:s:a:0", f"handler_name=AI-generated voice (Kokoro-82M, {speaker})",
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


def readme(name: str, script: dict, the_plan: dict) -> None:
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
        f"**{voice_notice(script)}.** The screencast's narration is synthetic speech, made by "
        f"the Kokoro-82M text-to-speech model (voice `{script.get('voice', 'am_michael')}`) "
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


def build(name: str, voice: bool, with_video: bool) -> None:
    print(f"{name}:", flush=True)
    script = load(name)
    audio = narrate(name, script, voice)
    the_plan = plan(name, script, audio, with_video, voice)
    timeline = play(name, the_plan)
    copy_shots(name, the_plan)
    readme(name, script, the_plan)
    if with_video:
        out = video(name, script, audio, the_plan, timeline)
        seconds = timeline["frames"] / timeline["fps"]
        print(f"  {out.relative_to(ROOT)}: {seconds:.1f} s", flush=True)
    print(f"  {(DOCS / name / 'README.md').relative_to(ROOT)}, {sum(1 for s in the_plan['steps'] if s['shot'])} pictures")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("names", nargs="*", help="tutorials (folders of --tutorials); all without")
    p.add_argument("--app", required=True, help="the app's executable")
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
    a = p.parse_args()
    global WINDOW, DOCS, WORK, APP, APP_ARGS, APP_ENV
    WINDOW = a.window
    DOCS = (ROOT / a.tutorials).resolve()
    WORK = (ROOT / a.work).resolve()
    APP = (ROOT / a.app).resolve()
    APP_ARGS = a.app_arg
    APP_ENV = dict(e.split("=", 1) for e in a.env)
    os.environ.setdefault("HF_HOME", str(WORK / "hf"))
    names = a.names or sorted(d.name for d in DOCS.iterdir() if (d / "script.yaml").exists())
    if not names:
        fail(f"no tutorials in {DOCS}")
    for name in names:
        build(name, voice=not a.no_voice, with_video=not a.no_video)


if __name__ == "__main__":
    main()
