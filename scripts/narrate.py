"""Narration of a tutorial's steps with Kokoro (hexgrad/Kokoro-82M).

The model and the voices come from Hugging Face at a fixed revision, into
the cache below `target/tutorials/hf`; a step's audio is kept by a hash of
its text, voice and speed, so only changed steps are spoken again.

    python narrate.py "Text to say" out.wav [--voice am_michael] [--phonemes]
"""

import hashlib
import os
import sys
from pathlib import Path

REPO = "hexgrad/Kokoro-82M"
# The model's revision on Hugging Face: the same weights and voices on
# every run.
REVISION = "f3ff3571791e39611d31c381e3a41a3af07b4987"
RATE = 24_000
VOICE = "am_michael"

_pipeline = None


def _load():
    global _pipeline
    if _pipeline is not None:
        return _pipeline
    try:
        from huggingface_hub import hf_hub_download
        from kokoro import KModel, KPipeline
    except ImportError as e:
        sys.exit(
            f"tutorials: Kokoro is not installed ({e}).\n"
            "  make tutorial-venv   # creates target/tutorials/venv from "
            "requirements.txt beside this script\n"
            "or run without narration: --no-voice"
        )
    config = hf_hub_download(REPO, "config.json", revision=REVISION)
    weights = hf_hub_download(REPO, "kokoro-v1_0.pth", revision=REVISION)
    model = KModel(repo_id=REPO, config=config, model=weights).eval()
    # American English ("a"); British voices (bf_, bm_) would take "b".
    _pipeline = KPipeline(lang_code="a", repo_id=REPO, model=model)
    return _pipeline


def voice_file(voice: str) -> str:
    from huggingface_hub import hf_hub_download

    return hf_hub_download(REPO, f"voices/{voice}.pt", revision=REVISION)


def key(text: str, voice: str, speed: float) -> str:
    """The cache key of a narration."""
    h = hashlib.sha256(f"{REVISION}\n{voice}\n{speed}\n{text}".encode())
    return h.hexdigest()[:16]


def speak(text: str, out: Path, voice: str = VOICE, speed: float = 1.0) -> float:
    """Writes `text` spoken as a 24 kHz mono WAV to `out`; its seconds."""
    import numpy as np
    import soundfile as sf

    pipeline = _load()
    chunks = [
        r.audio.numpy()
        for r in pipeline(text, voice=voice_file(voice), speed=speed)
        if r.audio is not None
    ]
    audio = np.concatenate(chunks) if chunks else np.zeros(RATE // 2, dtype=np.float32)
    out.parent.mkdir(parents=True, exist_ok=True)
    sf.write(out, audio, RATE, subtype="PCM_16")
    return len(audio) / RATE


def phonemes(text: str, voice: str = VOICE) -> str:
    """What Kokoro will say, as phonemes: to check numbers and units."""
    pipeline = _load()
    return " | ".join(r.phonemes for r in pipeline(text, voice=voice_file(voice)))


def duration(path: Path) -> float:
    import soundfile as sf

    info = sf.info(path)
    return info.frames / info.samplerate


if __name__ == "__main__":
    os.environ.setdefault(
        "HF_HOME", str(Path(__file__).resolve().parents[2] / "target/tutorials/hf")
    )
    args = sys.argv[1:]
    voice = VOICE
    if "--voice" in args:
        i = args.index("--voice")
        voice = args[i + 1]
        del args[i : i + 2]
    if "--phonemes" in args:
        args.remove("--phonemes")
        print(phonemes(args[0], voice))
    else:
        print(f"{speak(args[0], Path(args[1]), voice):.2f} s")
