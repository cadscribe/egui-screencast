"""Narration by Fish Audio S2 Pro, run locally from its open weights.

The model runs in a venv of its own (fish-speech pins its own torch and
transformers): `requirements-fish.txt`, installed with `--no-deps`, its
Python given by `--fish-python` or `$FISH_SPEECH_PYTHON` (default
`<work>/fish-venv/bin/python`). `fishspeech_worker.py` is started there
once a run, on the first section not in the cache, loads the model (on
Apple Silicon on the GPU by MPS) and speaks one section after another.

S2 Pro speaks in the voice of a **reference recording**: a WAV of 10 to 30
seconds and what is said in it (beside it as `.lab` or `.txt`, or the
script's `voice_reference_text`). Without one each section would have
another voice. Use a voice you may use: your own, one that is licensed, or
one made by another synthetic voice (Kokoro's, say).

S2 Pro gives no times of its words; they are found by forced alignment
(`align.py`, wav2vec2-base-960h, English).

**Licence:** S2 Pro's weights are under the Fish Audio Research License:
research and non-commercial use only; commercial use needs a licence from
Fish Audio (business@fish.audio). The notices say so.

Each section is kept in the cache by a key of everything that makes its
sound, as for ElevenLabs (`elevenlabs.Cache`); it saves minutes of compute
a section.
"""

import hashlib
import json
import os
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path

from elevenlabs import Cache, SpeechError, normalized, wav_seconds

BACKEND = "fish-s2-pro"
MODEL = "fishaudio/s2-pro"
# The revisions the worker loads (`fishspeech_worker.py`), in the key.
REVISION = "1de9996b6be38b745688de084d87a5633f714e4e"
FISH_SPEECH = "0f8a12e8a651062fdee9c33a894bdccd3a8513f1"
ALIGNER = "facebook/wav2vec2-base-960h@22aad52d435eb6dbaf354bdad9b0da84ce7d6156"
OUTPUT_FORMAT = "wav_24000"
# A fixed seed: the same text, the same sound.
SETTINGS = {"seed": 42, "temperature": 0.7, "top_p": 0.7, "chunk_length": 200, "normalize": True}
LICENCE = "Fish Audio Research License, non-commercial use"
CACHE_FORMAT = 1
WORKER = Path(__file__).with_name("fishspeech_worker.py")


@dataclass
class Voice:
    """A reference recording and the settings."""

    reference: Path
    reference_text: str = ""
    settings: dict = field(default_factory=lambda: dict(SETTINGS))

    def __post_init__(self):
        self.reference = Path(self.reference)
        if not self.reference.is_file():
            raise SpeechError(
                f"Fish S2 Pro: the reference recording {self.reference} is missing "
                "(fish-s2-pro:<a WAV of 10 to 30 s>, its words beside it as .lab or .txt)"
            )
        if not self.reference_text:
            for suffix in (".lab", ".txt"):
                side = self.reference.with_suffix(suffix)
                if side.is_file():
                    self.reference_text = side.read_text(encoding="utf-8").strip()
                    break
        if not self.reference_text:
            raise SpeechError(
                f"Fish S2 Pro: what is said in {self.reference} is missing: write it to "
                f"{self.reference.with_suffix('.lab')} or give voice_reference_text"
            )
        self.reference_hash = hashlib.sha256(self.reference.read_bytes()).hexdigest()

    @property
    def name(self) -> str:
        return self.reference.stem

    def key(self, text: str) -> str:
        """A hash of the backend, the model and code at their revisions, the
        aligner, the reference (its bytes and words), the settings, the
        output format and the text as said."""
        material = json.dumps(
            {
                "format": CACHE_FORMAT,
                "backend": BACKEND,
                "model": f"{MODEL}@{REVISION}",
                "fish_speech": FISH_SPEECH,
                "aligner": ALIGNER,
                "reference": self.reference_hash,
                "reference_text": normalized(self.reference_text),
                "settings": self.settings,
                "output_format": OUTPUT_FORMAT,
                "text": normalized(text),
            },
            sort_keys=True,
            ensure_ascii=False,
        )
        return hashlib.sha256(material.encode()).hexdigest()[:24]


class Worker:
    """`fishspeech_worker.py` in the Fish venv: started on the first
    request, asked one section at a time."""

    def __init__(self, python: Path, env: dict | None = None, command: list[str] | None = None):
        self.python = Path(python)
        self.env = env
        self.command = command
        self.process: subprocess.Popen | None = None

    def start(self) -> None:
        if self.command is None and not self.python.exists():
            raise SpeechError(
                f"Fish S2 Pro: no Python at {self.python}: make its venv "
                "(uv venv -p 3.12 <work>/fish-venv && VIRTUAL_ENV=<work>/fish-venv uv pip install "
                "--no-deps -r requirements-fish.txt), or say where it is: --fish-python"
            )
        command = self.command or [str(self.python), str(WORKER)]
        env = dict(os.environ, **(self.env or {}))
        # Its log (loading, downloading) on our stderr, its answers on a pipe.
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
    """Speaks sections with S2 Pro through the cache, and counts: how many
    from the cache, how many spoken, their characters and seconds."""

    label = "Fish Audio S2 Pro"

    def __init__(self, voice: Voice, cache: Path, worker, dry_run: bool = False):
        self.voice = voice
        self.cache = Cache(cache)
        self.worker = worker
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
        self.characters += len(text)
        if self.dry_run:
            return None, 0.0, []
        print(f"  {self.label}: {text[:60]}…", flush=True)
        started = time.time()
        out = self.cache.root / f".{key}.{os.getpid()}.wav"
        self.cache.root.mkdir(parents=True, exist_ok=True)
        try:
            answer = self.worker(
                {
                    "text": text,
                    "out": str(out),
                    "reference_audio": str(self.voice.reference.resolve()),
                    "reference_text": self.voice.reference_text,
                    **self.voice.settings,
                }
            )
            import wave

            with wave.open(str(out), "rb") as w:
                if w.getframerate() != 24_000 or w.getsampwidth() != 2 or w.getnchannels() != 1:
                    raise SpeechError("Fish S2 Pro: the worker's audio is not 16-bit mono at 24 kHz")
                pcm = w.readframes(w.getnframes())
        finally:
            out.unlink(missing_ok=True)
        self.compute += time.time() - started
        wav = self.cache.put(
            key,
            pcm,
            answer["words"],
            {
                "text": text,
                "backend": BACKEND,
                "model": f"{MODEL}@{REVISION}",
                "reference": str(self.voice.reference),
                "settings": self.voice.settings,
                "characters": len(text),
                "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
            },
        )
        seconds = wav_seconds(wav)
        self.seconds += seconds
        return wav, seconds, answer["words"]

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
