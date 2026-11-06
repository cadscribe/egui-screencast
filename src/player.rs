//! The player: a plan carried out frame by frame, the app's pictures taken.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, pos2};
use serde_json::{Value, json};

use crate::hand::hand_path;
use crate::plan::{Action, Card, Plan, Target, parse_plan};
use crate::writer::{Frames, Job, Writer};

/// Frames before the first step: fonts, layout and the first build.
const WARM: u32 = 12;
/// Seconds after an action before the next: the view settles.
const SETTLE: f64 = 0.35;
/// Seconds the pointer takes to a target, about.
const GLIDE: f64 = 0.55;
/// Seconds a zoom takes.
const ZOOM: f64 = 0.6;
/// Seconds an action of the app's is shown in the video.
const SHOWN: f64 = 3.0;
/// Seconds an action may take before the play gives up.
const PATIENCE: f64 = 60.0;
/// Seconds a card takes to show and to go.
const FADE: f64 = 0.4;
/// Points from the window's right and bottom to the caption's box.
const CAPTION_MARGIN: f32 = 16.0;
const CAPTION_BOTTOM: f32 = 84.0;

/// What the player needs of the app.
pub trait Stage {
    /// Where one of the app's targets is in the window now (`{"ribbon":
    /// "New"}`, `{"field": "Depth"}`), if it is shown.
    fn locate(&self, target: &Value) -> Option<Pos2>;

    /// The part of a scrolling list a target is in that is in view, if it
    /// is in one: the player turns the wheel over it until the target is in
    /// view, before the pointer goes there.
    fn scroll_view(&self, target: &Value) -> Option<Rect> {
        let _ = target;
        None
    }

    /// One of the app's own actions (`{"tool": …}`, `{"command": …}`),
    /// done at once; what to show for it in the video, if anything. `kept`
    /// holds what actions before returned (see [`crate::substitute`]).
    ///
    /// # Errors
    ///
    /// The action is unknown or failed: the play stops with the error.
    fn act(
        &mut self,
        ctx: &egui::Context,
        action: &Value,
        seconds: f64,
        kept: &mut serde_json::Map<String, Value>,
    ) -> Result<Option<String>, String>;

    /// Whether the app is still working after an action (things computed
    /// in the background): the play waits.
    fn busy(&mut self) -> bool {
        false
    }

    /// Before a step's picture: what does not belong in it hidden.
    fn before_shot(&mut self) {}
}

/// Where the play is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Warm(u32),
    /// The action of the step at work, since that clock.
    Act {
        started: u64,
    },
    /// After an action, until that clock.
    Settle {
        until: u64,
    },
    /// The step's picture asked for in the frame before. A picture
    /// shows the frame it was asked in or the next one, so neither has
    /// the pointer or is a frame of the video. Asked for by an action
    /// (`resume`: the step goes on with the next one) or at the step's end.
    Shot {
        resume: bool,
    },
    /// Until the step has its seconds of video.
    Hold,
    /// Waiting for the last pictures.
    Finish,
}

/// What happened to a step, for `timeline.json`.
#[derive(Debug, Clone)]
struct Logged {
    name: String,
    start: u32,
    frames: u32,
    /// The frame of each of its cues, from its start: where the runner
    /// puts the narration's words from that cue on.
    cues: Vec<u32>,
}

