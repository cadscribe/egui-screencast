"""Tests of `lesson_voices.py` on egui-lessons' test tutorials, without
ElevenLabs: its answers are a fake opener's, the cache a temporary folder.

    python scripts/test_lesson_voices.py   (PyYAML)
"""

import base64
import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import lesson_voices  # noqa: E402

TUTORIALS = Path(__file__).resolve().parent.parent / "lessons" / "tests" / "tutorials"
COURSE = "tray"
VOICE = ["--voice", "elevenlabs:HzBCRGsOskP44taMg2CT", "--voice-model", "eleven_v3", "--voice-language", "de"]


class Answer:
    def __init__(self, body: bytes):
        self.body = body

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False

    def read(self) -> bytes:
        return self.body


class Fake:
    """ElevenLabs as a fake: each text answered with a little silence and
    one timed character per letter."""

    def __init__(self):
        self.asked: list[dict] = []

    def __call__(self, request, timeout=None):
        body = json.loads(request.data)
        self.asked.append(body)
        chars = list(body["text"])
        times = [k * 0.01 for k in range(len(chars))]
        answer = {
            "audio_base64": base64.b64encode(b"\0\0" * 240).decode(),
            "alignment": {
                "characters": chars,
                "character_start_times_seconds": times,
                "character_end_times_seconds": [t + 0.01 for t in times],
            },
        }
        return Answer(json.dumps(answer).encode())


def run(args: list[str], opener=None) -> str:
    out = io.StringIO()
    with redirect_stdout(out):
        code = lesson_voices.main(["--tutorials", str(TUTORIALS), *VOICE, *args], opener=opener)
    assert code == 0, out.getvalue()
    return out.getvalue()


class LessonVoices(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.cache = Path(self.tmp.name) / "voices"
        os.environ["ELEVENLABS_API_KEY"] = "fake"

    def tearDown(self):
        os.environ.pop("ELEVENLABS_API_KEY", None)
        self.tmp.cleanup()

    def test_say_lesson_is_the_lessons_language_only(self):
        step = {"say": "Watch.", "say_de": "Schau.", "say_lesson": "Follow along."}
        self.assertEqual(lesson_voices.lesson_only(step, "say_lesson", "en"), "Follow along.")
        self.assertIsNone(lesson_voices.lesson_only(step, "say_lesson", "de"))
        step = {"say_lesson": {"en": "Follow.", "de": " Mach mit. "}}
        self.assertEqual(lesson_voices.lesson_only(step, "say_lesson", "de"), "Mach mit.")
        self.assertEqual(
            lesson_voices.step_text({"say_lesson_de": "Mit."}, {"say": "Schau."}, "de"), ("Mit.", True)
        )
        self.assertEqual(lesson_voices.step_text({}, {"say": "", "text": "Text."}, "de"), ("Text.", False))
        self.assertEqual(lesson_voices.step_text({}, {"text": " "}, "de"), (None, False))

    def test_a_dry_run_sends_nothing_and_writes_nothing(self):
        fake = Fake()
        out = run(["--voice-cache", str(self.cache), "--courses", COURSE], opener=fake)
        self.assertEqual(fake.asked, [])
        self.assertFalse(self.cache.exists())
        self.assertIn(COURSE, out)
        self.assertIn("dry run", out)

    def test_speak_fills_the_cache_as_the_recordings_lay_it_out(self):
        fake = Fake()
        args = ["--voice-cache", str(self.cache), "--courses", COURSE]
        run([*args, "--speak"], opener=fake)
        self.assertTrue(fake.asked)
        # Each body as the recorder sends it: the model and the language told.
        self.assertTrue(all(b["model_id"] == "eleven_v3" and b["language_code"] == "de" for b in fake.asked))
        entries = [d for d in self.cache.iterdir() if d.is_dir()]
        self.assertEqual(len(entries), len(fake.asked))
        for d in entries:
            for f in ("audio.wav", "words.json", "meta.json"):
                self.assertTrue((d / f).is_file(), d / f)
        # The keys are the tool's: run again, nothing is missing or sent.
        again = Fake()
        out = run([*args, "--speak"], opener=again)
        self.assertEqual(again.asked, [])
        self.assertIn("0 sections spoken", out)
        # One of them is the lesson's own ending.
        texts = [b["text"] for b in fake.asked]
        self.assertTrue(any("Danke fürs Mitmachen" in t for t in texts), texts)


    def test_english_is_the_recordings_kokoro(self):
        work = Path(self.tmp.name) / "audio"
        spoken = []

        def speak(s, voice, speed, cache):
            spoken.append((s.key, voice, speed))
            (cache / s.key).mkdir(parents=True)
            (cache / s.key / "audio.wav").write_bytes(b"RIFF")

        args = ["--tutorials", str(TUTORIALS), "--lang", "en", "--voice-cache", str(self.cache),
                "--work-audio", str(work), "--courses", COURSE]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(lesson_voices.main(args, speak=speak), 0)
        self.assertIn("Kokoro", out.getvalue())
        self.assertEqual(spoken, [], "a dry run speaks nothing")
        with redirect_stdout(io.StringIO()):
            self.assertEqual(lesson_voices.main([*args, "--speak"], speak=speak), 0)
        self.assertTrue(spoken)
        # The script's own Kokoro voice and speed.
        self.assertTrue(all(v == "bf_emma" and sp == 1.1 and len(k) == 16 for k, v, sp in spoken), spoken)
        # Run again: nothing is missing.
        again = []
        with redirect_stdout(io.StringIO()):
            lesson_voices.main([*args, "--speak"], speak=lambda s, *_: again.append(s.key))
        self.assertEqual(again, [])

    def test_the_cache_and_tutorials_are_required(self):
        with self.assertRaises(SystemExit), redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            lesson_voices.main(["--tutorials", str(TUTORIALS), *VOICE])
        with self.assertRaises(SystemExit) as e, redirect_stdout(io.StringIO()):
            lesson_voices.main(["--tutorials", str(TUTORIALS), "--voice-cache", str(self.cache), *VOICE,
                                "--courses", "nothing"])
        self.assertIn("no course nothing", str(e.exception.code))


if __name__ == "__main__":
    unittest.main()
