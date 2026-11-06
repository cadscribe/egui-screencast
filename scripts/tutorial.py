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


def narrate(name: str, script: dict, voice: bool) -> list[tuple[Path | None, float]]:
    """Each step's audio file and its seconds."""
    out = []
    speaker = script.get("voice", "am_michael")
    speed = float(script.get("speed", 1.0))
    if voice:
        sys.path.insert(0, str(Path(__file__).parent))
        import narrate as kokoro
    for i, step in enumerate(script["steps"]):
        text = spoken(step.get("say") or step.get("text", ""))
        if not text:
            out.append((None, 0.0))
            continue
        if not voice:
            out.append((None, len(text.split()) / WORDS_PER_SECOND))
            continue
        wav = WORK / "audio" / f"{kokoro.key(text, speaker, speed)}.wav"
        if wav.exists():
            seconds = kokoro.duration(wav)
        else:
            print(f"  narration {i + 1}: {text[:60]}…", flush=True)
            seconds = kokoro.speak(text, wav, speaker, speed)
        out.append((wav, seconds))
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


def plan(name: str, script: dict, audio, video: bool) -> dict:
    steps = []
    for i, step in enumerate(script["steps"]):
        steps.append(
            {
                "name": step["name"],
                "actions": absolute(step.get("actions", []), ROOT, WORK / name),
                "shot": shot_name(i, step),
                "seconds": round(audio[i][1] + PAUSE, 3),
            }
        )
        if step.get("card"):
            card = dict(step["card"])
            if card.get("logo"):
                card["logo"] = str((ROOT / card["logo"]).resolve())
            steps[-1]["card"] = card
    return {"fps": FPS, "video": video, "shot_width": SHOT_WIDTH, "steps": steps}


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


def video(name: str, script: dict, audio, timeline: dict) -> Path:
    if not shutil.which("ffmpeg"):
        fail("ffmpeg is missing (brew install ffmpeg), or build without video: --no-video")
    work = WORK / name
    fps = timeline["fps"]
    # The narration: each step's audio at the step's first frame, quiet to
    # its last.
    import numpy as np
    import soundfile as sf

    rate = 24_000
    track = np.zeros(round(timeline["frames"] / fps * rate), dtype=np.float32)
    srt = []
    for i, (step, logged) in enumerate(zip(script["steps"], timeline["steps"])):
        start = logged["start"] / fps
        wav, seconds = audio[i]
        if wav is not None:
            data, r = sf.read(wav, dtype="float32")
            assert r == rate, f"{wav}: {r} Hz"
            at = round(start * rate)
            data = data[: max(0, len(track) - at)]
            track[at : at + len(data)] = data
        text = plain(step.get("text", ""))
        if text:
            end = start + max(seconds, logged["frames"] / fps - PAUSE)
            srt.append(f"{len(srt) + 1}\n{srt_time(start)} --> {srt_time(end)}\n{text}\n")
    narration = work / "narration.wav"
    sf.write(narration, track, rate, subtype="PCM_16")
    subtitles = work / "subtitles.srt"
    subtitles.write_text("\n".join(srt), encoding="utf-8")
    out = DOCS / name / f"{name}.mp4"
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
    the_plan = plan(name, script, audio, with_video)
    timeline = play(name, the_plan)
    copy_shots(name, the_plan)
    readme(name, script, the_plan)
    if with_video:
        out = video(name, script, audio, timeline)
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
