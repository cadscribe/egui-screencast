# egui-screencast

Tutorials played and recorded by an egui app itself: pictures for a page
and a narrated video, made from a script, the same on every run.

- **A plan** of steps, each a list of actions: `click`/`hover` a target,
  `key`, `type`, `wait`, `zoom` about a target, and the app's own actions.
- **A hand-like pointer**: on a bow, fast then slow, a little past the
  target and back; drawn over the app, since a screenshot has none.
- **Pictures from egui**, not from the screen: a step's picture once it has
  settled, and every frame of the video (one equal to the last is held, not
  written again).
- **The frames' clock**: every frame is `1/fps` later than the last,
  however long it took, so waiting for the app costs no video time.
- **Without a window** (`headless::run`): the app drawn into a texture on
  the GPU at a fixed size (1920 × 1080), as fast as the GPU draws.

## In an app

```rust
impl egui_screencast::Stage for MyApp {
    fn locate(&self, target: &Value) -> Option<Pos2> { /* {"button": "Save"} → where */ }
    fn act(&mut self, ctx: &egui::Context, action: &Value, seconds: f64,
           kept: &mut serde_json::Map<String, Value>) -> Result<Option<String>, String> {
        /* {"command": "undo"} → done; Some(line) shows it in the video */
    }
}
```

Load a `Player` for `--play plan.json --record dir`, give it the input in
`raw_input_hook` (`player.input(raw)`) and a frame after drawing
(`player.frame(self, ctx)`, `true` when it is over), and with `--headless`
run the app by `headless::run`, its paint callbacks set up by the function
passed (as `wgpu_render_state` would offer them). OpenSolve's
`crates/opensolve-app/src/record.rs` is a whole example.

## Cards

A step can show a card over the whole window while it plays: an intro with
what the tutorial shows, an ending with a logo and a link. It comes in over
the app and goes at the end of the step's narration; on the first step the
video opens on it, on the last it ends on it. Its parts come in one after
the other, each fading in and moving a little into place: the logo, the
title, the subtitle, a rule in the accent's colour, the points one by one,
the link.

```yaml
card:
  title: A 3D-printed tray
  subtitle: in OpenSolve
  points: [Parameters for its sizes, An offset for the wall]
  logo: assets/icon/app-512.png      # PNG, from the folder the runner is run in
  link: github.com/you/app
  background: "#0e4a45"
  accent: "#f28c28"
```

## The runner

`scripts/tutorial.py` builds `<tutorials>/<name>/script.yaml` into the
plan, speaks the narration (Fish Audio S2 Pro by default, Kokoro with
`--fast`, or ElevenLabs; below), plays it in the app and writes `images/`,
`README.md` and `<name>.mp4` (ffmpeg), in English and, with `--lang de`,
in German (`README.de.md`, `<name>.de.mp4`):

    python scripts/tutorial.py --app target/release/app \
        [--app-arg ARG]… [--env K=V]… [--tutorials docs/tutorials] \
        [--no-voice] [--no-video] [--window] [--app-name NAME] \
        [--lang en|de|en,de] [--fast] [--voices DIR] [--voice V] [--dry-run] [NAME…]

`--remix` makes the narration, the subtitles and the video again from a run
already recorded (`<work>/<name>[.<lang>]/recording`, its `plan.json`) and
the cached sections: the app is not started and nothing is spoken; a
missing recording or section stops it. `--out DIR` writes the video, its
`narration.wav` and `subtitles.srt` to `DIR/<name>[.<lang>]/` instead of
beside the script and in the work folder:

    python scripts/tutorial.py --remix --lang en,de [--out DIR] [NAME…]

Tests: `python -m unittest discover scripts` with the venv's Python
(PyYAML); no model is loaded, no paid API called.

## Two languages

