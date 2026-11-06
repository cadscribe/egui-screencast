//! Tutorials played and recorded by an egui app itself.
//!
//! A plan (JSON) is a list of steps, each a list of actions: clicks and
//! hovers on targets, keys, typing, waits, zooms, and the app's own actions
//! (calls of its tools, commands). The [`Player`] carries them out frame by
//! frame through the app's input, moves a pointer it draws itself the way a
//! hand does, and takes pictures of the app as it draws: a step's picture
//! once it has settled, and a video's frames. It writes into a folder:
//!
//! - `shots/<name>.png`: a step's picture, without the pointer, scaled to
//!   the plan's `shot_width`;
//! - `frames/<n>.png` and `frames.txt` (ffmpeg's concat list): the video,
//!   a frame equal to the last one written once and held longer;
//! - `timeline.json`: each step's first frame and number of frames, or the
//!   error that stopped the play.
//!
//! The clock is the frames': each frame is `1/fps` seconds later than the
//! last however long it took, so the video keeps the plan's pace on any
//! computer, and [`headless::run`] plays it without a window as fast as the
//! GPU draws, at a fixed size.
//!
//! The app says where its targets are, which list to scroll to bring one
//! into view, what its own actions do and when it is busy, by [`Stage`].
//! `scripts/` beside this crate writes plans from tutorial scripts, speaks
//! their narration with Kokoro and makes pages and videos of the pictures.

mod hand;
pub mod headless;
mod plan;
mod player;
mod writer;

pub use hand::hand_path;
pub use plan::{Action, Card, Plan, Step, Target, parse_plan, substitute};
pub use player::{Player, Stage};
