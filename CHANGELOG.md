# Changelog

One version and one tag for both crates of the repository, the recorder
(`egui-screencast`) and the lessons (`egui-lessons`).

## 0.5.0

- **egui-lessons**, a second crate in `lessons/` (the repository a
  workspace, the recorder still its root package): a lesson's narration in
  the app, moved from OpenSolve. The keys of a spoken section as the
  recorder makes them (Fish S2 Pro, ElevenLabs with or without a language,
  Kokoro), the recordings' caches and the app's own, the voices found on
  the machine, a worker that speaks ahead (`worker.py`), the courses'
  sections downloaded once per course and language as a bundle (AAC in
  MP4, checked by SHA-256), the player with its clock, progress bar and the
  speaker's level, `--audio-check`, and the voice's row and settings rows
  (feature `ui`). One speaker per lesson. The app gives its names and
  places as a `Config` and what each step says as `voice::Narrate`.
  Features `playback` (rodio, symphonia) and `ui`, on by default.
- `scripts/lesson_fixture.py` writes the lessons' key fixture with the
  recorder's own code; CI checks it.
- The plan says the run's language (`"lang"`), so the app plays it in that
  language.
- `--ui-languages en,de`: a German run's pictures go to `images.de/` and
  its page links them, without a script saying `ui_languages`; a script's
  own still wins.

## 0.4.4

ElevenLabs is told the text's language (`--voice-language`,
`language_code`, in the key only when sent); a pronunciation list says a
group's digits as German number words (`words:`).

## 0.4.3

A pronunciation list per language says what the voice is given instead of
what is written.

## 0.4.2

The narration is cut in the pause before a cue's word, faded, and made
again from a recording by `--remix`.

## 0.4.1

The video is made again; a subtitle no longer hides the function that
names its language.

## 0.4.0

Fish Audio S2 Pro as the default voice in English and German, ElevenLabs,
every section cached, the voice says first that it is AI.
