"""Tests of the runner's timing: cues, the words they move, subtitles.

    python -m unittest discover scripts   # with the venv's Python (PyYAML)
"""

import sys
import tempfile
import unittest
import wave
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))

import tutorial  # noqa: E402


def words(text: str, gap: float = 0.5) -> list[dict]:
    """`text`'s words and marks, one each `gap` seconds."""
    out = []
    for i, w in enumerate(text.replace(",", " ,").replace(".", " .").split()):
        out.append({"text": w, "start": i * gap, "end": (i + 1) * gap, "space": True})
    for k, w in enumerate(out[:-1]):
        if out[k + 1]["text"] in (",", "."):
            w["space"] = False
    return out


class Cues(unittest.TestCase):
    def test_a_cue_is_when_its_words_are_said(self):
        said = words("The switcher says what you work on. Open it. It lists the parts.")
        step = {"name": "s", "actions": [{"cue": "Open it"}, {"click": {"spot": "Switcher"}}]}
        actions = tutorial.cue_actions("t", 0, step, said)
        # "Open" is the ninth mark: The switcher says what you work on . Open
        self.assertEqual(actions[0], {"cue": 4.0, "words": "Open it"})
        self.assertEqual(actions[1], {"click": {"spot": "Switcher"}})

    def test_cues_follow_the_narration_and_units_are_spoken(self):
        said = words("Type 8 as its depth: 8 millimeters high. Then 8 millimeters again.")
        step = {"name": "s", "actions": [{"cue": "8 mm high"}, {"cue": "8 mm"}]}
        actions = tutorial.cue_actions("t", 0, step, said)
        self.assertEqual(actions[0]["cue"], 2.5)
        # The second "8 mm" after the first cue, not the first one again.
        self.assertEqual(actions[1]["cue"], 5.0)

    def test_a_cue_not_said_stops_the_build(self):
        step = {"name": "s", "actions": [{"cue": "Close it"}]}
        with self.assertRaises(SystemExit):
            tutorial.cue_actions("t", 0, step, words("Open it."))


class Timing(unittest.TestCase):
    def test_the_words_after_a_late_cue_wait(self):
        planned = [{"wait": 1}, {"cue": 1.0}, {"cue": 3.0}]
        # Reached at frame 45 (1.5 s) and 105 (3.5 s): half a second late.
        cues = tutorial.shifts(planned, {"cues": [45, 105]}, 30)
        self.assertEqual(cues, [(1.0, 0.5), (3.0, 0.5)])
        self.assertEqual(tutorial.late_at(cues, 0.5), 0.0)
        self.assertEqual(tutorial.late_at(cues, 1.0), 0.5)

    def test_subtitles_are_what_is_said_when_it_is_said(self):
        said = words("Choose the case. Now you work on it.")
        subs = tutorial.subtitles_of(said, [])
        self.assertEqual([t for _, _, t in subs], ["Choose the case.", "Now you work on it."])
        self.assertEqual(subs[0][0], 0.0)
        self.assertEqual(subs[1][0], 2.0)

    def test_a_subtitle_ends_where_a_cue_moves_the_words(self):
        said = words("Click the line and type its name.")
        # "type" at 2.0 s comes half a second late.
        subs = tutorial.subtitles_of(said, [(2.0, 0.5)])
        self.assertEqual([t for _, _, t in subs], ["Click the line and", "type its name."])
        self.assertEqual(subs[1][0], 2.5)


