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
`scripts/requirements.txt`, Python 3.12), plays it in the app and writes
`images/`, `README.md` and `<name>.mp4` (ffmpeg):

    python scripts/tutorial.py --app target/release/app \
        [--app-arg ARG]… [--env K=V]… [--tutorials docs/tutorials] \
        [--no-voice] [--no-video] [--window] [NAME…]

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

The narration is synthetic speech (Kokoro). Whoever watches is told so
from the first frame (EU AI Act, Art. 50): a line at the bottom of every
card, and a small caption in the window's corner over the app. The page
says so under the screencast's line, and the video's metadata does too
(the file's comment and description, the sound's title). The line is
`Voice: AI-generated (Kokoro)`; a script in another language gives its
own as `voice_notice`. A video without a voice (`--no-voice`) has none.
