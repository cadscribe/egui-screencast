"""Fish Audio S2 Pro by mlx-speech: speaks sections given on stdin.

Run by `fishspeech.py` with the Python of the user's own mlx-speech
installation (`uv tool install mlx-speech`), never the tutorial venv. It
loads `fish-s2-pro` once (appautomaton/fishaudio-s2-pro-8bit-mlx, from the
Hugging Face cache: offline, it never downloads it), then reads one JSON
request a line and answers one JSON line:

    {"text": …, "out": "x.wav", "reference_audio": "voice.wav",
     "reference_text": …, "seed": 1, "language": "German" | null}
    → {"ok": true, "seconds": 3.2}  or  {"ok": false, "error": …}

The audio is written to `out` as 16-bit mono WAV at 24 kHz (S2 Pro's
44.1 kHz resampled by the FFT). The words' times are found by the caller.

S2 Pro's weights are under the Fish Audio Research License: research and
non-commercial use only.
"""

import json
import os
import sys
import time

RATE = 24_000
MODEL = "fish-s2-pro"


def log(message: str) -> None:
    print(f"fish-s2-pro: {message}", file=sys.stderr, flush=True)


def main() -> None:
    # Only answers on stdout: whatever the libraries print goes to stderr.
    answers = os.fdopen(os.dup(1), "w", buffering=1, encoding="utf-8")
    os.dup2(2, 1)
    started = time.time()
    try:
        import mlx_speech
        import numpy as np
        import soundfile as sf

        model = mlx_speech.tts.load(MODEL)
    except Exception as e:
        answers.write(
            json.dumps(
                {
                    "ok": False,
                    "error": f"{MODEL} could not be loaded ({type(e).__name__}: {e}); install mlx-speech "
                    f"and its model yourself (mlx-speech tts --model {MODEL} …): the runner never downloads it",
                }
            )
            + "\n"
        )
        return
    log(f"{MODEL} loaded in {time.time() - started:.0f} s")

    def speak(req: dict) -> dict:
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
        return {"ok": True, "seconds": n / RATE}

    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            answer = speak(json.loads(line))
        except Exception as e:  # said to the caller, which keeps nothing
            answer = {"ok": False, "error": f"{type(e).__name__}: {e}"}
        answers.write(json.dumps(answer) + "\n")


if __name__ == "__main__":
    main()
