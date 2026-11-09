# The script format

A course is a tutorial's script, `<tutorials>/<name>/script.yaml`, read
alike by the recorder (`scripts/tutorial.py`, which makes the page and the
video) and by egui-lessons (`egui_lessons::script`, which plays it as a
lesson in the app). This file says every key, who reads it, and which keys
are the app's own.

The recorder's page and video come from `title`, `summary`, the steps'
`text`, `say`, `actions` and `card`. A lesson reads the same keys, and a few
of its own (`*_lesson`) where it should say otherwise than the video. Keys
neither knows are left alone; an app reads its own keys of a step through
its `Extra` (`check:`, `check_result:`, `intent:` in OpenSolve).

## Languages

Every text, and any value of an action, may be given per language:

- as a map whose keys are all languages: `title: {en: A tray, de: Eine Schale}`;
- or as `<key>_<lang>` beside `<key>`: `title_de: Eine Schale`.

The language's entry is taken, English where it has none, at any depth
(`localized()` in both). A plain text is English. Today's languages are
`en` and `de` (opencad-ui's `Lang`); a script with any German is played in
German too.

## The script

| Key | Read by | Says |
|---|---|---|
| `title` | both | The course's title. Required. |
| `summary` | both | What it is and what you learn (Markdown). |
| `summary_lesson` | lesson | The summary in the app, where it differs (a file it opens itself). |
| `needs`, `next` | recorder | Lines of the page. |
| `voice` | both | Kokoro's voice (`am_michael`, `bf_emma`): the English recordings' and a local Kokoro's. |
| `speed` | both | The voice's speed, `1.0`; per language (`speed_de`) for ElevenLabs. |
| `voice_settings` | both | ElevenLabs' settings over its defaults (`{stability: 0.4}`): in its sections' keys. |
| `voice_model` | both | ElevenLabs' model (`eleven_v3`); the run's `--voice-model` wins. |
| `voice_language` | both | The language ElevenLabs is told (`de`, `auto`, `off`); the run's `--voice-language` wins. |
| `ui_languages` | recorder | The languages the app's window speaks: a run in another one has its own pictures. |
| `steps` | both | The steps, in order. Required, at least one. |

## A step

| Key | Read by | Says |
|---|---|---|
| `name` | both | Its name: the picture's (`02-<name>.png`); else its number. |
| `title` | both | Its title. |
| `text` | both | Markdown: on the page, and spoken where there is no `say`. |
| `text_lesson` | lesson | What the lesson pane shows instead of `text`. |
| `say` | both | Spoken instead of `text`. Delivery tags (`[pause]`) go to the voices that take them. |
| `say_lesson` | lesson | Spoken by a lesson instead of `say`, in that language only (`say_lesson_de`, `{en: …, de: …}`): "Thanks for following along" where the video says "Thanks for watching". No recording makes it; `scripts/lesson_voices.py` does. |
| `ui`, `ui_lesson` | both, lesson | Where the interface has it ("Sketch › Rectangle"). |
| `hint`, `hint_lesson` | lesson | What to try when the step does not hold. |
| `card` | both | A title card: `title`, `subtitle`, `points` (`points_de`), `logo`, `background`, `accent`. A lesson shows its points. |
| `shot` | recorder | `false`: no picture. |
| `pending_ui` | recorder | A step done through tools until the interface has it. |
| `actions` | both | What it does in the app, in order (below). |
| `check` | app | What the step made, checked against the app's model (OpenSolve). |
| `check_result` | app | Whether the step passes by its result too (`true`, `false`, `only`; OpenSolve). |
| `intent` | app | The parameters the result must follow (a list, or `false`; OpenSolve). |

## Actions

Each a map; any value may be given per language (`type: {en: width, de:
breite}`). The recorder plays them through the app's `egui_screencast::Stage`;
the targets of `click:` are the app's (`{ribbon: Extrude}`, `{field: Depth}`,
`{sketch: [x, y]}` …: the labels in English, found in any language).
The format's own:

| Action | Does |
|---|---|
| `click: {…}` | A click on a target. In a lesson the hint's targets, in order. |
| `type: text` | Typed: into the field clicked last, else into the inline editor that click opened (a lesson's target `{editor: true}`). |
| `key: Enter`, `shift`/`alt`/`command: true` | A key. `Escape` before a click: the tool at hand is put away first. |
| `cue: words` | Waits until the narration says these words (`{en: …, de: …}`, quoted where they have a comma). Each cue's words must be said in the step's `say` (else `text`) in that language, after the cue before it, or the script does not read. |
| `wait: seconds` | Nothing for a while. |
| `shot: true` | The step's picture here. |
| `open: path`, `open_de: path` | A file opened (the app's; in a lesson a file of the course, copied into its folder first). |
| `{work}` in a string | The course's work folder. |

Everything else (`tool:`, `select:`, `command:`, `hover:`, `ribbon:` …) is
the app's, played by its `Stage`.

## Pronunciation

`pronunciation.<lang>.yaml` beside the tutorials says what a voice is given
instead of what is written: `all`, then an engine's own (`elevenlabs`,
`fish`, `kokoro`), each a list of `{from: …, to: …}` (a word or phrase,
whole words, its case) or `{regex: …, to: …}` (`\1`, `\g<name>`; in German
`words: [2]` says group 2's digits as number words). Applied in order to the
text as written (Markdown gone), before units and signs are read out ("8 mm",
"M3", "×"). Regexes must be Rust's too: no look-around, no back-references.
The app embeds the lists (`egui_lessons::pronounce::Lists`), so its keys are
the recordings'.

## Keys and fixtures

What a voice is given is what its section is cached by. The recorder's
Python defines it; egui-lessons must make the same keys to the letter:

- `lessons/tests/fixtures/keys.json` (`scripts/lesson_fixture.py`): the keys
  of each voice for awkward texts.
- `lessons/tests/fixtures/spoken.json` (`scripts/spoken_fixture.py
  --tutorials lessons/tests/tutorials`): the text each engine is given and
  its keys, for the test tutorials and extra texts with lists of their own.
- `lessons/tests/fixtures/lesson_voices.json`
  (`scripts/lesson_voices_fixture.py`): the lessons' own sections.

CI writes each again with `--check`. An app keeps its own of its tutorials
the same way (OpenSolve: `--tutorials docs/tutorials --name opensolve`).
