"""Narration by ElevenLabs, a step's text at a time, each kept in a cache.

ElevenLabs is asked over its HTTP API (`POST /v1/text-to-speech/{voice_id}/
with-timestamps`, the standard library's `urllib`, nothing to install): it
answers with the audio (raw 16-bit PCM at 24 kHz, `pcm_24000`) and when
each character of the text is said. The characters are made the words
Kokoro gives (`narrate.py`): each word and each mark with when it starts
and ends and whether a space follows, so cues and subtitles work the same.

ElevenLabs bills by the characters sent. Each text section is kept in the
cache by a key of everything that makes its sound (backend, voice, model,
voice settings, output format, the text as said); a section whose key is
in the cache is never asked for again. An entry is a folder made whole and
then renamed into place, so a failed request leaves nothing behind.

The API key is read from `ELEVENLABS_API_KEY` and only ever sent in the
request's `xi-api-key` header: never written, never printed.

    python elevenlabs.py voices                      # the account's voices
    python elevenlabs.py prune --older-than 30 [--cache DIR]
    python elevenlabs.py stats [--cache DIR]
"""

import base64
import hashlib
import json
import os
import shutil
import sys
import time
import urllib.error
import urllib.request
import wave
from dataclasses import dataclass, field
from pathlib import Path

API = "https://api.elevenlabs.io"
KEY_ENV = "ELEVENLABS_API_KEY"
# The model when none is given: ElevenLabs' default, timestamps and all.
MODEL = "eleven_multilingual_v2"
# Raw 16-bit mono PCM at 24 kHz: the rate of the rest of the pipeline, on
# every plan (44.1 kHz needs Pro), no decoder needed.
OUTPUT_FORMAT = "pcm_24000"
RATE = 24_000
# The voice settings sent when the script gives none (ElevenLabs' own
# defaults, sent so that the cache key says what was used).
SETTINGS = {"stability": 0.5, "similarity_boost": 0.75, "style": 0.0, "use_speaker_boost": True, "speed": 1.0}
# Bumped when what an entry holds changes: every entry is made again.
CACHE_FORMAT = 1
TIMEOUT = 120


class SpeechError(Exception):
    """A request ElevenLabs did not answer with speech; its message says
    what to do (and never holds the API key)."""


@dataclass
class Voice:
    """What makes the sound of a section, besides its text."""

    voice_id: str
    model_id: str = MODEL
    settings: dict = field(default_factory=lambda: dict(SETTINGS))
    output_format: str = OUTPUT_FORMAT

    def key(self, text: str) -> str:
        """The cache key of `text` said by this voice: a hash of the backend,
        the voice, the model, the settings, the output format and the text
        as said (whitespace made single spaces)."""
        material = json.dumps(
            {
                "format": CACHE_FORMAT,
                "backend": "elevenlabs",
                "voice_id": self.voice_id,
                "model_id": self.model_id,
                "settings": self.settings,
                "output_format": self.output_format,
                "text": normalized(text),
            },
            sort_keys=True,
            ensure_ascii=False,
        )
        return hashlib.sha256(material.encode()).hexdigest()[:24]


def normalized(text: str) -> str:
    """The text as it is sent and as it is keyed."""
    return " ".join(text.split())


def setting(value: str):
    """A voice setting from the command line: a number, true/false."""
    low = value.strip().lower()
    if low in ("true", "false"):
        return low == "true"
    for kind in (int, float):
        try:
            return kind(value)
        except ValueError:
            pass
    return value


# Characters of a word besides letters and digits when between two of them:
# "it's", "3.5", "sheet-metal".
JOINERS = "'’.,-"


def words_of(alignment: dict) -> list[dict]:
    """ElevenLabs' characters and their times as words: each word and each
    mark with its first character's start, its last character's end, and
    whether a space follows (as Kokoro gives them)."""
    chars = alignment.get("characters") or []
    starts = alignment.get("character_start_times_seconds") or []
    ends = alignment.get("character_end_times_seconds") or []
    if not (len(chars) == len(starts) == len(ends)):
        raise SpeechError("ElevenLabs' alignment has unequal lists of characters and times")
    out: list[dict] = []
    word: list[int] = []

    def close():
        if word:
            out.append(
                {
                    "text": "".join(chars[k] for k in word),
                    "start": round(float(starts[word[0]]), 3),
                    "end": round(float(ends[word[-1]]), 3),
                    "space": False,
                }
            )
            word.clear()

    for k, c in enumerate(chars):
        if c.isspace():
            close()
            if out:
                out[-1]["space"] = True
        elif c.isalnum():
            word.append(k)
        elif (
            c in JOINERS
            and word
            and k + 1 < len(chars)
            and chars[k + 1].isalnum()
            and (c not in ".," or (chars[k - 1].isdigit() and chars[k + 1].isdigit()))
        ):
            word.append(k)
        else:
            close()
            out.append({"text": c, "start": round(float(starts[k]), 3), "end": round(float(ends[k]), 3), "space": False})
    close()
    return out