A run speaks and writes one language or several (`--lang en,de`). A
script gives a text in each: `say: {en: "…", de: "…"}`, or `say_de:`
beside `say:` (any key: `title_de`, `text_de`, a card's `title_de`, …); a
plain text is English, and a step with narration but none in the run's
language stops the build. German files are `<name>.de.mp4` and
`README.de.md`, the work in `<work>/<name>.de/`; the pictures stay in
`images/` unless the script says the app speaks the language too
(`ui_languages: [en, de]`, then `images.de/`; `{lang}` in an `--app-arg`
or `--env` tells the app). Units are spelled for the voice in each
language ("8 Millimeter"), and the runner's own words (the notices, the
page's lines) are German too.

## Voices: Fish Audio S2 Pro, Kokoro or ElevenLabs

The engine is Fish Audio S2 Pro (below) unless `--fast` (Kokoro), or
`--engine`, `$SCREENCAST_ENGINE` or the script's `engine:` says otherwise.
The voice is `--voice`, else `$SCREENCAST_VOICE`, else the script's
`voice:` (`voice_de:` for German), else by the language: `en-gb-female`
for English, `de-male` for German. `kokoro:<name>`,
`elevenlabs:<voice id>` or `fish-s2-pro:<voice>` name the engine too; a
Kokoro voice in a script (`voice: am_michael`) is used by Kokoro only.
Kokoro speaks English only.

    python scripts/tutorial.py --app … --fast                     # Kokoro: quick, English
    python scripts/tutorial.py --app … --voices ~/tts-voices --lang en,de
    export ELEVENLABS_API_KEY=…            # elevenlabs.io → Developers → API keys
    python scripts/elevenlabs.py voices    # your voices and their ids (costs nothing)
    python scripts/tutorial.py --app … --voice elevenlabs:<voice id> --dry-run   # what it would cost
    python scripts/tutorial.py --app … --voice elevenlabs:<voice id>

ElevenLabs is asked over its HTTP API with the standard library (nothing
more to install): `POST /v1/text-to-speech/{voice_id}/with-timestamps`,
16-bit PCM at 24 kHz (`pcm_24000`) and when each character is said. Its
characters are made words and marks with their times, as Kokoro gives
them, so cues and subtitles work alike. The key is only read from
`ELEVENLABS_API_KEY` and sent in the request's header; never written, never
printed. In a script, or over it on the command line:

```yaml
voice: elevenlabs:JBFqnCBsd6RMkjVDRZzb
voice_name: George                # for the notices; else asked of ElevenLabs once
voice_model: eleven_multilingual_v2   # the default; --voice-model, $ELEVENLABS_MODEL
voice_settings: {stability: 0.5, similarity_boost: 0.75, style: 0, use_speaker_boost: true}
speed: 1.0                        # ElevenLabs' speed (0.7 to 1.2) unless voice_settings has one
```

`--voice-setting K=V` (repeated) sets one over the script's.

**The language.** A model guesses the text's language unless it is told:
`--voice-language de` (else `$ELEVENLABS_LANGUAGE`, else the script's
`voice_language:`) sends `language_code: "de"` beside the model, `auto`
the run's `--lang`, `off` none. It is off unless asked for. German reads
its numbers far better with `eleven_v3` told so:

    python scripts/tutorial.py --app … --lang de --voice elevenlabs:<voice id> \
        --voice-model eleven_v3 --voice-language de

A language is part of a section's cache key only when one is sent, so
the sections cached without one keep their keys.

**The cache.** Every section said (a step's text, the voice's own notice)
is kept by a key: the SHA-256 of the backend, the voice id, the model, the
voice settings, the output format, the language (only when one is sent)
and the text as said (after `spoken()`,
its whitespace made single spaces). A section whose key is in the cache is
never asked for again; only new or changed text is sent. An entry is a
folder (`audio.wav`, `words.json`, `meta.json`) made whole and then renamed
into place, so a failed request (no key, HTTP 401, 429, …) leaves nothing
behind, and the sections spoken before it are kept. It lives in
`<work>/voices` (`--voice-cache DIR`, `$SCREENCAST_VOICE_CACHE`), with
Fish S2 Pro's (their keys say which engine),
beside Kokoro's model in `<work>/hf` and Kokoro's sections in
`<work>/audio`.

**Costs.** ElevenLabs bills by the characters sent. Each run says how many
sections came from the cache and how many characters were sent;
`--dry-run` only counts them (no key needed, no app played). A tutorial of
15 steps is about 3,000 characters the first time and nothing after, but
a changed sentence, another voice, model or setting is that section (or
all of them) again. Old entries go with

    python scripts/tutorial.py --prune-voice-cache 30   # not used for 30 days
    python scripts/elevenlabs.py prune --older-than 30 --cache target/tutorials/voices

Tests use a fake API: `python3 -m unittest discover scripts` never calls
ElevenLabs.

### Fish Audio S2 Pro, run locally

[S2 Pro](https://huggingface.co/fishaudio/s2-pro) speaks on the Mac
itself, by [mlx-speech](https://pypi.org/project/mlx-speech/) on Apple
Silicon (its 8-bit model, appautomaton/fishaudio-s2-pro-8bit-mlx, about
7 GB, about 10 GB of memory while it speaks). Nothing is sent, nothing is
billed. **The runner neither installs nor downloads it**: install it
yourself, in its own Python, and fetch its model once:

    uv tool install --python 3.13 mlx-speech      # 0.5.6 tested
    mlx-speech tts --model fish-s2-pro --text "Hello." -o /tmp/hello.wav

The runner finds its Python by the `mlx-speech` command
(`--fish-python`, `$FISH_SPEECH_PYTHON` otherwise) and starts
`scripts/fishspeech_worker.py` there once a run, offline
(`HF_HUB_OFFLINE=1`, the user's own Hugging Face cache), on the first
section not in the cache. Without mlx-speech it stops and says how to
install it, or to use `--fast`; it never switches to Kokoro by itself.

**Licence.** S2 Pro's weights are under the **Fish Audio Research
License: research and non-commercial use only** (no monetised channel, no
sponsorship); commercial use needs a licence from Fish Audio
(business@fish.audio). The weights must not be redistributed: they are
never in a repository or a release. The notices say it: the card line and
the corner name `Fish Audio S2 Pro`, the page and the MP4's comment and
sound title name the licence.

**Voices.** S2 Pro has no preset voices, and a seed alone is not a voice
(the same seed gives another speaker for another text). A voice is a
reference clip and its exact transcript, in the language of the text,
listed in a `voices.yaml` (`--voices DIR`, `$SCREENCAST_VOICES`, the
script's `voices:`):

```yaml
dir: fish-s2-pro
voices:
  en-gb-female: {language: English, reference: en-gb-female.wav, transcript: "In this tutorial, …"}
  de-male:      {language: German,  reference: de-male.wav,      transcript: "In diesem Tutorial …"}
```

or a WAV's path with its transcript beside it (`.lab`). A German voice is
told `--language German`. A fixed seed (`voice_settings: {seed: 1}`)
makes a section sound the same each time.

**Tags** in square brackets steer the delivery: `[pause]`, `[excited]`,
`[emphasis]`, `[whisper]` (no closing tag). S2 Pro gets them; the
subtitles, the cues and the alignment do not, and Kokoro and ElevenLabs
never hear them.

**Time.** On an M1 Max two new sentences (11.5 s of speech) took 33 s
with loading; reckon 3 to 7 times the speech's length. Every section is
kept in the cache (`<work>/voices`) by a key of the engine, the model, the
reference (its bytes and transcript), the language, the seed, the output
format and the text with its tags; only what changed is spoken again.

**Word times.** S2 Pro gives none; they are found by forced alignment in
the tutorial venv (`scripts/align.py`): a CTC model of the language's
letters (wav2vec2-base-960h for English, Apache 2.0;
wav2vec2-base-10k-voxpopuli-ft-de for German, CC BY-NC 4.0; about 380 MB
each, from Hugging Face on first use at fixed revisions) says how likely
each letter is in each 20 ms frame, and the most likely path through the
text's letters in order gives each word its frames; numbers are spelled
for it (num2words). Against Kokoro's own word times on Kokoro speech
(72 words) the aligned starts are a steady 73 ms later (CTC fires a
little after a word's onset), within 95 ms for 90 % of the words and
203 ms at most; the ends agree to a median of 22 ms. A cue thus comes a
few hundredths of a second after its word begins, as with Kokoro. Another
language falls back to times by the words' lengths.

## What is said with what is seen

A step's narration starts with the step, and its actions run as fast as
they can; left so, a click comes before the sentence that asks for it. A
cue ties them together:

```yaml
say: The switcher says what you work on. Open it. It lists the parts.
actions:
  - cue: Open it            # waits until the narration says these words
  - click: {spot: Switcher}
  - cue: It lists
  - shot: true              # the step's picture here, not at its end
```

The runner finds the words in Kokoro's times of each word it spoke and
makes the cue the seconds into the step when they are said; the player
waits for that moment. Should the actions before a cue take longer than
the words before it, the words from the cue on wait instead (the cue's
frame is in `timeline.json`, and the runner puts them there), so the two
never drift apart. A step lasts at least its narration, and as much longer
as its words waited. The subtitles are the words spoken, a sentence at a
time, when they are spoken. Cues go at the start of a sentence or a
clause: words that wait are cut there.

The cut is not at the cue word's time itself: forced alignment puts a word
some 70 ms after it begins, so a cut there split the word, a fragment, the
wait, then the rest. It is at the quietest point from 0.3 s before the
word to 30 ms after it (10 ms frames), at the start of the quiet there:
the pause before the word. Each piece is faded in and out over 6 ms, and
the subtitles' words move from the same cut on.

`shot: true` takes the step's picture where it stands in the actions: the
moment that shows what the page's text says, rather than whatever the step
ends on.

## Pronunciation

Some written words come out wrong from a voice: "XY-Ebene", "M2,5", "3MF".
The script keeps them as they should be read, and a pronunciation list
beside the tutorials says what the voice is given instead:
`<tutorials>/pronunciation.<lang>.yaml` (`pronunciation.de.yaml`,
`pronunciation.en.yaml`), or in `--pronunciations DIR`.

```yaml
all:                              # for every voice, in this order
  - {from: "XY-Ebene", to: "Iks-Ypsilon-Ebene"}
  - {regex: '\bM(\d+),(\d+)\b', to: 'Em \1 Komma \2'}
elevenlabs: []                    # then the engine's own: elevenlabs,
fish:                             # fish (S2 Pro), kokoro
  - {from: "Skizze", to: "Skitze"}
```

- `from:` is a word or phrase as written, matched whole (not inside a
  longer word) and in its case; `regex:` is a Python regex, its groups
  `\1` or `\g<name>` in `to:` (and `\\` a backslash).
- A regex's `words: [2]` says its group 2's digits as number words, in a
  German list only: `{regex: '\b(ISO|DIN) (\d+)\b', to: '\1 \2', words: [2]}`
  gives "ISO viertausendsiebenhundertzweiundsechzig" for "ISO 4762". Below
  a million it is one word ("einhunderteins", "eintausend", a final 1
  "eins"), "eine Million", "zwei Milliarden" are words of their own; 0 is
  "null", a number over 999 999 999 999 stays digits.
- Each entry is applied in order to the text the ones before made: the
  engine's section after `all`. Without a voice (`--no-voice`), only `all`.
- They see the step's text as written, its Markdown gone, before the
  runner reads out units, sizes and signs ("8 mm", "M3", "×").
- What the voice is given is what its cache keeps a section by: an entry
  changes the key only of the sections it replaces in.
- A cue is written as the script is: its words get the same replacements
  before they are looked for in what is said.
- The subtitles show what is written. Each spoken word is found in the
  text the voice was given; the words of a replacement show its written
  words, at the times they were said.

Each run says how many replacements it made, and a `--dry-run` also looks
for every cue in what the voice would be given, so a replacement that
hides a cue's words stops the run before anything is spoken.

## An AI voice, said so

The narration is synthetic speech (Kokoro, ElevenLabs or Fish Audio S2
Pro). Whoever watches
is told so from the first frame (EU AI Act, Art. 50): a line at the bottom
of every card, and a small caption in the window's corner over the app.
The page says so under the screencast's line, and the video's metadata
does too (the file's comment and description, the sound's title). The line
names the voice: `Voice: AI-generated (Kokoro)`, `Voice: AI-generated
(ElevenLabs, George)`, `Voice: AI-generated (Fish Audio S2 Pro,
narrator)`; a script in another language gives its own as
`voice_notice` (`{voice}` in it is that name). A video without a voice
(`--no-voice`) has none.

The voice says so itself, too: first, over the first step's card, before
the step's own words, it says that it is an AI voice and why:

> This video is narrated by an AI voice, so it can always show the current
> version of {app}.

`{app}` is the script's `app:` or `--app-name` ("the app" without). A
script gives its own words (and language) as `voice_intro:`, a project as
`--voice-intro`; it is off only when said so: `voice_intro: false` or
`--no-voice-intro`. It is a section like any other, kept in the cache; the
first step's cues are looked for after it and are as much later as it is
long, so they still come with their words.
