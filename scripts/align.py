"""Word times for speech that comes without them: forced alignment.

A voice that gives no times of its words (Fish Audio S2 Pro, run locally)
is aligned to the text it was given: a small CTC model (wav2vec2, English
letters, a frame each 20 ms) says how likely each letter is in each frame,
and the most likely path through the text's letters in order gives each
letter, and so each word, its frames. Words come out as Kokoro gives them:
each word and each mark with when it starts and ends and whether a space
follows, so cues and subtitles work alike.

This module is the standard library only (the model runs in the Fish venv,
`fishspeech_worker.py`); its tests need nothing installed.
"""

import re

# A word, with what joins its parts ("it's", "3.5", "sheet-metal"); every
# other character that is not a space is a mark of its own.
TOKEN = re.compile(r"[^\W_]+(?:['’.,\-][^\W_]+)*|[^\w\s]|_", re.UNICODE)

DIGITS = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]


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
    w = re.sub(r"[^a-z' ]+", " ", w)
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
