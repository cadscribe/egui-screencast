"""Tests of Fish Audio S2 Pro's narration (run locally), its cache and the
forced alignment of its words, with a fake worker: no model is loaded or
downloaded here.

    python3 -m unittest discover scripts
"""

import json
import os
import sys
import tempfile
import textwrap
import unittest
import wave
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))

import align  # noqa: E402
import elevenlabs  # noqa: E402
import fishspeech  # noqa: E402
import tutorial  # noqa: E402


def write_wav(path: Path, seconds: float) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(24_000)
        w.writeframes(b"\0\0" * round(seconds * 24_000))


class FakeWorker:
    """Speaks a tenth of a second a character, its words by length."""

    def __init__(self, error: str | None = None):
        self.error = error
        self.requests: list[dict] = []

    def __call__(self, request: dict) -> dict:
        self.requests.append(request)
        if self.error:
            raise elevenlabs.SpeechError(f"Fish S2 Pro: {self.error}")
        seconds = len(request["text"]) / 10
        write_wav(Path(request["out"]), seconds)
        return {"ok": True, "seconds": seconds, "words": align.by_length(request["text"], seconds)}


class Alignment(unittest.TestCase):
    VOCAB = {"<pad>": 0, "|": 1, "H": 2, "I": 3, "Y": 4, "O": 5, "T": 6, "W": 7, "E": 8, "N": 9}

    def emission(self, frames: str) -> list[list[float]]:
        """A frame a character of `frames` ("_" a blank), that one likely."""
        out = []
        for c in frames:
            idx = 0 if c == "_" else self.VOCAB[c]
            out.append([0.0 if k == idx else -10.0 for k in range(len(self.VOCAB))])
        return out

    def test_tokens_are_words_and_marks(self):
        self.assertEqual(
            [(t["text"], t["space"]) for t in align.tokens_of("Open it, then 3.5 mm. It's done!")],
            [("Open", True), ("it", False), (",", True), ("then", True), ("3.5", True), ("mm", False),
             (".", True), ("It's", True), ("done", False), ("!", False)],
        )

    def test_numbers_are_spelled(self):
        self.assertEqual(align.spelled("M3"), "m three")
        self.assertEqual(align.spelled("3.5", lambda n: {"3.5": "three point five"}[n]), "three point five")
        self.assertEqual(align.spelled(","), "")

    def test_each_word_gets_the_frames_of_its_letters(self):
        frames = "__HHI_|_YOO__"
        said = align.align("Hi, yo.", self.emission(frames), self.VOCAB, 0, "|", len(frames) * 0.02)
        self.assertEqual([w["text"] for w in said], ["Hi", ",", "yo", "."])
        hi, comma, yo, stop = said
        self.assertEqual((hi["start"], hi["end"]), (0.04, 0.1))
        self.assertEqual((yo["start"], yo["end"]), (0.16, 0.22))
        # A mark where the word before it ends.
        self.assertEqual((comma["start"], comma["end"]), (0.1, 0.1))
        self.assertTrue(comma["space"])

    def test_cues_and_subtitles_work_on_its_words(self):
        frames = "_HI_|_YO_|_TWO_"
        said = align.align("Hi. Yo two.", self.emission(frames), self.VOCAB, 0, "|", 0.3)
        step = {"name": "s", "actions": [{"cue": "yo two"}]}
        self.assertEqual(tutorial.cue_actions("t", 0, step, said)[0]["cue"], 0.12)
        self.assertEqual([t for _, _, t in tutorial.subtitles_of(said, [])], ["Hi.", "Yo two."])

    def test_too_few_frames_fall_back_to_the_words_lengths(self):
        said = align.align("Hi yo", self.emission("_H_"), self.VOCAB, 0, "|", 1.0)
        self.assertEqual([w["text"] for w in said], ["Hi", "yo"])
        self.assertEqual(said[0]["start"], 0.0)
        self.assertAlmostEqual(said[1]["end"], 1.0, places=3)


