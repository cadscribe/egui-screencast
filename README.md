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

## The runner

`scripts/tutorial.py` builds `<tutorials>/<name>/script.yaml` into the
plan, speaks the narration with Kokoro (`scripts/narrate.py`, pinned in
`scripts/requirements.txt`, Python 3.12), plays it in the app and writes
`images/`, `README.md` and `<name>.mp4` (ffmpeg):

    python scripts/tutorial.py --app target/release/app \
        [--app-arg ARG]… [--env K=V]… [--tutorials docs/tutorials] \
        [--no-voice] [--no-video] [--window] [NAME…]
