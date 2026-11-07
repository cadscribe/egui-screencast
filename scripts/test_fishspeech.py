"""Tests of Fish Audio S2 Pro's narration (run locally), its cache and the
forced alignment of its words, with a fake worker: no model is loaded or
downloaded here.

    python -m unittest discover scripts   # with the venv's Python (PyYAML)
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
    """Speaks a tenth of a second a character."""

    def __init__(self, error: str | None = None):
        self.error = error
        self.requests: list[dict] = []

    def __call__(self, request: dict) -> dict:
        self.requests.append(request)
        if self.error:
            raise elevenlabs.SpeechError(f"Fish S2 Pro: {self.error}")
        seconds = len(request["text"]) / 10
        write_wav(Path(request["out"]), seconds)
        return {"ok": True, "seconds": seconds}


class FakeAligner:
    """The words by their lengths, as the aligner would without a model."""

    def __init__(self):
        self.texts: list[str] = []

    def words(self, text, wav):
        self.texts.append(text)
        return align.by_length(align.untagged(text), elevenlabs.wav_seconds(wav))


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
        voices = self.dir / "voices"
        (voices / "fish-s2-pro").mkdir(parents=True)
        for v in ("en-gb-female", "de-male"):
            write_wav(voices / "fish-s2-pro" / f"{v}.wav", 1.0)
        (voices / "voices.yaml").write_text(
            textwrap.dedent(
                """
                dir: fish-s2-pro
                voices:
                  en-gb-female: {language: English, reference: en-gb-female.wav, transcript: "In this tutorial."}
                  de-male: {language: German, reference: de-male.wav, transcript: "In diesem Tutorial."}
                """
            ),
            encoding="utf-8",
        )
        self.voices = voices
        self.cache = self.dir / "cache"

    def voice(self, name="en-gb-female", **settings):
        return fishspeech.voice_of(name, self.voices, self.dir, settings)

    def narrator(self, worker, voice=None, dry_run=False, aligner=None):
        return fishspeech.Narrator(voice or self.voice(), self.cache, worker, dry_run, aligner or FakeAligner())

    def test_a_voice_comes_from_voices_yaml(self):
        en, de = self.voice(), self.voice("de-male")
        self.assertEqual(en.transcript, "In this tutorial.")
        self.assertIsNone(en.generation_language)
        self.assertEqual(de.generation_language, "German")
        with self.assertRaises(elevenlabs.SpeechError) as e:
            self.voice("nobody")
        self.assertIn("de-male, en-gb-female", str(e.exception))
        with self.assertRaises(elevenlabs.SpeechError) as e:
            fishspeech.voice_of("en-gb-female", None, self.dir, {})
        self.assertIn("--voices", str(e.exception))

    def test_a_section_is_spoken_once_then_kept(self):
        worker, aligner = FakeWorker(), FakeAligner()
        n = self.narrator(worker, aligner=aligner)
        wav, seconds, words = n.say("[excited] Open it.")
        self.assertAlmostEqual(seconds, 1.8)
        # The tags go to S2 Pro, not into the words.
        self.assertEqual(worker.requests[0]["text"], "[excited] Open it.")
        self.assertEqual([w["text"] for w in words], ["Open", "it", "."])
        request = worker.requests[0]
        self.assertEqual(request["reference_text"], "In this tutorial.")
        self.assertEqual(request["seed"], 1)
        self.assertIsNone(request["language"])
        self.assertFalse(Path(request["out"]).exists())  # moved into the cache
        again = self.narrator(worker)
        self.assertEqual(again.say("[excited] Open it.")[0], wav)
        self.assertEqual(len(worker.requests), 1)
        self.assertIn("1 from the cache, 0 spoken", again.report())

    def test_german_is_said_to_be_german(self):
        worker = FakeWorker()
        self.narrator(worker, self.voice("de-male")).say("Öffne sie.")
        self.assertEqual(worker.requests[0]["language"], "German")

    def test_the_key_changes_with_what_makes_the_sound(self):
        k = self.voice().key("Open it.")
        self.assertEqual(k, self.voice().key(" Open  it."))
        self.assertNotEqual(k, self.voice().key("Open it!"))
        self.assertNotEqual(k, self.voice().key("[pause] Open it."))
        self.assertNotEqual(k, self.voice(seed=7).key("Open it."))
        self.assertNotEqual(k, self.voice("de-male").key("Open it."))
        self.assertNotEqual(k, self.voice(language="German").key("Open it."))
        write_wav(self.voices / "fish-s2-pro" / "en-gb-female.wav", 1.5)  # another recording
        self.assertNotEqual(k, self.voice().key("Open it."))

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

    def test_a_wav_with_its_words_is_a_voice_too(self):
        ref = self.dir / "narrator.wav"
        write_wav(ref, 1.0)
        with self.assertRaises(elevenlabs.SpeechError) as e:
            fishspeech.voice_of("narrator.wav", None, self.dir, {})
        self.assertIn(".lab", str(e.exception))
        ref.with_suffix(".lab").write_text("I am the narrator.", encoding="utf-8")
        self.assertEqual(fishspeech.voice_of("narrator.wav", None, self.dir, {}).transcript, "I am the narrator.")

    def test_no_mlx_speech_says_how_to_install_it(self):
        with self.assertRaises(elevenlabs.SpeechError) as e:
            fishspeech.Worker(self.dir / "none" / "python")({"text": "x"})
        self.assertIn("uv tool install mlx-speech", str(e.exception))

    def test_the_worker_answers_a_line_a_request_offline(self):
        fake = self.dir / "worker.py"
        fake.write_text(
            textwrap.dedent(
                """
                import json, os, sys, wave
                for line in sys.stdin:
                    r = json.loads(line)
                    if r["text"] == "fail" or os.environ.get("HF_HUB_OFFLINE") != "1":
                        print(json.dumps({"ok": False, "error": "RuntimeError: no audio"}), flush=True)
                        continue
                    with wave.open(r["out"], "wb") as w:
                        w.setnchannels(1); w.setsampwidth(2); w.setframerate(24000)
                        w.writeframes(b"\\0\\0" * 2400)
                    print(json.dumps({"ok": True, "seconds": 0.1}), flush=True)
                """
            ),
            encoding="utf-8",
        )
        worker = fishspeech.Worker(None, {"HF_HOME": None}, command=[sys.executable, str(fake)])
        n = self.narrator(worker)
        self.assertAlmostEqual(n.say("Hello")[1], 0.1)
        with self.assertRaises(elevenlabs.SpeechError) as e:
            n.say("fail")
        self.assertIn("no audio", str(e.exception))
        self.assertEqual(len(elevenlabs.Cache(self.cache).entries()), 1)
        n.close()


class Choice(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        for p in (
            mock.patch.object(tutorial, "VOICE", None),
            mock.patch.object(tutorial, "ENGINE", None),
            mock.patch.object(tutorial, "FAST", False),
            mock.patch.object(tutorial, "LANG", "en"),
            mock.patch.object(tutorial, "VOICE_SETTINGS", {}),
            mock.patch.object(tutorial, "VOICE_CACHE", Path(self.tmp.name)),
            mock.patch.dict(os.environ, {"SCREENCAST_VOICE": "", "SCREENCAST_ENGINE": ""}),
        ):
            p.start()
            self.addCleanup(p.stop)

    def test_fish_by_the_language_is_the_default(self):
        self.assertEqual(tutorial.voice_choice({}), ("fish-s2-pro", "en-gb-female"))
        # A script's Kokoro voice is Kokoro's only.
        self.assertEqual(tutorial.voice_choice({"voice": "am_michael"}), ("fish-s2-pro", "en-gb-female"))
        with mock.patch.object(tutorial, "LANG", "de"):
            self.assertEqual(tutorial.voice_choice({}), ("fish-s2-pro", "de-male"))
        self.assertEqual(tutorial.voice_choice({"voice": "en-us-male"}), ("fish-s2-pro", "en-us-male"))
        self.assertEqual(tutorial.voice_choice({"voice": "fishaudio-s2-pro:x.wav"}), ("fish-s2-pro", "x.wav"))

    def test_fast_is_kokoro_and_kokoro_speaks_english_only(self):
        with mock.patch.object(tutorial, "FAST", True):
            self.assertEqual(tutorial.voice_choice({"voice": "am_michael"}), ("kokoro", "am_michael"))
            self.assertEqual(tutorial.voice_choice({"voice": "en-gb-female"}), ("kokoro", "am_michael"))
            with mock.patch.object(tutorial, "LANG", "de"), self.assertRaises(SystemExit):
                tutorial.speaker_of({})
        self.assertEqual(tutorial.voice_choice({"engine": "kokoro", "voice": "bf_emma"}), ("kokoro", "bf_emma"))

    def test_no_mlx_speech_names_the_install_and_fast(self):
        voices = Path(self.tmp.name) / "v"
        (voices / "fish-s2-pro").mkdir(parents=True)
        write_wav(voices / "fish-s2-pro" / "a.wav", 1.0)
        (voices / "voices.yaml").write_text(
            'dir: fish-s2-pro\nvoices:\n  en-gb-female: {language: English, reference: a.wav, transcript: "Hi."}\n',
            encoding="utf-8",
        )
        with mock.patch.object(tutorial, "VOICES", str(voices)), mock.patch.object(
            tutorial, "FISH_PYTHON", Path(self.tmp.name) / "nothing"
        ), self.assertRaises(SystemExit) as e:
            tutorial.speaker_of({})
        self.assertIn("uv tool install --python 3.13 mlx-speech", str(e.exception))
        self.assertIn("--fast", str(e.exception))
        with mock.patch.object(tutorial, "VOICES", str(voices)), mock.patch.object(
            tutorial, "FISH_PYTHON", Path(sys.executable)
        ):
            speaker = tutorial.speaker_of({})
        self.assertEqual(tutorial.voice_notice({}, speaker), "Voice: AI-generated (Fish Audio S2 Pro, en-gb-female)")
        self.assertIn("non-commercial", speaker["track"])
        self.assertIn("Fish Audio Research License", speaker["made_by_md"])
        with mock.patch.object(tutorial, "LANG", "de"):
            self.assertEqual(
                tutorial.voice_notice({}, speaker), "Stimme: KI-generiert (Fish Audio S2 Pro, en-gb-female)"
            )


if __name__ == "__main__":
    unittest.main()
