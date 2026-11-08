"""Tests of the runner's timing: cues, the words they move, subtitles.

    python -m unittest discover scripts   # with the venv's Python (PyYAML)
"""

import contextlib
import io
import json
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


def spoken_words(rate: int = 24_000):
    """A word (a tone) from 0.1 s to 0.5 s, a pause with a little noise to
    0.7 s, a second word to 1.2 s: 16-bit-like float audio."""
    import numpy as np

    t = np.arange(round(1.3 * rate)) / rate
    data = np.random.default_rng(1).normal(0, 3e-4, len(t)).astype(np.float32)
    for a, b in ((0.1, 0.5), (0.7, 1.2)):
        on = (t >= a) & (t < b)
        data[on] += 0.4 * np.sin(2 * np.pi * 220 * t[on]).astype(np.float32)
    return data


class Cuts(unittest.TestCase):
    RATE = 24_000

    def test_a_cue_late_into_its_word_is_cut_in_the_pause_before_it(self):
        data = spoken_words(self.RATE)
        # The aligner puts the second word 70 ms after it begins.
        cut = tutorial.quiet_cut(data, self.RATE, 0.77)
        self.assertGreater(cut / self.RATE, 0.5)
        self.assertLess(cut / self.RATE, 0.7)
        # The pause's start (a frame into it), not right before the word.
        self.assertLess(cut / self.RATE, 0.55)

    def test_cuts_follow_each_other(self):
        data = spoken_words(self.RATE)
        points = tutorial.cut_points(data, self.RATE, [(0.77, 0.5), (0.72, 1.0)])
        self.assertEqual(len(points), 2)
        self.assertGreaterEqual(points[1], points[0])

    def test_each_piece_is_faded_and_added(self):
        import numpy as np

        rate = self.RATE
        data = spoken_words(rate)
        track = np.full(round(3 * rate), 0.1, dtype=np.float32)
        at_cuts = tutorial.place_narration(track, data, rate, 0.5, [(0.77, 0.4)])
        cut = round(at_cuts[0] * rate)
        self.assertTrue(0.5 < at_cuts[0] < 0.7)
        # The first word where it was, added to what was there.
        k = round((0.5 + 0.3) * rate)
        self.assertAlmostEqual(float(track[k]), 0.1 + float(data[k - round(0.5 * rate)]), places=5)
        # The second word 0.4 s later, the gap only what was there before.
        gap = round((0.5 + at_cuts[0] + 0.2) * rate)
        self.assertLess(float(np.abs(track[gap] - 0.1)), 0.01)
        k = round((0.5 + 0.4 + 0.9) * rate)
        self.assertAlmostEqual(float(track[k]), 0.1 + float(data[round(0.9 * rate)]), places=5)
        # Faded at the cut: each piece starts and ends at nothing.
        piece = tutorial.faded(np.ones(1000, dtype=np.float32), rate)
        self.assertEqual(float(piece[0]), 0.0)
        self.assertLess(float(piece[-1]), 0.01)
        self.assertEqual(float(piece[500]), 1.0)
        start = round((0.5 + 0.4) * rate) + cut
        self.assertAlmostEqual(float(track[start]), 0.1, places=5)

    def test_no_cut_where_the_words_do_not_wait(self):
        import numpy as np

        data = spoken_words(self.RATE)
        track = np.zeros(round(2 * self.RATE), dtype=np.float32)
        tutorial.place_narration(track, data, self.RATE, 0.0, [(0.77, 0.0)])
        np.testing.assert_allclose(track[200:len(data) - 200], data[200:len(data) - 200])

    def test_the_subtitles_move_from_the_cut_on(self):
        said = [
            {"text": "Click", "start": 0.1, "end": 0.5, "space": True},
            {"text": "Open", "start": 0.77, "end": 1.2, "space": True},
        ]
        # The cut at 0.52 s moves "Open" (at 0.77 s) as one with its sound.
        subs = tutorial.subtitles_of(said, [(0.52, 0.4)])
        self.assertEqual([t for _, _, t in subs], ["Click", "Open"])
        self.assertAlmostEqual(subs[1][0], 1.17)


