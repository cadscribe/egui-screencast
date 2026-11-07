"""Narration by Fish Audio S2 Pro, run locally with mlx-speech.

S2 Pro (open weights, 8-bit for MLX: appautomaton/fishaudio-s2-pro-8bit-mlx,
about 7 GB, about 10 GB of memory while it speaks) runs on Apple Silicon
by mlx-speech, which the user installs (`uv tool install mlx-speech`, with
its own Python); the runner neither installs nor downloads it. Its Python
is found by the `mlx-speech` command on the PATH (its first line), or
given: `--fish-python`, `$FISH_SPEECH_PYTHON`. `fishspeech_worker.py` is
started there once a run, on the first section not in the cache, and
speaks one section after another (5 to 7 times slower than the speech).

**A voice** is a reference recording and its exact transcript, in the
language of the text: S2 Pro has no preset voices, and a seed alone is
not one. Voices come from a `voices.yaml` (`--voices DIR`,
`$SCREENCAST_VOICES`, a script's `voices:`):

    dir: fish-s2-pro
    voices:
      en-gb-female: {language: English, reference: en-gb-female.wav, transcript: "…"}

or the voice is a WAV's path, its words beside it (`.lab`, `.txt`). A
fixed seed makes a section sound the same each time.

**Tags** in square brackets steer the delivery ([pause], [excited],
[emphasis], [whisper]); S2 Pro gets them, the subtitles and the alignment
do not.

**Word times**: S2 Pro gives none; `align.Aligner` finds them by forced
alignment, in the tutorial venv.

**Licence:** the Fish Audio Research License: research and non-commercial
use only (no monetised channel, no sponsorship); commercial use needs a
licence from Fish Audio. The notices say so.
"""

import hashlib
import json
import os
import shutil
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path

from align import Aligner, untagged
from elevenlabs import Cache, SpeechError, wav_seconds

BACKEND = "fish-s2-pro"
MODEL = "appautomaton/fishaudio-s2-pro-8bit-mlx"
OUTPUT_FORMAT = "wav_24000"
SETTINGS = {"seed": 1}
LICENCE = "Fish Audio Research License, non-commercial use only"
CACHE_FORMAT = 1
WORKER = Path(__file__).with_name("fishspeech_worker.py")


def normalized(text: str) -> str:
    """The text as sent and keyed: its tags kept, its spaces single."""
    return " ".join(text.split())


@dataclass
class Voice:
    """A reference recording, its transcript and language, and the seed."""

    name: str
    reference: Path
    transcript: str
    language: str | None = None
    settings: dict = field(default_factory=lambda: dict(SETTINGS))
    engine: str = "mlx-speech"

    def __post_init__(self):
        self.reference = Path(self.reference)
        if not self.reference.is_file():
            raise SpeechError(f"Fish S2 Pro: the reference recording {self.reference} of {self.name} is missing")
        if not self.transcript.strip():
            raise SpeechError(f"Fish S2 Pro: the transcript of {self.reference} is missing")
        self.reference_hash = hashlib.sha256(self.reference.read_bytes()).hexdigest()

    @property
    def generation_language(self) -> str | None:
        """What S2 Pro is told: nothing for English (as the references
        were made), else the language's name."""
        if not self.language or self.language.lower() == "english":
            return None
        return self.language

    def key(self, text: str) -> str:
        """A hash of the backend, the engine and model, the reference (its
        bytes and transcript), the language, the seed, the output format
        and the text as said (tags and all)."""
        material = json.dumps(
            {
                "format": CACHE_FORMAT,
                "backend": BACKEND,
                "engine": self.engine,
                "model": MODEL,
                "reference": self.reference_hash,
                "transcript": " ".join(self.transcript.split()),
                "language": self.generation_language,
                "settings": self.settings,
                "output_format": OUTPUT_FORMAT,
                "text": normalized(text),
            },
            sort_keys=True,
            ensure_ascii=False,
        )
        return hashlib.sha256(material.encode()).hexdigest()[:24]


def voice_of(spec: str, voices_dir: Path | None, base: Path, settings: dict) -> Voice:
    """The voice named `spec` in `voices_dir/voices.yaml`, or the WAV at
    `spec` with its transcript beside it."""
    path = (base / spec).resolve()
    if spec.lower().endswith(".wav") or path.is_file():
        transcript = ""
        for suffix in (".lab", ".txt"):
            side = path.with_suffix(suffix)
            if side.is_file():
                transcript = side.read_text(encoding="utf-8").strip()
                break
        if not transcript:
            raise SpeechError(f"Fish S2 Pro: what is said in {path} is missing: write it to {path.with_suffix('.lab')}")
        return Voice(path.stem, path, transcript, settings.get("language"), {"seed": settings.get("seed", 1)})
    if voices_dir is None:
        raise SpeechError(
            f"Fish S2 Pro: the voice {spec!r} is not a WAV, and no voices.yaml is given "
            "(--voices DIR, $SCREENCAST_VOICES or the script's voices:)"
        )
    import yaml

    index = Path(voices_dir) / "voices.yaml"
    try:
        listed = yaml.safe_load(index.read_text(encoding="utf-8")) or {}
    except OSError as e:
        raise SpeechError(f"Fish S2 Pro: {index} cannot be read ({e.strerror})") from None
    except yaml.YAMLError as e:
        where = getattr(e, "problem_mark", None)
        at = f" at line {where.line + 1}, column {where.column + 1}" if where else ""
        raise SpeechError(f"Fish S2 Pro: {index} is not valid YAML{at}: {getattr(e, 'problem', e)}") from None
    entry = (listed.get("voices") or {}).get(spec)
    if not entry:
        known = ", ".join(sorted(listed.get("voices") or {}))
        raise SpeechError(f"Fish S2 Pro: no voice {spec!r} in {index} (there are: {known})")
    folder = Path(voices_dir) / str(listed.get("dir") or ".")
    return Voice(
        spec,
        folder / entry["reference"],
        str(entry.get("transcript") or ""),
        settings.get("language") or entry.get("language"),
        {"seed": settings.get("seed", 1)},
    )


