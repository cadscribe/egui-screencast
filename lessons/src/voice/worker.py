"""egui-lessons' voice worker (OpenSolve's ADR 0056): speaks a lesson's sections given on
stdin, one JSON request a line, one JSON answer a line.

    python -I worker.py fish     # in mlx-speech's own Python (S2 Pro)
    python -I worker.py kokoro   # in the tutorial venv (Kokoro)

    {"text": …, "out": "x.wav", "reference_audio": "voice.wav",
     "reference_text": …, "seed": 1, "language": "German" | null}   (fish)
    {"text": …, "out": "x.wav", "voice": "am_michael", "speed": 1.0}  (kokoro)
    → {"ok": true, "seconds": 3.2}  or  {"ok": false, "error": …}

Built into the app and written beside its voice cache. As egui-screencast's
`fishspeech_worker.py` and `narrate.py` speak, so that a section sounds as
in a recording: 16-bit mono WAV at 24 kHz (S2 Pro's 44.1 kHz resampled by
the FFT). Run with HF_HUB_OFFLINE=1: a model not on the machine is an
error, never a download. S2 Pro's weights are under the Fish Audio Research
License: non-commercial use only.
"""

import json
import os
import sys

RATE = 24_000
FISH = "fish-s2-pro"
KOKORO_REPO = "hexgrad/Kokoro-82M"
KOKORO_REVISION = "f3ff3571791e39611d31c381e3a41a3af07b4987"


def fish():
    import mlx_speech
    import numpy as np
    import soundfile as sf

    model = mlx_speech.tts.load(FISH)

    def speak(req):
        kwargs = {
            "reference_audio": req["reference_audio"],
            "reference_text": req["reference_text"],
            "seed": int(req["seed"]),
        }
        if req.get("language"):
            kwargs["language"] = req["language"]
        result = model.generate(req["text"], **kwargs)
        audio = np.asarray(result.waveform, dtype=np.float32).reshape(-1)
        if audio.size == 0:
            raise RuntimeError("no audio")
        n = round(audio.size * RATE / int(result.sample_rate))
        out = np.fft.irfft(np.fft.rfft(audio)[: n // 2 + 1], n) * (n / audio.size)
        sf.write(req["out"], np.clip(out, -1, 1), RATE, subtype="PCM_16")
        return n / RATE

    return speak


def kokoro():
    import numpy as np
    import soundfile as sf
    from huggingface_hub import hf_hub_download
    from kokoro import KModel, KPipeline

    config = hf_hub_download(KOKORO_REPO, "config.json", revision=KOKORO_REVISION)
    weights = hf_hub_download(KOKORO_REPO, "kokoro-v1_0.pth", revision=KOKORO_REVISION)
    model = KModel(repo_id=KOKORO_REPO, config=config, model=weights).eval()
    pipeline = KPipeline(lang_code="a", repo_id=KOKORO_REPO, model=model)

    def speak(req):
        voice = hf_hub_download(KOKORO_REPO, f"voices/{req['voice']}.pt", revision=KOKORO_REVISION)
        chunks = [
            r.audio.numpy() for r in pipeline(req["text"], voice=voice, speed=float(req["speed"])) if r.audio is not None
        ]
        audio = np.concatenate(chunks) if chunks else np.zeros(RATE // 2, dtype=np.float32)
        sf.write(req["out"], audio, RATE, subtype="PCM_16")
        return len(audio) / RATE

    return speak


def main():
    # Only answers on stdout: whatever the libraries print goes to stderr.
    answers = os.fdopen(os.dup(1), "w", buffering=1, encoding="utf-8")
    os.dup2(2, 1)
    engine = sys.argv[1] if len(sys.argv) > 1 else FISH
    try:
        speak = kokoro() if engine == "kokoro" else fish()
    except Exception as e:
        error = f"{engine} could not be loaded ({type(e).__name__}: {e})"
        for line in sys.stdin:
            if line.strip():
                answers.write(json.dumps({"ok": False, "error": error}) + "\n")
        return
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            answer = {"ok": True, "seconds": speak(json.loads(line))}
        except Exception as e:
            answer = {"ok": False, "error": f"{type(e).__name__}: {e}"}
        answers.write(json.dumps(answer) + "\n")


if __name__ == "__main__":
    main()