class Cached:
    """A narrator's cache: the sections given, nothing else (as a dry run:
    `None` for a section it has not)."""

    def __init__(self, folder: Path, texts: list[str]):
        self.fake = FakeNarrator(folder)
        self.have = {t: self.fake.say(t) for t in texts}
        self.asked: list[str] = []

    def say(self, text: str):
        self.asked.append(text)
        return self.have.get(text, (None, 0.0, []))

    def report(self) -> str:
        return "cached"


class Remix(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.docs, self.work, self.out = root / "docs", root / "work", root / "out"
        (self.docs / "t").mkdir(parents=True)
        (self.docs / "t" / "script.yaml").write_text(
            "title: T\nsteps:\n"
            "  - {name: a, say: 'Click it. Open it.', actions: [{cue: Open it}]}\n"
            "  - {name: b, say: Next.}\n",
            encoding="utf-8",
        )
        record = self.work / "t" / "recording"
        (record / "frames").mkdir(parents=True)
        (record / "frames.txt").write_text("", encoding="utf-8")
        plan = {"fps": 30, "steps": [
            {"name": "a", "actions": [{"cue": 1.5, "words": "Open it"}], "shot": None, "seconds": 3.0},
            {"name": "b", "actions": [], "shot": None, "seconds": 1.0},
        ], "caption": "Voice: AI-generated (Kokoro)"}
        (self.work / "t" / "plan.json").write_text(json.dumps(plan), encoding="utf-8")
        timeline = {"fps": 30, "frames": 180, "errors": [],
                    "steps": [{"start": 0, "cues": [60]}, {"start": 120, "cues": []}]}
        (record / "timeline.json").write_text(json.dumps(timeline), encoding="utf-8")
        cache = root / "cache"
        cache.mkdir()
        self.cached = Cached(cache, ["Click it. Open it.", "Next."])
        speaker = dict(tutorial.KOKORO_SPEAKER, narrator=lambda: self.cached)
        self.ffmpeg = []
        for p in (
            mock.patch.object(tutorial, "DOCS", self.docs),
            mock.patch.object(tutorial, "WORK", self.work),
            mock.patch.object(tutorial, "OUT", self.out),
            mock.patch.object(tutorial, "REMIX", True),
            mock.patch.object(tutorial, "VOICE_INTRO", False),
            mock.patch.object(tutorial, "speaker_of", lambda script, dry_run=False: speaker),
            mock.patch.object(tutorial, "play", mock.Mock(side_effect=AssertionError("the app was started"))),
            mock.patch.object(tutorial.subprocess, "run", lambda cmd, **kw: self.ffmpeg.append(cmd)),
            mock.patch.object(tutorial.shutil, "which", lambda name: "/usr/bin/" + name),
        ):
            p.start()
            self.addCleanup(p.stop)

    def test_it_uses_the_recording_and_the_cache_and_not_the_app(self):
        tutorial.build("t", voice=True, with_video=True, lang="en")
        made = self.out / "t"
        self.assertTrue((made / "narration.wav").exists())
        self.assertIn("Open it.", (made / "subtitles.srt").read_text(encoding="utf-8"))
        self.assertEqual(len(self.ffmpeg), 1)
        cmd = [str(c) for c in self.ffmpeg[0]]
        self.assertEqual(cmd[-1], str(made / "t.mp4"))
        self.assertIn(str(self.work / "t" / "recording" / "frames.txt"), cmd)
        self.assertEqual(self.cached.asked, ["Click it. Open it.", "Next."])
        # Nothing written beside the script or into the recording.
        self.assertEqual(sorted(p.name for p in (self.docs / "t").iterdir()), ["script.yaml"])
        self.assertFalse((self.work / "t" / "narration.wav").exists())

    def test_a_section_not_in_the_cache_stops_it(self):
        del self.cached.have["Next."]
        with self.assertRaises(SystemExit) as e:
            tutorial.build("t", voice=True, with_video=True, lang="en")
        self.assertIn("not in the cache", str(e.exception))
        self.assertEqual(self.ffmpeg, [])

    def test_no_recording_stops_it(self):
        (self.work / "t" / "recording" / "timeline.json").unlink()
        with self.assertRaises(SystemExit) as e:
            tutorial.build("t", voice=True, with_video=True, lang="en")
        self.assertIn("needs a recording", str(e.exception))


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


def old_spoken(text: str, lang: str) -> str:
    """`tutorial.spoken` of 0.4.2, before the pronunciation list: the
    sections' keys must stay."""
    import re

    t = tutorial.plain(text)
    if lang == "de":
        t = re.sub(r"(\d)\s*mm\b", r"\1 Millimeter", t)
        t = re.sub(r"(\d)\s*°", r"\1 Grad", t)
        t = re.sub(r"\bM(\d)", r"M \1", t)
        t = t.replace("×", "mal").replace("›", ",").replace("→", "zu")
        return t.replace("JLCCNC", "JLC CNC")
    t = re.sub(r"(\d)\s*mm\b", r"\1 millimeters", t)
    t = re.sub(r"(\d)\s*°", r"\1 degrees", t)
    t = re.sub(r"\bM(\d)", r"M \1", t)
    t = t.replace("×", "by").replace("›", ",").replace("→", "to")
    return t.replace("JLCCNC", "JLC CNC").replace("aluminium", "aluminum")


class Pronunciation(unittest.TestCase):
    """The pronunciation list: what the voice is given instead of what is
    written, for each engine; cues found and subtitles shown as written."""

    LIST = r"""
all:
  - {from: "XY-Ebene", to: "Iks-Ypsilon-Ebene"}
  - {from: "z. B.", to: "zum Beispiel"}
  - {regex: '\bM(\d+)\.(\d+)\b', to: 'M \1 Komma \2'}
fish:
  - {from: "Ebene", to: "Ebehne"}
elevenlabs:
  - {from: "Skizze", to: "Skitze"}
kokoro: []
"""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        (self.dir / "pronunciation.de.yaml").write_text(self.LIST, encoding="utf-8")
        for p in (
            mock.patch.object(tutorial, "PRONUNCIATIONS", self.dir),
            mock.patch.object(tutorial, "WORK", self.dir),
            mock.patch.object(tutorial, "LANG", "de"),
            mock.patch.object(tutorial, "PRONOUNCE", []),
            mock.patch.object(tutorial, "VOICE_INTRO", False),
            mock.patch.dict(tutorial.LOADED_PRONUNCIATIONS, clear=True),
        ):
            p.start()
            self.addCleanup(p.stop)

    def said(self, text: str, backend: str | None = "kokoro") -> str:
        return tutorial.said_text(text, backend, "de")

    def test_a_literal_is_whole_words_and_its_case(self):
        self.assertEqual(self.said("Die XY-Ebene, z. B. hier."), "Die Iks-Ypsilon-Ebene, zum Beispiel hier.")
        # Not inside another word, not in another case.
        self.assertEqual(self.said("Die XY-Ebenen und die xy-ebene."), "Die XY-Ebenen und die xy-ebene.")
        self.assertEqual(self.said("AXY-Ebene"), "AXY-Ebene")
        # A phrase that ends in a mark needs no word's end after it.
        self.assertEqual(self.said("z. B.Test"), "zum BeispielTest")

    def test_a_regex_with_its_groups(self):
        self.assertEqual(self.said("Eine M2.5-Schraube"), "Eine M 2 Komma 5-Schraube")
        # On the text as written, before the units are read out: "M3" then
        # is "M 3", "8 mm" "8 Millimeter".
        self.assertEqual(self.said("M10.25 und M3, **8 mm**"), "M 10 Komma 25 und M 3, 8 Millimeter")

    def test_spoken_is_as_it_was_without_a_list(self):
        texts = [
            "Extrude by **10 mm**, [pause] then turn it 90° and use an M3 screw: 20 × 30 → done.",
            "A `code` mark, _emphasis_, a [link](https://example.org) and ![a picture](x.png).\n\n- a point",
            "JLCCNC cuts aluminium; it’s 3.5 mm  thick. Datei › Neu, M12x1.5, 5mm",
        ]
        for lang in ("en", "de"):
            for t in texts:
                with mock.patch.object(tutorial, "LANG", lang):
                    self.assertEqual(tutorial.spoken(t), old_spoken(t, lang))

    def test_each_engine_after_all(self):
        text = "Die XY-Ebene und eine Ebene, eine Skizze."
        self.assertEqual(self.said(text, "kokoro"), "Die Iks-Ypsilon-Ebene und eine Ebene, eine Skizze.")
        # `all` first: S2 Pro's own `Ebene` then finds the one `all` made too.
        self.assertEqual(self.said(text, "fish-s2-pro"), "Die Iks-Ypsilon-Ebehne und eine Ebehne, eine Skizze.")
        self.assertEqual(self.said(text, "elevenlabs"), "Die Iks-Ypsilon-Ebene und eine Ebene, eine Skitze.")
        # Without a voice, only `all`.
        self.assertEqual(self.said(text, None), "Die Iks-Ypsilon-Ebene und eine Ebene, eine Skizze.")
        # No list in a language: nothing replaced.
        self.assertEqual(tutorial.said_text("Die XY-Ebene", "kokoro", "en"), "Die XY-Ebene")

    def test_a_bad_list_stops_the_build(self):
        for bad in (
            "fishaudio: []",
            "all: [{from: a}]",
            "all: [{from: a, regex: b, to: c}]",
            "all: [{regex: '(', to: c}]",
            "all: [{regex: 'a(b)', to: '\\2'}]",
            "all: [{from: a, to: '\\1'}]",
            "all: [{regex: 'a', to: '\\n'}]",
        ):
            (self.dir / "pronunciation.en.yaml").write_text(bad, encoding="utf-8")
            tutorial.LOADED_PRONUNCIATIONS.clear()
            with self.assertRaises(SystemExit, msg=bad):
                tutorial.rules_for("en", "kokoro")

    def test_the_voice_gets_the_replaced_text_and_cues_find_it(self):
        narrator = FakeNarrator(self.dir)
        script = {
            "title": "T",
            "steps": [
                {
                    "name": "a",
                    "say": "Wähle die XY-Ebene. Dann zeichne.",
                    "actions": [{"cue": "die XY-Ebene"}, {"cue": "Dann"}],
                },
            ],
        }
        audio = tutorial.narrate("t", script, True, {"backend": "kokoro"}, narrator)
        self.assertEqual(narrator.said, ["Wähle die Iks-Ypsilon-Ebene. Dann zeichne."])
        actions = tutorial.cue_actions("t", 0, script["steps"][0], audio[0]["words"])
        # "die" is the second mark, "Dann" the fifth: Wähle die Iks-Ypsilon-Ebene . Dann
        self.assertEqual(actions[0], {"cue": 0.5, "words": "die XY-Ebene"})
        self.assertEqual(actions[1]["cue"], 2.0)

    def test_subtitles_show_what_is_written(self):
        narrator = FakeNarrator(self.dir)
        script = {"title": "T", "steps": [{"name": "a", "say": "Nimm z. B. eine M2.5-Schraube. Fertig."}]}
        audio = tutorial.narrate("t", script, True, {"backend": "fish-s2-pro"}, narrator)
        self.assertEqual(narrator.said, ["Nimm zum Beispiel eine M 2 Komma 5-Schraube. Fertig."])
        subs = tutorial.subtitles_of(audio[0]["words"], [])
        self.assertEqual([t for _, _, t in subs], ["Nimm z. B. eine M2.5-Schraube.", "Fertig."])
        # Timed by the words said: from "Nimm" to the "." after "Schraube".
        said = words(narrator.said[0])
        self.assertEqual(subs[0][0], said[0]["start"])
        self.assertEqual(subs[1][0], said[-2]["start"])

    def test_back_to_what_is_written_word_by_word(self):
        rules = tutorial.rules_for("de", "fish-s2-pro")
        said, pieces = tutorial.pronounce("Die XY-Ebene, z. B. hier", rules)
        self.assertEqual(said, "Die Iks-Ypsilon-Ebehne, zum Beispiel hier")
        self.assertEqual("".join(w for _, w, _ in pieces), "Die XY-Ebene, z. B. hier")
        spoken_words = [
            {"text": t, "start": float(i), "end": i + 1.0, "space": s}
            for i, (t, s) in enumerate(
                [("Die", True), ("Iks-Ypsilon-Ebehne", False), (",", True), ("zum", True), ("Beispiel", True),
                 ("hier", False)]
            )
        ]
        shown = tutorial.written_words(spoken_words, pieces)
        self.assertEqual([w.get("shown", w["text"]) for w in shown], ["Die", "XY-Ebene", ",", "z. B.", "", "hier"])
        self.assertTrue(shown[3]["shown_space"])
        # The said words stay, for the cues.
        self.assertEqual([w["text"] for w in shown], [w["text"] for w in spoken_words])

    def test_a_space_said_but_not_written(self):
        import align

        (self.dir / "pronunciation.en.yaml").write_text(
            "all:\n  - {regex: '\\bM(\\d+)-', to: 'Em \\1 '}\n", encoding="utf-8"
        )
        said, pieces = tutorial.pronounce("Two M3-screws hold it.", tutorial.rules_for("en", None), lang="en")
        self.assertEqual(said, "Two Em 3 screws hold it.")
        said_words = [dict(t, start=float(k), end=k + 1.0) for k, t in enumerate(align.tokens_of(said))]
        subs = tutorial.subtitles_of(tutorial.written_words(said_words, pieces), [])
        self.assertEqual(subs[0][2], "Two M3-screws hold it.")

    def test_a_rule_over_a_replacement_takes_it_whole(self):
        (self.dir / "pronunciation.en.yaml").write_text(
            "all:\n  - {from: CAD, to: C A D}\n  - {regex: 'A D file', to: 'A D data'}\n", encoding="utf-8"
        )
        said, pieces = tutorial.pronounce("A CAD file.", tutorial.rules_for("en", None))
        self.assertEqual(said, "A C A D data.")
        self.assertEqual(pieces, [("A ", "A ", False), ("C A D data", "CAD file", True), (".", ".", False)])

    def test_the_key_changes_only_where_a_rule_replaces(self):
        import elevenlabs
        import narrate

        untouched, touched = "Zeichne eine Lini.", "Wähle die XY-Ebene."
        voice = elevenlabs.Voice("v")
        for backend in ("kokoro", "fish-s2-pro", "elevenlabs"):
            self.assertEqual(narrate.key(self.said(untouched, backend), "am_michael", 1.0),
                             narrate.key(untouched, "am_michael", 1.0))
            self.assertEqual(voice.key(self.said(untouched, backend)), voice.key(untouched))
            self.assertNotEqual(voice.key(self.said(touched, backend)), voice.key(touched))
        # An engine's own rule changes only that engine's key.
        self.assertEqual(voice.key(self.said("Eine Skizze.", "kokoro")), voice.key("Eine Skizze."))
        self.assertNotEqual(voice.key(self.said("Eine Skizze.", "elevenlabs")), voice.key("Eine Skizze."))

    def test_the_run_says_how_many_replaced(self):
        narrator = FakeNarrator(self.dir)
        script = {
            "title": "T",
            "steps": [{"name": "a", "say": "Die XY-Ebene, die XY-Ebene."}, {"name": "b", "say": "Nichts."}],
        }
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            tutorial.narrate("t", script, True, {"backend": "kokoro"}, narrator)
        self.assertIn("pronunciation.de.yaml: 2 replacements in 1 of 2 sections", out.getvalue())