class FakeNarrator:
    """Says a text as a word every half second of quiet, into `folder`."""

    def __init__(self, folder: Path):
        self.folder = folder
        self.said: list[str] = []

    def say(self, text: str):
        self.said.append(text)
        said = words(text)
        wav = self.folder / f"{len(self.said)}.wav"
        with wave.open(str(wav), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(24_000)
            w.writeframes(b"\0\0" * round(said[-1]["end"] * 24_000))
        return wav, said[-1]["end"], said

    def report(self) -> str:
        return "fake"


class VoiceIntro(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        for p in (
            mock.patch.object(tutorial, "WORK", Path(self.tmp.name)),
            mock.patch.object(tutorial, "VOICE_INTRO", tutorial.VOICE_INTRO_DEFAULT),
            mock.patch.object(tutorial, "APP_NAME", "OpenSolve"),
        ):
            p.start()
            self.addCleanup(p.stop)
        self.script = {
            "title": "T",
            "steps": [
                {"name": "a", "say": "It can be done. Open it.", "actions": [{"cue": "it can"}, {"cue": "Open it"}]},
                {"name": "b", "say": "Next."},
            ],
        }

    def test_the_voice_says_first_that_it_is_ai_and_why(self):
        narrator = FakeNarrator(Path(self.tmp.name))
        audio = tutorial.narrate("t", self.script, True, narrator=narrator)
        intro = "This video is narrated by an AI voice, so it can always show the current version of OpenSolve."
        # Spoken (and so kept in the cache) as a section of its own.
        self.assertIn(intro, narrator.said)
        said = [w["text"] for w in audio[0]["words"]]
        self.assertEqual(said[: audio[0]["lead"]], [w["text"] for w in words(intro)])
        self.assertEqual(said[audio[0]["lead"] :], ["It", "can", "be", "done", ".", "Open", "it", "."])
        # Its seconds and its quiet before the first step's words.
        shift = words(intro)[-1]["end"] + tutorial.INTRO_PAUSE
        self.assertAlmostEqual(audio[0]["seconds"], shift + 4.0)
        with wave.open(str(audio[0]["wav"])) as w:
            self.assertAlmostEqual(w.getnframes() / 24_000, shift + 4.0, places=3)
        # The other steps are as they were.
        self.assertEqual(audio[1]["lead"], 0)
        self.assertEqual(audio[1]["words"][0]["start"], 0.0)

    def test_its_words_do_not_move_the_cues(self):
        audio = tutorial.narrate("t", self.script, True, narrator=FakeNarrator(Path(self.tmp.name)))
        step = self.script["steps"][0]
        actions = tutorial.cue_actions("t", 0, step, audio[0]["words"], audio[0]["lead"])
        shift = audio[0]["words"][audio[0]["lead"]]["start"]
        # "it can" of the step, not of the notice; as late as the notice is long.
        self.assertEqual(actions[0]["cue"], shift)
        self.assertEqual(actions[1]["cue"], round(shift + 2.5, 3))
        planned = tutorial.plan("t", self.script, audio, False, True)
        self.assertEqual(planned["steps"][0]["actions"][0]["cue"], shift)

    def test_its_wording_and_off_only_when_said(self):
        self.assertEqual(tutorial.voice_intro({"app": "Foo"}).split()[-1], "Foo.")
        self.assertEqual(tutorial.voice_intro({"voice_intro": "Eine KI-Stimme spricht, für {app}."}),
                         "Eine KI-Stimme spricht, für OpenSolve.")
        self.assertIsNone(tutorial.voice_intro({"voice_intro": False}))
        with mock.patch.object(tutorial, "VOICE_INTRO", False):
            self.assertIsNone(tutorial.voice_intro({}))
            self.assertIsNone(tutorial.voice_intro({"voice_intro": "Said anyway?"}))
        narrator = FakeNarrator(Path(self.tmp.name))
        audio = tutorial.narrate("t", dict(self.script, voice_intro=False), True, narrator=narrator)
        self.assertEqual(audio[0]["lead"], 0)
        self.assertEqual(len(narrator.said), 2)

    def test_no_voice_no_intro(self):
        audio = tutorial.narrate("t", self.script, False)
        self.assertEqual(audio[0]["lead"], 0)
        self.assertEqual(audio[0]["words"][0]["text"], "It")


class Languages(unittest.TestCase):
    SCRIPT = {
        "title": {"en": "A tray", "de": "Eine Schale"},
        "summary": "English only.",
        "card": {"title": "Tray", "title_de": "Schale", "points": [{"en": "Sketch", "de": "Skizze"}]},
        "steps": [
            {"name": "a", "say": {"en": "Open it.", "de": "Öffne sie."}, "actions": [{"click": {"ribbon": "Extrude"}}]},
            {"name": "b", "say": "Next.", "say_de": "Weiter."},
            {"name": "c", "shot": False},
        ],
    }

    def test_a_script_in_each_language(self):
        de = tutorial.localized(self.SCRIPT, "de")
        self.assertEqual(de["title"], "Eine Schale")
        self.assertEqual(de["summary"], "English only.")
        self.assertEqual(de["card"], {"title": "Schale", "points": ["Skizze"]})
        self.assertEqual([s.get("say") for s in de["steps"]], ["Öffne sie.", "Weiter.", None])
        self.assertEqual(de["steps"][0]["actions"], [{"click": {"ribbon": "Extrude"}}])
        en = tutorial.localized(self.SCRIPT, "en")
        self.assertEqual([s.get("say") for s in en["steps"]], ["Open it.", "Next.", None])
        self.assertNotIn("say_de", en["steps"][1])

    def test_a_step_without_german_text_stops_a_german_build(self):
        self.assertEqual(tutorial.missing_text(self.SCRIPT, "de"), [])
        script = {"steps": [{"name": "a", "say": "Only English."}, {"name": "b", "text": {"de": "Nur Deutsch."}}]}
        self.assertEqual(tutorial.missing_text(script, "de"), [1])
        self.assertEqual(tutorial.missing_text(script, "en"), [2])

    def test_german_files_notices_and_words(self):
        with mock.patch.object(tutorial, "LANG", "de"), mock.patch.object(tutorial, "VOICE_INTRO", None), \
                mock.patch.object(tutorial, "APP_NAME", "OpenSolve"):
            self.assertEqual(tutorial.suffix(), ".de")
            self.assertEqual(tutorial.work_dir("tray").name, "tray.de")
            self.assertEqual(tutorial.spoken("8 mm, 90°"), "8 Millimeter, 90 Grad")
            self.assertEqual(
                tutorial.voice_intro({}),
                "Dieses Video wird von einer KI-Stimme gesprochen, damit es immer die aktuelle Version "
                "von OpenSolve zeigen kann.",
            )
            self.assertEqual(tutorial.voice_notice({}), "Stimme: KI-generiert (Kokoro)")
            self.assertEqual(tutorial.images_dir({}), "images")
            self.assertEqual(tutorial.images_dir({"ui_languages": ["en", "de"]}), "images.de")
        self.assertEqual(tutorial.suffix(), "")
        self.assertEqual(tutorial.voice_notice({}), "Voice: AI-generated (Kokoro)")


if __name__ == "__main__":
    unittest.main()


class Names(unittest.TestCase):
    """A function's own names never shadow the module's functions it calls
    later: a loop variable `text` once hid `text()` and the video step
    failed with "'str' object is not callable"."""

    def test_no_local_name_hides_a_module_function(self):
        import ast

        source = Path(tutorial.__file__).read_text(encoding="utf-8")
        tree = ast.parse(source)
        functions = {n.name for n in tree.body if isinstance(n, ast.FunctionDef)}
        hidden = []
        for f in (n for n in tree.body if isinstance(n, ast.FunctionDef)):
            bound = set()
            for node in ast.walk(f):
                if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store):
                    bound.add(node.id)
            called = {
                node.func.id
                for node in ast.walk(f)
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
            }
            hidden += [f"{f.name}: {n}" for n in bound & called & functions]
        self.assertEqual(hidden, [])
