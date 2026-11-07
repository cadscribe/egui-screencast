"""Word times for speech that comes without them: forced alignment.

A voice that gives no times of its words (Fish Audio S2 Pro, run locally)
is aligned to the text it was given: a small CTC model (wav2vec2, English
letters, a frame each 20 ms) says how likely each letter is in each frame,
and the most likely path through the text's letters in order gives each
letter, and so each word, its frames. Words come out as Kokoro gives them:
each word and each mark with when it starts and ends and whether a space
follows, so cues and subtitles work alike.

The path is found here with the standard library only, so its tests need
nothing installed; `Aligner` runs the model (torch and transformers, in
the tutorial venv already for Kokoro), downloaded from Hugging Face at a
fixed revision on first use: wav2vec2-base-960h for English (Apache 2.0,
about 380 MB), wav2vec2-base-10k-voxpopuli-ft-de for German (CC BY-NC 4.0,
about 380 MB). Another language falls back to times by the words' lengths.

Measured against Kokoro's own word times (it gives them) on Kokoro
speech: see the README; a word's start is right to a frame or two.
"""

import re

# A word, with what joins its parts ("it's", "3.5", "sheet-metal"); every
# other character that is not a space is a mark of its own.
TOKEN = re.compile(r"[^\W_]+(?:['’.,\-][^\W_]+)*|[^\w\s]|_", re.UNICODE)

# Delivery tags inline in square brackets ([pause], [excited], …): said to
# the voice that knows them, never shown, never aligned.
TAG = re.compile(r"\[[^\[\]]*\]")

# The aligner of each language: a CTC model at a fixed revision, its word
# separator, and the language for num2words.
MODELS = {
    "english": ("facebook/wav2vec2-base-960h", "22aad52d435eb6dbaf354bdad9b0da84ce7d6156", "en", "model.safetensors"),
    "german": ("facebook/wav2vec2-base-10k-voxpopuli-ft-de", "1cbaf198475e1af97cc479ca081ce4ddd2d6b5cf", "de", "pytorch_model.bin"),
}

DIGITS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]


def untagged(text: str) -> str:
    """`text` without its delivery tags, its spaces single."""
    return " ".join(TAG.sub(" ", text).split())


def tokens_of(text: str) -> list[dict]:
    """The words and marks of `text`, each with whether a space follows."""
    out = []
    for m in TOKEN.finditer(text):
        out.append({"text": m.group(0), "space": m.end() < len(text) and text[m.end()].isspace()})
    return out


def digits_spoken(number: str) -> str:
    """A number as words, digit by digit: what a caller without num2words
    gets (the worker gives num2words)."""
    return " ".join(DIGITS[int(d)] for d in number if d.isdigit())


def spelled(word: str, number_words=digits_spoken) -> str:
    """A word as the letters the aligner knows: small letters and the
    apostrophe, its numbers said ("3.5" → "three point five"), its parts
    with spaces between them. Empty for a mark."""
    w = word.lower().replace("’", "'")
    w = re.sub(r"\d+(?:\.\d+)?", lambda m: " " + number_words(m.group(0)) + " ", w)
    w = "".join(c if c.isalpha() or c in "' " else " " for c in w)
    return " ".join(w.split())


def ctc_path(emission: list[list[float]], targets: list[int], blank: int) -> list[tuple[int, int]] | None:
    """The most likely way through `targets` in order over the frames of
    `emission` (log probabilities, a row a frame): each target's first and
    last frame. None when there are fewer frames than targets."""
    frames, n = len(emission), len(targets)
    if n == 0 or frames < n:
        return None
    ninf = float("-inf")
    # best[t][j]: the best score with the first j targets said after t frames.
    best = [[ninf] * (n + 1) for _ in range(frames + 1)]
    best[0][0] = 0.0
    for t in range(frames):
        row, prev, e = best[t + 1], best[t], emission[t]
        lo = max(0, n - (frames - t))
        for j in range(lo, min(n, t + 1) + 1):
            # Stay: a blank, or the same target again.
            stay = prev[j] + (max(e[blank], e[targets[j - 1]]) if j > 0 else e[blank])
            step = prev[j - 1] + e[targets[j - 1]] if j > 0 else ninf
            row[j] = stay if stay >= step else step
    if best[frames][n] == ninf:
        return None
    spans: list[list[int]] = [[-1, -1] for _ in range(n)]
    j = n
    for t in range(frames, 0, -1):
        e = emission[t - 1]
        prev = best[t - 1]
        step = prev[j - 1] + e[targets[j - 1]] if j > 0 else ninf
        stay = prev[j] + (max(e[blank], e[targets[j - 1]]) if j > 0 else e[blank])
        if j > 0 and step >= stay:
            spans[j - 1][0] = t - 1
            if spans[j - 1][1] < 0:
                spans[j - 1][1] = t - 1
            j -= 1
        elif j > 0 and e[targets[j - 1]] > e[blank] and spans[j - 1][1] < 0:
            # The target said again: its last frame.
            spans[j - 1][1] = t - 1
    if j != 0:
        return None
    return [(s, e) for s, e in spans]


