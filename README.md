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
plan, speaks the narration with Kokoro (`scripts/narrate.py`, pinned in
`scripts/requirements.txt`, Python 3.12) or ElevenLabs (below), plays it
in the app and writes `images/`, `README.md` and `<name>.mp4` (ffmpeg):

    python scripts/tutorial.py --app target/release/app \
        [--app-arg ARG]… [--env K=V]… [--tutorials docs/tutorials] \
        [--no-voice] [--no-video] [--window] [--app-name NAME] \
        [--voice elevenlabs:<voice id>] [--dry-run] [NAME…]

## Voices: Kokoro, ElevenLabs or Fish Audio S2 Pro

Kokoro is the default: local, free, the model at a fixed revision. The
voice is chosen by `--voice`, else `$SCREENCAST_VOICE`, else the script's
`voice:`; a bare name (`am_michael`) or `kokoro:<name>` is Kokoro,
`elevenlabs:<voice id>` is ElevenLabs, `fish-s2-pro:<reference.wav>` is
Fish Audio S2 Pro run locally (below):

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

**The cache.** Every section said (a step's text, the voice's own notice)
is kept by a key: the SHA-256 of the backend, the voice id, the model, the
voice settings, the output format and the text as said (after `spoken()`,
its whitespace made single spaces). A section whose key is in the cache is
never asked for again; only new or changed text is sent. An entry is a
folder (`audio.wav`, `words.json`, `meta.json`) made whole and then renamed
into place, so a failed request (no key, HTTP 401, 429, …) leaves nothing
behind, and the sections spoken before it are kept. It lives in
`<work>/voices` (`--voice-cache DIR`, `$SCREENCAST_VOICE_CACHE`), with
Fish S2 Pro's (their keys say which backend),
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

[S2 Pro](https://huggingface.co/fishaudio/s2-pro) (5B parameters, open
weights) runs on the computer, on Apple Silicon on the GPU by MPS, else
the CPU: nothing is sent anywhere, nothing is billed, but it is slow and
needs memory (the weights are about 11 GB in bf16; 32 GB of RAM or more).
On an M1 Max (64 GB, busy with other work) loading took 3 minutes and a
sentence of 7.5 s took 107 s (1.5 tokens a second, about 14 times the
speech's length): an hour or more for a tutorial the first time. Every
section is cached like ElevenLabs' (the key: backend, model and
fish-speech at their revisions, the aligner, the reference recording's
bytes and words, the settings, the output format, the text), so only
what changed is spoken again.

**Licence.** S2 Pro's weights are under the **Fish Audio Research
License: research and non-commercial use only**; commercial use needs a
licence from Fish Audio (business@fish.audio). Whether a tutorial video
is non-commercial is yours to judge. The notices say it: the card line
and corner name `Fish Audio S2 Pro`, the page names the licence, the
MP4's comment and its sound's title too. The aligner
(facebook/wav2vec2-base-960h) is Apache 2.0.

**Its venv.** fish-speech pins its own torch (2.8.0) and transformers, so
it gets a venv of its own beside the tutorial one, from
`scripts/requirements-fish.txt` (fish-speech's own lock at a fixed commit,
installed without resolving again):

    uv venv -p 3.12 target/tutorials/fish-venv
    VIRTUAL_ENV=target/tutorials/fish-venv uv pip install --no-deps -r scripts/requirements-fish.txt

The runner starts `scripts/fishspeech_worker.py` with that Python
(`--fish-python`, `$FISH_SPEECH_PYTHON`, else
`<work>/fish-venv/bin/python`) on the first section not in the cache; it
loads the model once (from Hugging Face on first use, into `<work>/hf`)
and speaks one section after the other.

**The voice** is a reference recording: a WAV of 10 to 30 seconds, and
what is said in it beside it (`narrator.lab` or `.txt`, or the script's
`voice_reference_text`). Without one S2 Pro would choose another voice for
each section. Use a voice you may use: your own, a licensed one, or one
made by Kokoro:

    python scripts/narrate.py "Hello, I am the narrator of these tutorials. …" voices/narrator.wav
    echo "Hello, I am the narrator of these tutorials. …" > voices/narrator.lab
    python scripts/tutorial.py --app … --voice fish-s2-pro:voices/narrator.wav

`voice_settings:` (or `--voice-setting K=V`) sets `seed` (42: the same
text, the same sound), `temperature`, `top_p` (0.7 each), `chunk_length`
(200) and `normalize` (true). The script's `speed` does not apply.

**Word times.** S2 Pro run locally gives none. They are found by forced
alignment (`scripts/align.py`): wav2vec2-base-960h (English letters, a
frame each 20 ms) says how likely each letter is in each frame of the
speech, and the most likely path through the text's letters in order gives
each word its first and last frame; numbers are spelled for it
(num2words). A word's start is right to about a frame or two (20 to
40 ms), which is far finer than a cue needs. Should the path not be found
(a text not in English, say), the times are spread by the words' lengths,
and cues are then only roughly right.

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

`shot: true` takes the step's picture where it stands in the actions: the
moment that shows what the page's text says, rather than whatever the step
ends on.

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
