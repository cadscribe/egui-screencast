"""Tests of the ElevenLabs narration and its cache, against a fake API:
nothing here asks the real ElevenLabs (each call costs money).

    python -m unittest discover scripts   # with the venv's Python (PyYAML)
"""

import base64
import io
import json
import os
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))

import elevenlabs  # noqa: E402
import tutorial  # noqa: E402

KEY = "test-key-not-real"


def alignment(text: str, per_char: float = 0.05) -> dict:
    return {
        "characters": list(text),
        "character_start_times_seconds": [round(i * per_char, 3) for i in range(len(text))],
        "character_end_times_seconds": [round((i + 1) * per_char, 3) for i in range(len(text))],
    }


class Response(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


class FakeAPI:
    """Answers like ElevenLabs: a tenth of a second of quiet a character,
    the characters' times; or, when `status` is set, that error."""

    def __init__(self, status: int | None = None, name: str = "Rachel"):
        self.status = status
        self.name = name
        self.requests: list = []

    def __call__(self, request, timeout=None):
        self.requests.append(request)
        if self.status:
            body = json.dumps({"detail": {"status": "error", "message": f"fake {self.status}"}}).encode()
            raise urllib.error.HTTPError(request.full_url, self.status, "error", {}, io.BytesIO(body))
        if request.get_method() == "GET":
            return Response(json.dumps({"voice_id": "v1", "name": self.name}).encode())
        text = json.loads(request.data)["text"]
        pcm = b"\0\0" * (2400 * len(text))
        return Response(
            json.dumps(
                {"audio_base64": base64.b64encode(pcm).decode(), "alignment": alignment(text), "normalized_alignment": None}
            ).encode()
        )

    def posts(self) -> list[dict]:
        return [json.loads(r.data) for r in self.requests if r.get_method() == "POST"]


class Words(unittest.TestCase):
    def test_characters_become_words_and_marks(self):
        said = elevenlabs.words_of(alignment("Open it, then type 3.5 mm. It's done!"))
        self.assertEqual(
            [w["text"] for w in said],
            ["Open", "it", ",", "then", "type", "3.5", "mm", ".", "It's", "done", "!"],
        )
        self.assertEqual([w["space"] for w in said][:3], [True, False, True])
        # "it" is the 6th and 7th character.
        self.assertEqual((said[1]["start"], said[1]["end"]), (0.25, 0.35))

    def test_cues_and_subtitles_work_on_its_words(self):
        said = elevenlabs.words_of(alignment("Choose the case. Open it now."))
        step = {"name": "s", "actions": [{"cue": "Open it"}]}
        self.assertEqual(tutorial.cue_actions("t", 0, step, said)[0]["cue"], 0.85)
        subs = tutorial.subtitles_of(said, [])
        self.assertEqual([t for _, _, t in subs], ["Choose the case.", "Open it now."])

    def test_unequal_lists_are_an_error(self):
        with self.assertRaises(elevenlabs.SpeechError):
            elevenlabs.words_of({"characters": ["a"], "character_start_times_seconds": [], "character_end_times_seconds": []})


class Key(unittest.TestCase):
    def test_it_changes_with_everything_that_makes_the_sound(self):
        base = elevenlabs.Voice("v1")
        k = base.key("Open it.")
        self.assertEqual(k, elevenlabs.Voice("v1").key("Open  it. "))  # whitespace is not text
        self.assertNotEqual(k, base.key("Open it!"))
        self.assertNotEqual(k, elevenlabs.Voice("v2").key("Open it."))
        self.assertNotEqual(k, elevenlabs.Voice("v1", model_id="eleven_v3").key("Open it."))
        self.assertNotEqual(k, elevenlabs.Voice("v1", output_format="pcm_16000").key("Open it."))
        other = dict(elevenlabs.SETTINGS, stability=0.3)
        self.assertNotEqual(k, elevenlabs.Voice("v1", settings=other).key("Open it."))


class Narration(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.cache = Path(self.tmp.name) / "cache"
        env = mock.patch.dict(os.environ, {elevenlabs.KEY_ENV: KEY})
        env.start()
        self.addCleanup(env.stop)

    def tearDown(self):
        self.tmp.cleanup()

    def narrator(self, api, voice=None, dry_run=False):
        return elevenlabs.Narrator(voice or elevenlabs.Voice("v1"), self.cache, dry_run, api)

    def test_a_section_is_asked_for_once_then_kept(self):
        api = FakeAPI()
        n = self.narrator(api)
        wav, seconds, words = n.say("Open it.")
        self.assertAlmostEqual(seconds, 0.8)
        self.assertEqual([w["text"] for w in words], ["Open", "it", "."])
        self.assertEqual((n.cached, n.spoken, n.characters), (0, 1, 8))
        request = api.requests[0]
        self.assertEqual(request.get_header("Xi-api-key"), KEY)
        self.assertIn("/v1/text-to-speech/v1/with-timestamps?output_format=pcm_24000", request.full_url)
        self.assertEqual(api.posts()[0]["model_id"], elevenlabs.MODEL)

        again = self.narrator(api)
        self.assertEqual(again.say("Open it.")[0], wav)
        self.assertEqual((again.cached, again.spoken, again.characters), (1, 0, 0))
        self.assertEqual(len(api.requests), 1)
        self.assertIn("1 from the cache, 0 spoken: 0 characters sent", again.report())

    def test_only_changed_sections_are_sent(self):
        api = FakeAPI()
        self.narrator(api).say("One.")
        n = self.narrator(api)
        n.say("One.")
        n.say("Two!")
        self.assertEqual([p["text"] for p in api.posts()], ["One.", "Two!"])
        self.assertEqual((n.cached, n.spoken, n.characters), (1, 1, 4))

    def test_another_voice_or_setting_is_asked_for_again(self):
        api = FakeAPI()
        self.narrator(api).say("One.")
        self.narrator(api, elevenlabs.Voice("v2")).say("One.")
        self.narrator(api, elevenlabs.Voice("v1", settings=dict(elevenlabs.SETTINGS, style=0.3))).say("One.")
        self.assertEqual(len(api.posts()), 3)

    def test_a_dry_run_counts_and_asks_nothing(self):
        api = FakeAPI()
        self.narrator(api).say("One.")
        n = self.narrator(api, dry_run=True)
        n.say("One.")
        n.say("Two and three.")
        self.assertEqual(len(api.requests), 1)
        self.assertIn("1 in the cache, 1 to speak: 14 characters would be sent", n.report())

    def test_errors_say_what_to_do_and_keep_nothing(self):
        for status, words in ((401, "API key"), (429, "quota"), (500, "HTTP 500")):
            with self.subTest(status=status):
                with self.assertRaises(elevenlabs.SpeechError) as e:
                    self.narrator(FakeAPI(status)).say("One.")
                self.assertIn(words, str(e.exception))
                self.assertNotIn(KEY, str(e.exception))
                self.assertEqual(elevenlabs.Cache(self.cache).entries(), [])

    def test_no_key_no_request(self):
        api = FakeAPI()
        with mock.patch.dict(os.environ, {elevenlabs.KEY_ENV: ""}):
            with self.assertRaises(elevenlabs.SpeechError) as e:
                self.narrator(api).say("One.")
        self.assertIn("ELEVENLABS_API_KEY is not set", str(e.exception))
        self.assertEqual(api.requests, [])

    def test_a_broken_answer_keeps_nothing(self):
        def broken(request, timeout=None):
            return Response(json.dumps({"audio_base64": "AAAA", "alignment": None}).encode())

        with self.assertRaises(elevenlabs.SpeechError):
            self.narrator(broken).say("One.")
        self.assertEqual(elevenlabs.Cache(self.cache).entries(), [])

    def test_prune_removes_what_was_not_used(self):
        api = FakeAPI()
        n = self.narrator(api)
        n.say("Old.")
        n.say("New.")
        cache = elevenlabs.Cache(self.cache)
        old = cache.entry(n.voice.key("Old.")) / "meta.json"
        os.utime(old, (0, 0))
        (self.cache / ".half.123.tmp").mkdir()
        removed, _ = cache.prune(30)
        self.assertEqual(removed, 2)
        self.assertEqual([d.name for d in cache.entries()], [n.voice.key("New.")])


class Choice(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        patches = [
            mock.patch.object(tutorial, "VOICE", None),
            mock.patch.object(tutorial, "ENGINE", None),
            mock.patch.object(tutorial, "FAST", False),
            mock.patch.object(tutorial, "LANG", "en"),
            mock.patch.object(tutorial, "VOICE_MODEL", None),
            mock.patch.object(tutorial, "VOICE_SETTINGS", {}),
            mock.patch.object(tutorial, "VOICE_CACHE", Path(self.tmp.name)),
            mock.patch.dict(os.environ, {elevenlabs.KEY_ENV: KEY}),
        ]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)
        os.environ.pop("SCREENCAST_VOICE", None)
        os.environ.pop("ELEVENLABS_MODEL", None)

    def tearDown(self):
        self.tmp.cleanup()

    def test_the_command_line_then_the_environment_then_the_script(self):
        script = {"voice": "am_michael"}
        with mock.patch.object(tutorial, "FAST", True):
            self.assertEqual(tutorial.voice_choice(script), ("kokoro", "am_michael"))
        with mock.patch.dict(os.environ, {"SCREENCAST_VOICE": "elevenlabs:env"}):
            self.assertEqual(tutorial.voice_choice(script), ("elevenlabs", "env"))
            with mock.patch.object(tutorial, "VOICE", "elevenlabs:cli"):
                self.assertEqual(tutorial.voice_choice(script), ("elevenlabs", "cli"))
        self.assertEqual(tutorial.voice_choice({"voice": "kokoro:af_heart"}), ("kokoro", "af_heart"))

    def test_the_notice_names_the_voice(self):
        self.assertEqual(tutorial.voice_notice({}), "Voice: AI-generated (Kokoro)")
        api = FakeAPI(name="Rachel")
        speaker = tutorial.speaker_of({"voice": "elevenlabs:v1"}, opener=api)
        self.assertEqual(tutorial.voice_notice({}, speaker), "Voice: AI-generated (ElevenLabs, Rachel)")
        self.assertIn("ElevenLabs", speaker["made_by"])
        self.assertIn("Rachel", speaker["track"])
        # The name is asked once, then kept.
        tutorial.speaker_of({"voice": "elevenlabs:v1"}, opener=api)
        self.assertEqual(len(api.requests), 1)
        german = {"voice_notice": "Stimme KI-generiert ({voice})"}
        self.assertEqual(tutorial.voice_notice(german, speaker), "Stimme KI-generiert (ElevenLabs, Rachel)")

    def test_settings_and_model_from_the_script_and_the_command_line(self):
        api = FakeAPI()
        script = {"voice": "elevenlabs:v1", "voice_name": "Narrator", "voice_model": "eleven_v3",
                  "speed": 1.1, "voice_settings": {"stability": 0.3}}
        with mock.patch.object(tutorial, "VOICE_SETTINGS", {"style": 0.2}):
            speaker = tutorial.speaker_of(script, opener=api)
        narrator = speaker["narrator"]()
        self.assertEqual(narrator.voice.model_id, "eleven_v3")
        self.assertEqual(narrator.voice.settings["stability"], 0.3)
        self.assertEqual(narrator.voice.settings["style"], 0.2)
        self.assertEqual(narrator.voice.settings["speed"], 1.1)
        self.assertEqual(speaker["label"], "ElevenLabs, Narrator")
        self.assertEqual(api.requests, [])  # the name was given: nothing asked


if __name__ == "__main__":
    unittest.main()