/// A plan being played and recorded.
pub struct Player {
    plan: Plan,
    out: PathBuf,
    /// Who the app's actions shown are made as ("Assistant"), and its
    /// colour.
    who: (String, Color32),
    step: usize,
    action: usize,
    phase: Phase,
    /// Passes so far, the clock: each is `1/fps` seconds.
    clock: u64,
    /// Frames of video so far.
    video_frames: u32,
    step_start: u32,
    /// Frames the step's narration is behind its plan: actions before a
    /// cue took longer than the words before it, and the words wait.
    late: u32,
    /// The step's cues so far, in frames from its start.
    cues: Vec<u32>,
    /// The step's picture was taken by an action.
    shot_taken: bool,
    kept: serde_json::Map<String, Value>,
    /// Where the plan's pointer is, and where it started its glide.
    pointer: Option<Pos2>,
    from: Option<Pos2>,
    /// The frames of a zoom given so far.
    zoomed_to: Option<u64>,
    /// When it last clicked, for the ring.
    clicked: Option<u64>,
    /// The last of the app's actions shown and when.
    called: Option<(String, u64)>,
    /// Events of the coming frames, one set a frame, with the keys held.
    events: VecDeque<(Vec<egui::Event>, egui::Modifiers)>,
    frames: Arc<Mutex<Frames>>,
    writer: Writer,
    log: Vec<Logged>,
    error: Option<String>,
    /// The cards' logos, loaded once (none: it could not be read).
    logos: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("out", &self.out)
            .field("step", &self.step)
            .field("action", &self.action)
            .field("phase", &self.phase)
            .field("clock", &self.clock)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Player {
    /// The plan at `plan`, recorded into `out`; the app's actions shown as
    /// made by `who` in `colour`.
    ///
    /// # Errors
    ///
    /// The plan cannot be read or the folders made.
    pub fn load(plan: &Path, out: &Path, who: &str, colour: Color32) -> Result<Player, String> {
        let text = std::fs::read_to_string(plan).map_err(|e| format!("{}: {e}", plan.display()))?;
        let plan = parse_plan(&text)?;
        for dir in ["shots", "frames"] {
            let d = out.join(dir);
            if d.exists() {
                std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        Ok(Player {
            plan,
            out: out.to_path_buf(),
            who: (who.to_owned(), colour),
            step: 0,
            action: 0,
            phase: Phase::Warm(0),
            clock: 0,
            video_frames: 0,
            step_start: 0,
            late: 0,
            cues: Vec::new(),
            shot_taken: false,
            kept: serde_json::Map::new(),
            pointer: None,
            from: None,
            zoomed_to: None,
            clicked: None,
            called: None,
            events: VecDeque::new(),
            frames: Arc::default(),
            writer: Writer::new(),
            log: Vec::new(),
            error: None,
            logos: HashMap::new(),
        })
    }

    /// The error that stopped the play, if one did.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn seconds(&self, frames: u64) -> f64 {
        frames as f64 / self.plan.fps
    }

    fn frames_of(&self, seconds: f64) -> u64 {
        (seconds * self.plan.fps).round() as u64
    }

    /// The window's input replaced by the plan's (eframe's
    /// `raw_input_hook`): its clock, its events, the keys it holds.
    pub fn input(&mut self, raw: &mut egui::RawInput) {
        let (events, modifiers) = self.events.pop_front().unwrap_or_default();
        raw.events = vec![egui::Event::ModifiersChanged(modifiers)];
        raw.events.extend(events);
        raw.time = Some(self.seconds(self.clock));
        raw.predicted_dt = (1.0 / self.plan.fps) as f32;
        raw.hovered_files.clear();
        raw.dropped_files.clear();
    }

    /// Queues events for the coming frames, one set a frame.
    fn queue(
        &mut self,
        frames: impl IntoIterator<Item = Vec<egui::Event>>,
        modifiers: egui::Modifiers,
    ) {
        self.events
            .extend(frames.into_iter().map(|f| (f, modifiers)));
    }

    fn fail(&mut self, e: String) {
        eprintln!("play: step {}: {e}", self.step + 1);
        self.error = Some(format!(
            "step {} ({}): {e}",
            self.step + 1,
            self.plan
                .steps
                .get(self.step)
                .map_or("", |s| s.name.as_str())
        ));
        self.phase = Phase::Finish;
    }

    fn locate(stage: &dyn Stage, target: &Target) -> Option<Pos2> {
        match target {
            Target::At([x, y]) => Some(pos2(*x, *y)),
            Target::App(v) => stage.locate(v),
        }
    }

    /// Starts an action; `true` when it is done at once.
    fn start(&mut self, stage: &mut dyn Stage, ctx: &egui::Context, action: &Action) -> bool {
        match action {
            Action::Click { .. } | Action::Zoom { .. } => {
                self.from = self.pointer;
                false
            }
            Action::Key { key, modifiers } => {
                let k = |pressed| egui::Event::Key {
                    key: *key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: *modifiers,
                };
                self.queue([vec![k(true)], vec![k(false)]], *modifiers);
                true
            }
            Action::Type(text) => {
                // What the field holds selected first (⌘A), so that the
                // typing replaces it; then a character a frame.
                let all = egui::Modifiers::COMMAND;
                let a = |pressed| egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: all,
                };
                self.queue([vec![a(true)], vec![a(false)]], all);
                let chars: Vec<Vec<egui::Event>> = text
                    .chars()
                    .map(|c| vec![egui::Event::Text(c.to_string())])
                    .collect();
                self.queue(chars, egui::Modifiers::NONE);
                true
            }
            Action::Wait(_) | Action::Cue(_) | Action::Shot => false,
            Action::App(v) => {
                let now = self.seconds(self.clock);
                let mut kept = std::mem::take(&mut self.kept);
                let done = stage.act(ctx, v, now, &mut kept);
                self.kept = kept;
                match done {
                    Ok(Some(line)) => self.called = Some((line, self.clock)),
                    Ok(None) => {}
                    Err(e) => self.fail(e),
                }
                true
            }
        }
    }

    /// One frame of an action that takes time; `true` once it is done.
    fn tick(&mut self, stage: &dyn Stage, action: &Action, started: u64) -> bool {
        let t = self.seconds(self.clock - started);
        match action {
            Action::Wait(s) => t >= *s,
            Action::Cue(at) => {
                // Without a video there is no narration to wait for.
                if !self.plan.video {
                    return true;
                }
                let planned = self.frames_of(*at) as u32;
                let elapsed = self.video_frames - self.step_start;
                if elapsed < planned + self.late {
                    return false;
                }
                // Come to late: the words from here on wait as long.
                self.late = elapsed - planned;
                self.cues.push(elapsed);
                true
            }
            Action::Zoom { factor, at } => {
                let Some(to) = Self::locate(stage, at) else {
                    self.fail(format!("{at:?} is not shown"));
                    return true;
                };
                let glide = self.glide_secs(to);
                if t < glide {
                    self.glide(to, t);
                    return false;
                }
                // Spread over the frames of `ZOOM` seconds.
                let n = self.frames_of(ZOOM).max(1);
                let k = self.frames_of(t - glide);
                if k < n {
                    // The share of the zoom that the frames up to this one
                    // have, less what was given: the whole however many
                    // frames there are, and none dropped.
                    let done = |j: u64| factor.powf(j.min(n) as f32 / n as f32);
                    let given = *self.zoomed_to.get_or_insert(0);
                    let step = done(k + 1) / done(given);
                    self.zoomed_to = Some(k + 1);
                    self.queue(
                        [vec![egui::Event::PointerMoved(to), egui::Event::Zoom(step)]],
                        egui::Modifiers::NONE,
                    );
                    return false;
                }
                // The rest, should a frame have been skipped.
                let given = self.zoomed_to.take().unwrap_or(n);
                if given < n {
                    let rest = factor.powf((n - given) as f32 / n as f32);
                    self.queue(
                        [vec![egui::Event::PointerMoved(to), egui::Event::Zoom(rest)]],
                        egui::Modifiers::NONE,
                    );
                }
                true
            }
            Action::Click {
                target,
                shift,
                hover,
            } => {
                // In a list, out of its view: the wheel turned over it, as
                // a hand does, before the pointer goes there.
                let view = match target {
                    Target::App(v) => stage.scroll_view(v),
                    Target::At(_) => None,
                };
                if let (Some(to), Some(view)) = (Self::locate(stage, target), view)
                    && !view.shrink(12.0).contains(to)
                {
                    let dy = (view.center().y - to.y).clamp(-120.0, 120.0);
                    self.queue(
                        [vec![
                            egui::Event::PointerMoved(view.center()),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, dy),
                                phase: egui::TouchPhase::Move,
                                source: egui::MouseWheelSource::Unknown,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]],
                        egui::Modifiers::NONE,
                    );
                    self.pointer = Some(view.center());
                    self.from = self.pointer;
                    return false;
                }
                let Some(to) = Self::locate(stage, target) else {
                    if t > 2.0 {
                        self.fail(format!("{target:?} is not shown"));
                        return true;
                    }
                    return false;
                };
                if t < self.glide_secs(to) {
                    self.glide(to, t);
                    return false;
                }
                self.pointer = Some(to);
                if !hover {
                    let modifiers = egui::Modifiers {
                        shift: *shift,
                        ..Default::default()
                    };
                    let button = |pressed| egui::Event::PointerButton {
                        pos: to,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    };
                    self.events.clear();
                    self.queue([vec![button(true)], vec![button(false)]], modifiers);
                    self.clicked = Some(self.clock);
                }
                true
            }
            _ => true,
        }
    }