class Narration(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        self.ref = self.dir / "narrator.wav"
        write_wav(self.ref, 1.0)
        self.ref.with_suffix(".lab").write_text("I am the narrator.", encoding="utf-8")
        self.cache = self.dir / "cache"

    def narrator(self, worker, voice=None, dry_run=False):
        return fishspeech.Narrator(voice or fishspeech.Voice(self.ref), self.cache, worker, dry_run)

    def test_a_section_is_spoken_once_then_kept(self):
        worker = FakeWorker()
        n = self.narrator(worker)
        wav, seconds, words = n.say("Open it.")
        self.assertAlmostEqual(seconds, 0.8)
        self.assertEqual([w["text"] for w in words], ["Open", "it", "."])
        request = worker.requests[0]
        self.assertEqual(request["reference_text"], "I am the narrator.")
        self.assertEqual(request["seed"], 42)
        self.assertFalse(Path(request["out"]).exists())  # moved into the cache
        again = self.narrator(worker)
        self.assertEqual(again.say("Open it.")[0], wav)
        self.assertEqual(len(worker.requests), 1)
        self.assertIn("1 from the cache, 0 spoken", again.report())

    def test_the_key_changes_with_what_makes_the_sound(self):
        voice = fishspeech.Voice(self.ref)
        k = voice.key("Open it.")
        self.assertEqual(k, fishspeech.Voice(self.ref).key(" Open  it."))
        self.assertNotEqual(k, voice.key("Open it!"))
        self.assertNotEqual(k, fishspeech.Voice(self.ref, settings=dict(fishspeech.SETTINGS, seed=7)).key("Open it."))
        self.assertNotEqual(k, fishspeech.Voice(self.ref, "Another transcript.").key("Open it."))
        write_wav(self.ref, 1.5)  # another recording
        self.assertNotEqual(k, fishspeech.Voice(self.ref).key("Open it."))

    def test_a_dry_run_counts_and_speaks_nothing(self):
        worker = FakeWorker()
        n = self.narrator(worker, dry_run=True)
        n.say("Open it.")
        self.assertEqual(worker.requests, [])
        self.assertIn("1 to speak (8 characters)", n.report())

    def test_a_failure_keeps_nothing(self):
        with self.assertRaises(elevenlabs.SpeechError):
            self.narrator(FakeWorker("out of memory")).say("Open it.")
        self.assertEqual(elevenlabs.Cache(self.cache).entries(), [])
        self.assertEqual(list(self.cache.glob("*.wav")), [])

    def test_a_reference_and_its_words_are_needed(self):
        with self.assertRaises(elevenlabs.SpeechError) as e:
            fishspeech.Voice(self.dir / "missing.wav")
        self.assertIn("reference recording", str(e.exception))
        self.ref.with_suffix(".lab").unlink()
        with self.assertRaises(elevenlabs.SpeechError) as e:
            fishspeech.Voice(self.ref)
        self.assertIn(".lab", str(e.exception))

    def test_no_venv_says_how_to_make_it(self):
        with self.assertRaises(elevenlabs.SpeechError) as e:
            fishspeech.Worker(self.dir / "no-venv" / "bin" / "python")({"text": "x"})
        self.assertIn("requirements-fish.txt", str(e.exception))

    def test_the_worker_answers_a_line_a_request(self):
        fake = self.dir / "worker.py"
        fake.write_text(
            textwrap.dedent(
                """
                import json, sys, wave
                for line in sys.stdin:
                    r = json.loads(line)
                    if r["text"] == "fail":
                        print(json.dumps({"ok": False, "error": "RuntimeError: no audio"}), flush=True)
                        continue
                    with wave.open(r["out"], "wb") as w:
                        w.setnchannels(1); w.setsampwidth(2); w.setframerate(24000)
                        w.writeframes(b"\\0\\0" * 2400)
                    words = [{"text": r["text"], "start": 0.0, "end": 0.1, "space": False}]
                    print(json.dumps({"ok": True, "seconds": 0.1, "words": words}), flush=True)
                """
            ),
            encoding="utf-8",
        )
        worker = fishspeech.Worker(Path(sys.executable), command=[sys.executable, str(fake)])
        n = self.narrator(worker)
        self.assertAlmostEqual(n.say("Hello")[1], 0.1)
        with self.assertRaises(elevenlabs.SpeechError) as e:
            n.say("fail")
        self.assertIn("no audio", str(e.exception))
        self.assertEqual(len(elevenlabs.Cache(self.cache).entries()), 1)
        n.close()

    def test_the_notices_name_fish_audio_and_its_licence(self):
        with mock.patch.object(tutorial, "VOICE", None), mock.patch.dict(os.environ, {"SCREENCAST_VOICE": ""}):
            self.assertEqual(tutorial.voice_choice({"voice": "fishaudio-s2-pro:x.wav"}), ("fish-s2-pro", "x.wav"))
        patches = [
            mock.patch.object(tutorial, "VOICE", f"fish-s2-pro:{self.ref}"),
            mock.patch.object(tutorial, "VOICE_SETTINGS", {}),
            mock.patch.object(tutorial, "VOICE_CACHE", self.cache),
        ]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)
        speaker = tutorial.speaker_of({"voice": "am_michael"})
        self.assertEqual(speaker["backend"], "fish-s2-pro")
        self.assertEqual(tutorial.voice_notice({}, speaker), "Voice: AI-generated (Fish Audio S2 Pro, narrator)")
        self.assertIn("non-commercial", speaker["track"])
        self.assertIn("Fish Audio Research License", speaker["made_by_md"])


if __name__ == "__main__":
    unittest.main()
