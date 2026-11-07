"""Fish Audio S2 Pro, run locally: speaks sections given on stdin.

Run by `fishspeech.py` with the Python of the Fish venv
(`requirements-fish.txt`), never by the tutorial venv: fish-speech pins
its own torch and transformers. It loads the model once (on Apple Silicon
on the GPU by MPS, else the CPU), then reads one JSON request a line and
answers one JSON line:

    {"text": …, "out": "x.wav", "reference_audio": "voice.wav",
     "reference_text": …, "seed": 42, "temperature": 0.7, "top_p": 0.7,
     "chunk_length": 200, "normalize": true}
    → {"ok": true, "seconds": 3.2, "words": [...]}  or  {"ok": false, "error": …}

The audio is written to `out` as 16-bit mono WAV at 24 kHz; the words are
found by forced alignment (`align.py`) with wav2vec2-base-960h. The model
(fishaudio/s2-pro, about 11 GB) and the aligner (about 380 MB) come from
Hugging Face on first use, at fixed revisions, into $HF_HOME.

S2 Pro's weights are under the Fish Audio Research License: research and
non-commercial use only; commercial use needs a licence from Fish Audio.
"""

import json
import os
import sys
import time
from pathlib import Path

REPO = "fishaudio/s2-pro"
REVISION = "1de9996b6be38b745688de084d87a5633f714e4e"
ALIGNER = "facebook/wav2vec2-base-960h"
ALIGNER_REVISION = "22aad52d435eb6dbaf354bdad9b0da84ce7d6156"
RATE = 24_000


def log(message: str) -> None:
    print(f"fish-s2-pro: {message}", file=sys.stderr, flush=True)


def main() -> None:
    # Only answers on stdout: whatever the libraries print goes to stderr.
    answers = os.fdopen(os.dup(1), "w", buffering=1, encoding="utf-8")
    os.dup2(2, 1)
    os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
    sys.path.insert(0, str(Path(__file__).parent))

    import pyrootutils

    # fish-speech looks for its source tree's root; installed, it has none.
    pyrootutils.setup_root = lambda *a, **k: None

    import numpy as np
    import soundfile as sf
    import torch
    import torchaudio.functional as AF
    from huggingface_hub import snapshot_download
    from num2words import num2words
    from transformers import Wav2Vec2ForCTC, Wav2Vec2Processor

    import align
    from fish_speech.inference_engine import TTSInferenceEngine
    from fish_speech.models.dac.inference import load_model as load_decoder
    from fish_speech.models.text2semantic.inference import launch_thread_safe_queue
    from fish_speech.utils.schema import ServeReferenceAudio, ServeTTSRequest

    device = "mps" if torch.backends.mps.is_available() else "cuda" if torch.cuda.is_available() else "cpu"
    started = time.time()
    log(f"loading {REPO}@{REVISION[:8]} on {device} (first use downloads about 11 GB)")
    checkpoint = snapshot_download(REPO, revision=REVISION, allow_patterns=["*.json", "*.safetensors", "*.pth", "*.jinja"])
    precision = torch.bfloat16
    llama = launch_thread_safe_queue(checkpoint_path=checkpoint, device=device, precision=precision, compile=False)
    decoder = load_decoder(config_name="modded_dac_vq", checkpoint_path=str(Path(checkpoint) / "codec.pth"), device=device)
    engine = TTSInferenceEngine(llama_queue=llama, decoder_model=decoder, precision=precision, compile=False)
    aligner_dir = snapshot_download(ALIGNER, revision=ALIGNER_REVISION, allow_patterns=["*.json", "model.safetensors"])
    processor = Wav2Vec2Processor.from_pretrained(aligner_dir)
    aligner = Wav2Vec2ForCTC.from_pretrained(aligner_dir).eval()
    vocab = processor.tokenizer.get_vocab()
    blank = processor.tokenizer.pad_token_id
    log(f"ready in {time.time() - started:.0f} s")
    references: dict[str, ServeReferenceAudio] = {}

    def number_words(n: str) -> str:
        try:
            return num2words(float(n) if "." in n else int(n))
        except Exception:
            return align.digits_spoken(n)

    def speak(req: dict) -> dict:
        ref_path = req["reference_audio"]
        if ref_path not in references:
            references[ref_path] = ServeReferenceAudio(audio=Path(ref_path).read_bytes(), text=req["reference_text"])
        request = ServeTTSRequest(
            text=req["text"],
            references=[references[ref_path]],
            seed=int(req["seed"]),
            temperature=float(req["temperature"]),
            top_p=float(req["top_p"]),
            chunk_length=int(req["chunk_length"]),
            normalize=bool(req["normalize"]),
            format="wav",
            use_memory_cache="on",
        )
        audio = None
        for result in engine.inference(request):
            if result.code == "error":
                raise RuntimeError(str(result.error))
            if result.code == "final":
                rate, audio = result.audio
        if audio is None:
            raise RuntimeError("no audio")
        wave = torch.from_numpy(np.asarray(audio, dtype=np.float32).reshape(-1))
        out24 = AF.resample(wave, rate, RATE).clamp(-1, 1).numpy()
        Path(req["out"]).parent.mkdir(parents=True, exist_ok=True)
        sf.write(req["out"], out24, RATE, subtype="PCM_16")
        seconds = len(out24) / RATE
        with torch.inference_mode():
            x = AF.resample(wave, rate, 16_000)
            inputs = processor(x.numpy(), sampling_rate=16_000, return_tensors="pt")
            emission = torch.log_softmax(aligner(inputs.input_values).logits[0], dim=-1).tolist()
        words = align.align(req["text"], emission, vocab, blank, "|", seconds, number_words)
        return {"ok": True, "seconds": seconds, "words": words}

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