    /// Seconds the pointer takes to `to`: longer the further, as a hand.
    fn glide_secs(&self, to: Pos2) -> f64 {
        let from = self.from.unwrap_or_else(|| to + egui::vec2(-120.0, 160.0));
        let d = f64::from(from.distance(to));
        (GLIDE * (0.6 + 0.25 * (1.0 + d / 80.0).log2())).clamp(0.3, 1.1)
    }

    /// The pointer on its way to `to`, `t` seconds after it set off.
    fn glide(&mut self, to: Pos2, t: f64) {
        let from = self.from.unwrap_or_else(|| to + egui::vec2(-120.0, 160.0));
        let k = (t / self.glide_secs(to)).min(1.0) as f32;
        let at = hand_path(from, to, k);
        self.pointer = Some(at);
        self.queue([vec![egui::Event::PointerMoved(at)]], egui::Modifiers::NONE);
    }

    /// The app's action done last, for a few seconds: what changed where
    /// no click did.
    fn paint_call(&self, ctx: &egui::Context, painter: &egui::Painter) {
        let Some((line, when)) = &self.called else {
            return;
        };
        let t = self.seconds(self.clock - when);
        if t > SHOWN {
            return;
        }
        let fade = ((SHOWN - t) / 0.4).min(1.0) as f32;
        let (who, colour) = &self.who;
        let font = egui::FontId::proportional(13.0);
        let text = format!("{who}  ·  {line}");
        let galley =
            painter.layout_no_wrap(text, font, Color32::from_gray(30).gamma_multiply(fade));
        let screen = ctx.content_rect();
        let size = galley.size() + egui::vec2(40.0, 16.0);
        let rect = egui::Rect::from_center_size(
            pos2(screen.center().x + 180.0, screen.bottom() - 70.0),
            size,
        );
        painter.rect(
            rect,
            8.0,
            Color32::WHITE.gamma_multiply(0.96 * fade),
            Stroke::new(1.0, colour.gamma_multiply(fade)),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(
            pos2(rect.left() + 16.0, rect.center().y),
            4.5,
            colour.gamma_multiply(fade),
        );
        painter.galley(
            pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
            galley,
            Color32::from_gray(30),
        );
    }

    /// The plan's pointer, and a ring where it clicked; the action shown.
    fn paint_pointer(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            egui::Id::unique("screencast_pointer"),
        ));
        self.paint_call(ctx, &painter);
        let Some(at) = self.pointer else {
            return;
        };
        if let Some(c) = self.clicked {
            let t = self.seconds(self.clock - c) / 0.45;
            if t < 1.0 {
                let a = ((1.0 - t) * 200.0) as u8;
                painter.circle_stroke(
                    at,
                    8.0 + 22.0 * t as f32,
                    Stroke::new(3.0, Color32::from_rgba_unmultiplied(59, 130, 246, a)),
                );
            }
        }
        // An arrow, white with a dark edge, as the system's.
        let tip = [
            (0.0, 0.0),
            (0.0, 17.0),
            (4.2, 13.2),
            (7.2, 19.8),
            (9.8, 18.6),
            (6.9, 12.2),
            (12.2, 12.2),
        ];
        let points: Vec<Pos2> = tip.iter().map(|&(x, y)| at + egui::vec2(x, y)).collect();
        painter.add(egui::Shape::convex_polygon(
            points.clone(),
            Color32::WHITE,
            Stroke::NONE,
        ));
        painter.add(egui::Shape::closed_line(
            points,
            Stroke::new(1.2, Color32::from_gray(20)),
        ));
    }

    /// How much the step's card shows now, 0 to 1: it comes in over the
    /// app (at once on the first step, so the video opens on it) and goes
    /// at the end of its narration (not on the last step, so the video ends
    /// on it).
    fn card_shown(&self) -> f32 {
        let fade = self.frames_of(FADE).max(1) as f32;
        let elapsed = (self.video_frames - self.step_start) as f32;
        let first = self.step == 0;
        let last = self.step + 1 == self.plan.steps.len();
        let come = if first { 1.0 } else { elapsed / fade };
        let go = match self.phase {
            Phase::Hold if !last => {
                let want = self.step_frames() as f32;
                (want - elapsed) / fade
            }
            _ => 1.0,
        };
        come.min(go).clamp(0.0, 1.0)
    }

    fn logo(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        self.logos
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let img = image::open(path)
                    .map_err(|e| eprintln!("play: {}: {e}", path.display()))
                    .ok()?
                    .to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(ctx.load_texture(
                    format!("screencast_logo {}", path.display()),
                    image,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .clone()
    }

    /// The step's card over the whole window, if it has one; how much it
    /// shows.
    fn paint_card(&mut self, ctx: &egui::Context) -> f32 {
        let Some(card) = self.plan.steps.get(self.step).and_then(|s| s.card.clone()) else {
            return 0.0;
        };
        let k = self.card_shown();
        if k <= 0.0 {
            return 0.0;
        }
        let logo = card.logo.as_ref().and_then(|p| self.logo(ctx, p));
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("screencast_card"),
        ));
        // Its parts come in once the card is half there (at once on the
        // first step).
        let elapsed = self.seconds(u64::from(self.video_frames - self.step_start));
        let t = if self.step == 0 {
            elapsed
        } else {
            elapsed - FADE / 2.0
        };
        paint_card(
            &painter,
            ctx.content_rect(),
            &card,
            logo.as_ref(),
            self.plan.caption.as_deref(),
            k,
            t as f32,
        );
        k
    }

    /// The plan's caption in the corner, over the app: that the voice is
    /// synthetic. `k` of it shown (the card shows its own).
    fn paint_caption(&self, ctx: &egui::Context, k: f32) {
        let Some(text) = self.plan.caption.as_deref() else {
            return;
        };
        if k <= 0.0 {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Debug,
            egui::Id::unique("screencast_caption"),
        ));
        let galley = painter.layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(11.0),
            Color32::from_gray(60).gamma_multiply(k),
        );
        // Above the bottom's status line and timeline, at the right, where
        // an app seldom has what a tutorial points at.
        let screen = ctx.content_rect();
        let size = galley.size() + egui::vec2(16.0, 8.0);
        let rect = Rect::from_min_size(
            pos2(
                screen.right() - CAPTION_MARGIN - size.x,
                screen.bottom() - CAPTION_BOTTOM - size.y,
            ),
            size,
        );
        painter.rect_filled(rect, 6.0, Color32::WHITE.gamma_multiply(0.82 * k));
        painter.galley(
            rect.min + egui::vec2(8.0, 4.0),
            galley,
            Color32::from_gray(60),
        );
    }

    /// Asks for this frame's picture: a frame of the video, or `shot`.
    fn capture(&mut self, ctx: &egui::Context, shot: Option<String>) {
        let frames = self.frames.clone();
        if let Ok(mut f) = frames.lock() {
            f.pending += 1;
        }
        let out = self.out.clone();
        let tx = self.writer.tx.clone();
        let width = self.plan.shot_width;
        let index = self.video_frames;
        ctx.request_screenshot(move |image| {
            let Ok(mut f) = frames.lock() else {
                return;
            };
            f.pending = f.pending.saturating_sub(1);
            let send = |job| {
                if let Some(tx) = &tx {
                    let _ = tx.send(job);
                }
            };
            if let Some(name) = shot {
                send(Job {
                    image,
                    path: out.join("shots").join(name),
                    width: Some(width),
                });
                return;
            }
            f.size = image.size;
            let same = f.last.as_ref().is_some_and(|l| l.pixels == image.pixels);
            match f.files.last_mut() {
                Some((_, n)) if same => *n += 1,
                _ => {
                    let name = format!("{index:06}.png");
                    f.files.push((name.clone(), 1));
                    f.last = Some(image.clone());
                    send(Job {
                        image,
                        path: out.join("frames").join(name),
                        width: None,
                    });
                }
            }
        });
    }

    /// Ends the play: the last pictures, `timeline.json`, `frames.txt`.
    fn finish(&mut self) -> Result<(), String> {
        let mut errors = self.writer.finish();
        let f = self.frames.lock().map_err(|e| e.to_string())?;
        let dt = 1.0 / self.plan.fps;
        let mut list = String::from("ffconcat version 1.0\n");
        for (name, n) in &f.files {
            list += &format!("file 'frames/{name}'\nduration {:.6}\n", f64::from(*n) * dt);
        }
        // The concat demuxer takes the last file's duration only when it
        // is named once more.
        if let Some((name, _)) = f.files.last() {
            list += &format!("file 'frames/{name}'\n");
        }
        let write = |name: &str, text: &str| {
            let p = self.out.join(name);
            std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
        };
        if self.plan.video {
            write("frames.txt", &list)?;
        }
        errors.extend(self.error.clone());
        let steps: Vec<Value> = self
            .log
            .iter()
            .map(|l| json!({"name": l.name, "start": l.start, "frames": l.frames, "cues": l.cues}))
            .collect();
        let timeline = json!({
            "fps": self.plan.fps,
            "frames": self.video_frames,
            "size": f.size,
            "steps": steps,
            "errors": errors,
        });
        write(
            "timeline.json",
            &serde_json::to_string_pretty(&timeline).unwrap_or_default(),
        )
    }

    /// The action the play is at.
    fn current(&self) -> Option<Action> {
        self.plan
            .steps
            .get(self.step)?
            .actions
            .get(self.action)
            .cloned()
    }

    /// Once a frame, after the app was drawn: the plan goes on. `true` when
    /// it is over and the app may close.
    pub fn frame(&mut self, stage: &mut dyn Stage, ctx: &egui::Context) -> bool {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        ctx.request_repaint();
        match self.phase {
            Phase::Warm(n) if n < WARM => {
                self.phase = Phase::Warm(n + 1);
                self.clock += 1;
                return false;
            }
            Phase::Warm(_) => self.begin_step(stage, ctx),
            Phase::Act { started } => {
                if let Some(a) = self.current() {
                    if a == Action::Shot {
                        // Its picture now; the step goes on after it.
                        self.take_shot(stage, ctx, true);
                        return false;
                    }
                    // A failed action has ended the play: it stays ended.
                    if self.tick(stage, &a, started) && self.error.is_none() {
                        // A cue is a moment, nothing to settle after.
                        let settle = if matches!(a, Action::Cue(_)) {
                            0
                        } else {
                            self.frames_of(SETTLE)
                        };
                        self.phase = Phase::Settle {
                            until: self.clock + settle,
                        };
                    } else if self.seconds(self.clock - started) > PATIENCE {
                        self.fail(format!("{a:?} took longer than {PATIENCE} s"));
                    }
                }
            }
            Phase::Settle { until } => {
                let busy = stage.busy() || !self.events.is_empty();
                if self.clock >= until && !busy {
                    self.action += 1;
                    if self.current().is_some() {
                        self.begin_action(stage, ctx);
                    } else if !self.shot_taken && self.plan.steps[self.step].shot.is_some() {
                        self.take_shot(stage, ctx, false);
                        return false;
                    } else {
                        self.phase = Phase::Hold;
                    }
                }
            }
            Phase::Shot { resume } => {
                self.phase = if resume {
                    Phase::Settle { until: self.clock }
                } else {
                    Phase::Hold
                };
                self.clock += 1;
                return false;
            }
            Phase::Hold => {
                if self.video_frames - self.step_start >= self.step_frames() || !self.plan.video {
                    self.log.push(Logged {
                        name: self.plan.steps[self.step].name.clone(),
                        start: self.step_start,
                        frames: self.video_frames - self.step_start,
                        cues: std::mem::take(&mut self.cues),
                    });
                    self.step += 1;
                    if self.step >= self.plan.steps.len() {
                        self.phase = Phase::Finish;
                        return false;
                    }
                    self.begin_step(stage, ctx);
                }
            }
            Phase::Finish => {
                let pending = self.frames.lock().map_or(0, |f| f.pending);
                if pending == 0 {
                    if let Err(e) = self.finish() {
                        eprintln!("play: {e}");
                    }
                    return true;
                }
                self.clock += 1;
                return false;
            }
        }
        if self.error.is_some() {
            return false;
        }
        let shown = self.paint_card(ctx);
        if shown < 1.0 {
            self.paint_pointer(ctx);
        }
        self.paint_caption(ctx, 1.0 - shown);
        if self.plan.video {
            self.capture(ctx, None);
            self.video_frames += 1;
        }
        self.clock += 1;
        false
    }

    /// Frames the step lasts at least: its narration and pause, and as
    /// long again as its cues made the narration wait.
    fn step_frames(&self) -> u32 {
        self.frames_of(self.plan.steps[self.step].seconds) as u32 + self.late
    }

    /// Asks for the step's picture in this frame, which is then no frame of
    /// the video: by an action (`resume`: the step goes on) or at the
    /// step's end. A step without a picture (`shot: false`) takes none, and
    /// none twice.
    fn take_shot(&mut self, stage: &mut dyn Stage, ctx: &egui::Context, resume: bool) {
        if let Some(name) = self.plan.steps[self.step].shot.clone()
            && !self.shot_taken
        {
            stage.before_shot();
            self.capture(ctx, Some(name));
            self.shot_taken = true;
        }
        self.phase = Phase::Shot { resume };
        self.clock += 1;
    }

    fn begin_step(&mut self, stage: &mut dyn Stage, ctx: &egui::Context) {
        self.action = 0;
        self.step_start = self.video_frames;
        self.late = 0;
        self.cues.clear();
        self.shot_taken = false;
        self.begin_action(stage, ctx);
    }

    /// Starts the action the play is at; one done at once settles.
    fn begin_action(&mut self, stage: &mut dyn Stage, ctx: &egui::Context) {
        self.phase = Phase::Act {
            started: self.clock,
        };
        let Some(a) = self.current() else {
            // A step without actions: its picture, then its narration.
            self.phase = Phase::Settle { until: self.clock };
            return;
        };
        if self.start(stage, ctx, &a) && self.error.is_none() {
            self.phase = Phase::Settle {
                until: self.clock + self.frames_of(SETTLE),
            };
        }
    }
}