def api_key() -> str:
    key = os.environ.get(KEY_ENV, "").strip()
    if not key:
        raise SpeechError(
            f"{KEY_ENV} is not set: ElevenLabs needs an API key (elevenlabs.io → "
            "Developers → API keys); set it in the environment, never in a script"
        )
    return key


def _detail(body: bytes) -> str:
    """ElevenLabs' reason for an error, short."""
    try:
        detail = json.loads(body).get("detail")
    except (ValueError, AttributeError):
        return body[:200].decode("utf-8", "replace").strip()
    if isinstance(detail, dict):
        return str(detail.get("message") or detail.get("status") or detail)[:300]
    if isinstance(detail, list):
        return "; ".join(str(d.get("msg", d)) if isinstance(d, dict) else str(d) for d in detail)[:300]
    return str(detail)[:300]


def _call(method: str, path: str, body: dict | None, opener) -> dict:
    """One request to the API; its JSON. Errors as `SpeechError`s that say
    what to do."""
    key = api_key()
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(
        API + path,
        data=data,
        method=method,
        headers={"xi-api-key": key, "Content-Type": "application/json", "Accept": "application/json"},
    )
    try:
        with opener(request, timeout=TIMEOUT) as response:
            return json.loads(response.read())
    except urllib.error.HTTPError as e:
        try:
            reason = _detail(e.read() or b"")
        finally:
            e.close()
        if e.code == 401:
            raise SpeechError(
                f"ElevenLabs refused the API key (HTTP 401: {reason}); check {KEY_ENV}"
            ) from None
        if e.code == 429:
            raise SpeechError(
                f"ElevenLabs: too many requests or the quota is used up (HTTP 429: {reason}); "
                "wait and run again, the sections spoken so far are kept"
            ) from None
        if e.code == 404:
            raise SpeechError(f"ElevenLabs does not know the voice or model (HTTP 404: {reason})") from None
        raise SpeechError(f"ElevenLabs answered HTTP {e.code}: {reason}") from None
    except urllib.error.URLError as e:
        raise SpeechError(f"ElevenLabs is not reachable: {e.reason}") from None
    except ValueError:
        raise SpeechError("ElevenLabs answered with something that is not JSON") from None


def synthesize(text: str, voice: Voice, opener=urllib.request.urlopen) -> tuple[bytes, list[dict]]:
    """`text` spoken by ElevenLabs: its PCM samples and its words."""
    from urllib.parse import quote

    answer = _call(
        "POST",
        f"/v1/text-to-speech/{quote(voice.voice_id, safe='')}/with-timestamps"
        f"?output_format={quote(voice.output_format)}",
        {"text": normalized(text), "model_id": voice.model_id, "voice_settings": voice.settings},
        opener,
    )
    try:
        pcm = base64.b64decode(answer["audio_base64"], validate=True)
    except (KeyError, TypeError, ValueError):
        raise SpeechError("ElevenLabs' answer has no audio") from None
    alignment = answer.get("alignment") or answer.get("normalized_alignment")
    if not alignment:
        raise SpeechError("ElevenLabs' answer has no timestamps (alignment)")
    if not pcm:
        raise SpeechError("ElevenLabs' answer has empty audio")
    return pcm, words_of(alignment)