def mlx_python() -> Path | None:
    """The Python of the `mlx-speech` command on the PATH (its first line)."""
    command = shutil.which("mlx-speech") or str(Path.home() / ".local" / "bin" / "mlx-speech")
    try:
        first = Path(command).read_text(encoding="utf-8", errors="replace").splitlines()[0]
    except (OSError, IndexError):
        return None
    return Path(first[2:].strip()) if first.startswith("#!") else None


class Worker:
    """`fishspeech_worker.py` in mlx-speech's Python: started on the first
    request, asked one section at a time; offline, so that it never
    downloads the model."""

    def __init__(self, python: Path | None, env: dict | None = None, command: list[str] | None = None):
        self.python = python
        self.env = env or {}
        self.command = command
        self.process: subprocess.Popen | None = None

    def start(self) -> None:
        if self.command is None and (self.python is None or not Path(self.python).exists()):
            raise SpeechError(
                "Fish S2 Pro: mlx-speech is not installed (uv tool install mlx-speech, then its model "
                "fish-s2-pro by mlx-speech tts --model fish-s2-pro …), or say where its Python is: "
                "--fish-python"
            )
        command = self.command or [str(self.python), str(WORKER)]
        env = dict(os.environ)
        for k, v in self.env.items():
            if v is None:
                env.pop(k, None)
            else:
                env[k] = v
        env["HF_HUB_OFFLINE"] = "1"
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env, bufsize=1
        )

    def __call__(self, request: dict) -> dict:
        if self.process is None:
            self.start()
        assert self.process and self.process.stdin and self.process.stdout
        try:
            self.process.stdin.write(json.dumps(request) + "\n")
            self.process.stdin.flush()
        except BrokenPipeError:
            pass
        line = self.process.stdout.readline()
        if not line:
            code = self.process.wait()
            raise SpeechError(f"Fish S2 Pro: the worker ended ({code}) without an answer; its log is above")
        answer = json.loads(line)
        if not answer.get("ok"):
            raise SpeechError(f"Fish S2 Pro: {answer.get('error', 'no audio')}")
        return answer

    def close(self) -> None:
        if self.process is not None:
            if self.process.stdin:
                self.process.stdin.close()
            try:
                self.process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                self.process.kill()
            self.process = None


class Narrator:
    """Speaks sections with S2 Pro through the cache, finds their words'
    times, and counts: how many from the cache, how many spoken, their
    characters and seconds."""

    label = "Fish Audio S2 Pro"

    def __init__(self, voice: Voice, cache: Path, worker, dry_run: bool = False, aligner=None):
        self.voice = voice
        self.cache = Cache(cache)
        self.worker = worker
        self.aligner = aligner or Aligner(voice.language)
        self.dry_run = dry_run
        self.cached = 0
        self.spoken = 0
        self.characters = 0
        self.seconds = 0.0
        self.compute = 0.0

    def say(self, text: str) -> tuple[Path | None, float, list[dict]]:
        text = normalized(text)
        key = self.voice.key(text)
        hit = self.cache.get(key)
        if hit is not None:
            self.cached += 1
            wav, words = hit
            return wav, wav_seconds(wav), words
        self.spoken += 1
        self.characters += len(untagged(text))
        if self.dry_run:
            return None, 0.0, []
        print(f"  {self.label}: {text[:60]}…", flush=True)
        started = time.time()
        self.cache.root.mkdir(parents=True, exist_ok=True)
        out = self.cache.root / f".{key}.{os.getpid()}.wav"
        try:
            self.worker(
                {
                    "text": text,
                    "out": str(out),
                    "reference_audio": str(self.voice.reference.resolve()),
                    "reference_text": self.voice.transcript,
                    "language": self.voice.generation_language,
                    **self.voice.settings,
                }
            )
            import wave

            with wave.open(str(out), "rb") as w:
                if w.getframerate() != 24_000 or w.getsampwidth() != 2 or w.getnchannels() != 1:
                    raise SpeechError("Fish S2 Pro: the worker's audio is not 16-bit mono at 24 kHz")
                pcm = w.readframes(w.getnframes())
            words = self.aligner.words(text, out)
        finally:
            out.unlink(missing_ok=True)
        self.compute += time.time() - started
        wav = self.cache.put(
            key,
            pcm,
            words,
            {
                "text": text,
                "backend": BACKEND,
                "model": MODEL,
                "voice": self.voice.name,
                "reference": str(self.voice.reference),
                "language": self.voice.language,
                "settings": self.voice.settings,
                "characters": len(untagged(text)),
                "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
            },
        )
        seconds = wav_seconds(wav)
        self.seconds += seconds
        return wav, seconds, words

    def report(self) -> str:
        total = self.cached + self.spoken
        if self.dry_run:
            return (
                f"{self.label} (dry run): {total} sections, {self.cached} in the cache, "
                f"{self.spoken} to speak ({self.characters} characters)"
            )
        return (
            f"{self.label}: {total} sections, {self.cached} from the cache, {self.spoken} spoken "
            f"({self.characters} characters, {self.seconds:.0f} s of speech in {self.compute:.0f} s)"
        )

    def close(self) -> None:
        close = getattr(self.worker, "close", None)
        if close:
            close()