/// How far a card's part has come in, 0 to 1, `t` seconds after the card
/// began to show: from `from`, over `CARD_IN` seconds, fast then slow.
fn coming(t: f32, from: f32) -> f32 {
    let x = ((t - from) / CARD_IN).clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// When a card's parts come in, seconds after it began to show: the logo,
/// the title, the subtitle, the accent's rule under them, the first point
/// (the others each `CARD_POINT` later); the link after the last of them.
const CARD_LOGO: f32 = 0.0;
const CARD_TITLE: f32 = 0.3;
const CARD_SUBTITLE: f32 = 0.55;
const CARD_RULE: f32 = 0.8;
const CARD_POINTS: f32 = 1.1;
const CARD_POINT: f32 = 0.45;
/// Seconds a part of a card takes to come in.
const CARD_IN: f32 = 0.7;

/// A card in `screen`, `k` of it shown, `t` seconds after it began to show:
/// its logo, title and subtitle, its points with the accent's dots, its
/// link, and the plan's caption at the bottom. With points it reads from
/// the left, without (an ending) it is centred. The parts come in one
/// after the other, each fading in and moving a little into its place.
fn paint_card(
    painter: &egui::Painter,
    screen: Rect,
    card: &Card,
    logo: Option<&egui::TextureHandle>,
    caption: Option<&str>,
    k: f32,
    t: f32,
) {
    painter.rect_filled(screen, 0.0, card.background.gamma_multiply(k));
    let white = Color32::WHITE.gamma_multiply(k);
    let soft = Color32::from_white_alpha(200).gamma_multiply(k);
    let accent = card.accent.gamma_multiply(k);
    // A galley in `colour`, `a` of it come in: faded, and moved by `(dx,
    // dy)` times what is still to come.
    let place =
        |at: Pos2, g: std::sync::Arc<egui::Galley>, colour: Color32, a: f32, dx: f32, dy: f32| {
            if a > 0.0 {
                let at = at + egui::vec2(dx, dy) * (1.0 - a);
                painter.add(egui::Shape::galley_with_override_text_color(
                    at,
                    g,
                    colour.gamma_multiply(a),
                ));
            }
        };
    let width = (screen.width() - 160.0).min(780.0);
    let centred = card.points.is_empty();
    let layout = |text: &str, size: f32, colour: Color32, wrap: f32| {
        let mut job = egui::text::LayoutJob::simple(
            text.to_owned(),
            egui::FontId::proportional(size),
            colour,
            wrap,
        );
        if centred {
            job.halign = egui::Align::Center;
        }
        painter.layout_job(job)
    };
    let title = layout(&card.title, 40.0, white, width);
    let subtitle = card
        .subtitle
        .as_deref()
        .map(|t| layout(t, 22.0, soft, width));
    let points: Vec<_> = card
        .points
        .iter()
        .map(|p| layout(p, 22.0, white, width - 32.0))
        .collect();
    let link = card.link.as_deref().map(|t| layout(t, 20.0, accent, width));
    let logo_size = if centred { 160.0 } else { 88.0 };
    // The whole height, to centre it.
    let mut height = title.size().y;
    if logo.is_some() {
        height += logo_size + 28.0;
    }
    if let Some(g) = &subtitle {
        height += 12.0 + g.size().y;
    }
    if !points.is_empty() {
        height += 36.0 + points.iter().map(|g| g.size().y + 14.0).sum::<f32>() - 14.0;
    }
    if let Some(g) = &link {
        height += 44.0 + g.size().y;
    }
    let left = screen.center().x - width / 2.0;
    // Where a galley's left goes: centred galleys are laid out about their
    // own centre.
    let x = |g: &egui::Galley| {
        if centred {
            screen.center().x - g.size().x / 2.0 - g.rect.left()
        } else {
            left
        }
    };
    let mut y = screen.center().y - height / 2.0;
    if let Some(tex) = logo {
        // Grows a little to its size about its centre as it fades in.
        let a = coming(t, CARD_LOGO);
        let lx = if centred {
            screen.center().x - logo_size / 2.0
        } else {
            left
        };
        let r = Rect::from_min_size(pos2(lx, y), egui::vec2(logo_size, logo_size));
        if a > 0.0 {
            painter.image(
                tex.id(),
                r.scale_from_center(0.9 + 0.1 * a),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(k * a),
            );
        }
        y += logo_size + 28.0;
    }
    let h = title.size().y;
    place(
        pos2(x(&title), y),
        title,
        white,
        coming(t, CARD_TITLE),
        0.0,
        16.0,
    );
    y += h;
    if let Some(g) = subtitle {
        y += 12.0;
        let h = g.size().y;
        place(pos2(x(&g), y), g, soft, coming(t, CARD_SUBTITLE), 0.0, 12.0);
        y += h;
    }
    // The accent's rule under the titles, drawn out from its start (from
    // its middle when centred).
    let gap = if points.is_empty() { 44.0 } else { 36.0 };
    let rule = coming(t, CARD_RULE) * 56.0;
    if rule > 0.5 && (!points.is_empty() || link.is_some()) {
        let ry = y + gap / 2.0;
        let (x0, x1) = if centred {
            (
                screen.center().x - rule / 2.0,
                screen.center().x + rule / 2.0,
            )
        } else {
            (left, left + rule)
        };
        painter.line_segment([pos2(x0, ry), pos2(x1, ry)], Stroke::new(3.0, accent));
    }
    // The link after the last point.
    let mut next = CARD_POINTS;
    if !points.is_empty() {
        y += 36.0;
        for g in points {
            let a = coming(t, next);
            next += CARD_POINT;
            let row = g.rows.first().map_or(g.size().y, |r| r.rect().height());
            if a > 0.0 {
                painter.circle_filled(
                    pos2(left + 8.0, y + row / 2.0),
                    6.0 * (0.5 + 0.5 * a),
                    accent.gamma_multiply(a),
                );
            }
            let h = g.size().y;
            place(pos2(left + 32.0, y), g, white, a, -14.0, 0.0);
            y += h + 14.0;
        }
        y -= 14.0;
    }
    if let Some(g) = link {
        y += 44.0;
        place(pos2(x(&g), y), g, accent, coming(t, next + 0.1), 0.0, 10.0);
    }
    // The caption, at once: what a viewer is to know from the start.
    if let Some(c) = caption {
        let g = painter.layout_no_wrap(c.to_owned(), egui::FontId::proportional(14.0), soft);
        let at = pos2(
            screen.center().x - g.size().x / 2.0,
            screen.bottom() - 28.0 - g.size().y,
        );
        place(at, g, soft, coming(t, 0.0), 0.0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;

    impl Stage for Nothing {
        fn locate(&self, _: &Value) -> Option<Pos2> {
            None
        }

        fn act(
            &mut self,
            _: &egui::Context,
            _: &Value,
            _: f64,
            _: &mut serde_json::Map<String, Value>,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    /// The plan played until its steps are logged (its pictures are never
    /// written without a renderer, so it does not finish).
    fn play(name: &str, plan: &str) -> Player {
        let dir = std::env::temp_dir().join(format!(
            "egui-screencast-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plan.json");
        std::fs::write(&path, plan).unwrap();
        let mut player = Player::load(&path, &dir, "Assistant", Color32::BLUE).unwrap();
        let ctx = egui::Context::default();
        let steps = player.plan.steps.len();
        for _ in 0..2000 {
            let mut raw = egui::RawInput::default();
            player.input(&mut raw);
            let mut out = ctx.run_ui(raw, |ui| {
                player.frame(&mut Nothing, ui.ctx());
            });
            out.textures_delta.clear();
            if player.log.len() == steps {
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(player.log.len(), steps, "{:?}", player.error());
        player
    }

    #[test]
    fn a_cue_waits_for_its_words_and_the_words_for_a_late_cue() {
        // Two seconds of narration: the first cue, at 0.2 s, comes after a
        // wait of half a second, late; the second, at 1.0 s, is waited for.
        let p = play(
            "late",
            r#"{"fps": 30, "steps": [{"seconds": 2.0, "actions": [
                {"wait": 0.5}, {"cue": 0.2}, {"cue": 1.0}]}]}"#,
        );
        let log = &p.log[0];
        let [first, second] = log.cues[..] else {
            panic!("{:?}", log.cues);
        };
        assert!(first > 6, "the first cue is late: {first}");
        // The words between the cues keep their time: the second cue is as
        // late as the first.
        assert_eq!(second - first, 30 - 6);
        // The step as long as its narration and the time its words waited.
        assert_eq!(log.frames, 60 + (first - 6));
    }

    #[test]
    fn a_step_on_time_lasts_its_narration() {
        let p = play(
            "on-time",
            r#"{"fps": 30, "steps": [{"seconds": 2.0, "actions": [{"cue": 1.0}]}]}"#,
        );
        assert_eq!(p.log[0].cues, [30]);
        assert_eq!(p.log[0].frames, 60);
    }

    #[test]
    fn a_picture_by_an_action_is_taken_once() {
        let p = play(
            "shot",
            r#"{"fps": 30, "steps": [{"seconds": 0.5, "shot": "a.png",
                "actions": [{"shot": true}, {"wait": 0.2}]}]}"#,
        );
        assert!(p.shot_taken);
        // Asked for once: the step's end takes none again.
        assert_eq!(
            p.frames.lock().unwrap().pending,
            p.log[0].frames as usize + 1
        );
    }

    #[test]
    fn a_card_comes_in_part_by_part() {
        assert!(coming(0.0, CARD_TITLE).abs() < 1e-6);
        assert!((coming(CARD_TITLE + CARD_IN, CARD_TITLE) - 1.0).abs() < 1e-6);
        // Fast, then slow.
        let half = coming(CARD_TITLE + CARD_IN / 2.0, CARD_TITLE);
        assert!(half > 0.8 && half < 1.0, "{half}");
    }
}