def voice_name(voice_id: str, cache: Path, opener=urllib.request.urlopen) -> str:
    """The voice's name in the account (`GET /v1/voices/{id}`, costs no
    characters), kept in the cache; the id when it cannot be asked."""
    names = cache / "voice-names.json"
    try:
        known = json.loads(names.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        known = {}
    if voice_id in known:
        return known[voice_id]
    from urllib.parse import quote

    try:
        name = str(_call("GET", f"/v1/voices/{quote(voice_id, safe='')}", None, opener).get("name") or "")
    except SpeechError:
        return voice_id
    if not name:
        return voice_id
    known[voice_id] = name
    cache.mkdir(parents=True, exist_ok=True)
    names.write_text(json.dumps(known, indent=1, sort_keys=True), encoding="utf-8")
    return name


class Cache:
    """Spoken sections in `root/<key>/`: `audio.wav`, `words.json` and
    `meta.json` (the text, the voice, the characters, when last used)."""

    def __init__(self, root: Path):
        self.root = Path(root)

    def entry(self, key: str) -> Path:
        return self.root / key

    def get(self, key: str) -> tuple[Path, list[dict]] | None:
        """The audio and words of `key`, if whole; marked as used now."""
        d = self.entry(key)
        wav, words, meta = d / "audio.wav", d / "words.json", d / "meta.json"
        if not (wav.exists() and words.exists() and meta.exists()):
            return None
        try:
            said = json.loads(words.read_text(encoding="utf-8"))
        except ValueError:
            return None
        os.utime(meta)
        return wav, said

    def has(self, key: str) -> bool:
        d = self.entry(key)
        return all((d / f).exists() for f in ("audio.wav", "words.json", "meta.json"))

    def put(self, key: str, pcm: bytes, words: list[dict], meta: dict) -> Path:
        """Keeps a section: written into a folder of its own, then renamed
        into place whole."""
        self.root.mkdir(parents=True, exist_ok=True)
        tmp = self.root / f".{key}.{os.getpid()}.tmp"
        shutil.rmtree(tmp, ignore_errors=True)
        tmp.mkdir()
        try:
            with wave.open(str(tmp / "audio.wav"), "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(RATE)
                w.writeframes(pcm)
            (tmp / "words.json").write_text(json.dumps(words), encoding="utf-8")
            (tmp / "meta.json").write_text(json.dumps(meta, indent=1, ensure_ascii=False), encoding="utf-8")
            final = self.entry(key)
            shutil.rmtree(final, ignore_errors=True)
            os.replace(tmp, final)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
        return final / "audio.wav"

    def entries(self) -> list[Path]:
        if not self.root.exists():
            return []
        return [d for d in self.root.iterdir() if d.is_dir() and not d.name.startswith(".")]

    def prune(self, older_than_days: float, now: float | None = None) -> tuple[int, int]:
        """Removes entries not used for `older_than_days` days, and what a
        broken run left; how many and their bytes."""
        now = time.time() if now is None else now
        removed = size = 0
        for d in self.root.iterdir() if self.root.exists() else []:
            if not d.is_dir():
                continue
            meta = d / "meta.json"
            stale = d.name.startswith(".") or not meta.exists()
            if not stale and now - meta.stat().st_mtime < older_than_days * 86_400:
                continue
            size += sum(f.stat().st_size for f in d.rglob("*") if f.is_file())
            shutil.rmtree(d, ignore_errors=True)
            removed += 1
        return removed, size


def wav_seconds(path: Path) -> float:
    with wave.open(str(path), "rb") as w:
        return w.getnframes() / w.getframerate()


class Narrator:
    """Speaks sections with ElevenLabs through the cache, and counts: how
    many from the cache, how many spoken, how many characters sent (or,
    in a dry run, that would be)."""

    def __init__(self, voice: Voice, cache: Path, dry_run: bool = False, opener=urllib.request.urlopen):
        self.voice = voice
        self.cache = Cache(cache)
        self.dry_run = dry_run
        self.opener = opener
        self.cached = 0
        self.spoken = 0
        self.characters = 0

    def say(self, text: str) -> tuple[Path | None, float, list[dict]]:
        """`text`'s audio, seconds and words: from the cache, or asked for
        and kept. In a dry run, only counted (no audio, seconds guessed)."""
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
        pcm, words = synthesize(text, self.voice, self.opener)
        wav = self.cache.put(
            key,
            pcm,
            words,
            {
                "text": text,
                "voice_id": self.voice.voice_id,
                "model_id": self.voice.model_id,
                "settings": self.voice.settings,
                "output_format": self.voice.output_format,
                "characters": len(text),
                "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
            },
        )
        return wav, wav_seconds(wav), words

    def report(self) -> str:
        total = self.cached + self.spoken
        if self.dry_run:
            return (
                f"ElevenLabs (dry run): {total} sections, {self.cached} in the cache, "
                f"{self.spoken} to speak: {self.characters} characters would be sent"
            )
        return (
            f"ElevenLabs: {total} sections, {self.cached} from the cache, "
            f"{self.spoken} spoken: {self.characters} characters sent"
        )


def _main(argv: list[str]) -> None:
    import argparse

    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("command", choices=["voices", "prune", "stats"])
    p.add_argument("--cache", default="target/tutorials/voices", help="the cache folder")
    p.add_argument("--older-than", type=float, default=30, help="prune: days not used")
    a = p.parse_args(argv)
    try:
        if a.command == "voices":
            for v in _call("GET", "/v1/voices", None, urllib.request.urlopen).get("voices", []):
                print(f"{v.get('voice_id')}  {v.get('name')}  ({v.get('category', '')})")
        elif a.command == "prune":
            removed, size = Cache(Path(a.cache)).prune(a.older_than)
            print(f"{removed} entries removed, {size / 1e6:.1f} MB")
        else:
            entries = Cache(Path(a.cache)).entries()
            chars = 0
            for d in entries:
                try:
                    chars += json.loads((d / "meta.json").read_text(encoding="utf-8"))["characters"]
                except (OSError, ValueError, KeyError):
                    pass
            print(f"{len(entries)} sections, {chars} characters in {a.cache}")
    except SpeechError as e:
        sys.exit(f"elevenlabs: {e}")


if __name__ == "__main__":
    _main(sys.argv[1:])