def words_from_path(
    text: str, spans_of_words: list[tuple[int, int] | None], seconds_per_frame: float, seconds: float
) -> list[dict]:
    """The tokens of `text` with times: a word from its first letter's frame
    to after its last's; a mark (or a word with no letters) where the word
    before it ends."""
    out = []
    k = 0
    last_end = 0.0
    tokens = tokens_of(text)
    for tok in tokens:
        span = None
        if spelled(tok["text"]):
            span = spans_of_words[k] if k < len(spans_of_words) else None
            k += 1
        if span is None:
            start = end = last_end
        else:
            start = span[0] * seconds_per_frame
            end = min(seconds, (span[1] + 1) * seconds_per_frame)
            last_end = end
        out.append({"text": tok["text"], "start": round(start, 3), "end": round(end, 3), "space": tok["space"]})
    return out


def align(
    text: str,
    emission: list[list[float]],
    vocab: dict[str, int],
    blank: int,
    separator: str,
    seconds: float,
    number_words=digits_spoken,
) -> list[dict]:
    """`text`'s words and marks with their times in speech of `seconds`
    whose letters' log probabilities are `emission` (a row a frame, columns
    by `vocab`, letters in capitals as wav2vec2 has them). Falls back to
    times by the length of each word should the path not be found."""
    tokens = tokens_of(text)
    words = [spelled(t["text"], number_words) for t in tokens]
    words = [w for w in words if w]
    targets: list[int] = []
    owner: list[int] = []
    for i, w in enumerate(words):
        if i:
            targets.append(vocab[separator])
            owner.append(-1)
        for c in w.replace(" ", separator):
            idx = vocab.get(c.upper(), vocab.get(c))
            if idx is None:
                continue
            targets.append(idx)
            owner.append(i)
    path = ctc_path(emission, targets, blank) if emission else None
    if path is None:
        return by_length(text, seconds)
    spans: list[tuple[int, int] | None] = [None] * len(words)
    for (s, e), i in zip(path, owner):
        if i < 0:
            continue
        cur = spans[i]
        spans[i] = (s, e) if cur is None else (min(cur[0], s), max(cur[1], e))
    return words_from_path(text, spans, seconds / len(emission), seconds)


def by_length(text: str, seconds: float) -> list[dict]:
    """Times by the length of each word over the whole: when nothing better
    is known."""
    tokens = tokens_of(text)
    total = sum(len(t["text"]) + 1 for t in tokens) or 1
    out = []
    at = 0.0
    for t in tokens:
        span = seconds * (len(t["text"]) + 1) / total
        out.append({"text": t["text"], "start": round(at, 3), "end": round(at + span, 3), "space": t["space"]})
        at += span
    return out


class Aligner:
    """The CTC model of a language, loaded on first use; `words(text, wav)`
    the words of `text` with their times in the WAV."""

    def __init__(self, language: str | None):
        self.language = (language or "english").lower()
        self.model = None

    def known(self) -> bool:
        return self.language in MODELS

    def words(self, text: str, wav) -> list[dict]:
        import wave as wavefile

        with wavefile.open(str(wav), "rb") as w:
            rate, frames = w.getframerate(), w.readframes(w.getnframes())
        import numpy as np

        audio = np.frombuffer(frames, dtype="<i2").astype(np.float32) / 32768.0
        seconds = len(audio) / rate
        text = untagged(text)
        if not self.known():
            return by_length(text, seconds)
        import torch
        from huggingface_hub import snapshot_download
        from num2words import num2words
        from transformers import Wav2Vec2FeatureExtractor, Wav2Vec2ForCTC, Wav2Vec2CTCTokenizer

        repo, revision, lang, weights = MODELS[self.language]
        if self.model is None:
            path = snapshot_download(repo, revision=revision, allow_patterns=["*.json", weights])
            self.extractor = Wav2Vec2FeatureExtractor.from_pretrained(path)
            self.tokenizer = Wav2Vec2CTCTokenizer.from_pretrained(path)
            self.model = Wav2Vec2ForCTC.from_pretrained(path).eval()
        # To 16 kHz by the FFT: the whole clip at once.
        n = round(len(audio) * 16_000 / rate)
        x = np.fft.irfft(np.fft.rfft(audio)[: n // 2 + 1], n) * (n / len(audio))
        inputs = self.extractor(x.astype(np.float32), sampling_rate=16_000, return_tensors="pt")
        with torch.inference_mode():
            emission = torch.log_softmax(self.model(inputs.input_values).logits[0], dim=-1).tolist()

        def number_words(number: str) -> str:
            try:
                return num2words(float(number) if "." in number else int(number), lang=lang)
            except Exception:
                return digits_spoken(number)

        vocab = self.tokenizer.get_vocab()
        separator = self.tokenizer.word_delimiter_token or "|"
        return align(text, emission, vocab, self.tokenizer.pad_token_id, separator, seconds, number_words)
