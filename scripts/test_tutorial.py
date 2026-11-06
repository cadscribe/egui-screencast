"""Tests of the runner's timing: cues, the words they move, subtitles.

    python3 -m unittest discover scripts
"""

import sys
import unittest
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
