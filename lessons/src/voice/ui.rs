//! The voice's row in the lesson pane: the switch, what it does, Stop or
//! Replay, and the notice that it is an AI voice; while it plays, the bar
//! of how far it is and the speaker's level; why it fails; and its rows in
//! the settings. Each control drawn is put into `spots` by name ("Read
//! aloud", "Voice info", "Stop voice", …), where an app's tests and
//! recordings find it.

use std::path::{Path, PathBuf};

use opencad_ui::widgets::button::IconButton;
use opencad_ui::widgets::feedback::spinner;
use opencad_ui::widgets::field::{FieldLook, TextField};
use opencad_ui::widgets::segmented::{Segment, Segmented};
use opencad_ui::widgets::toggle;
use opencad_ui::{Icon, Type, colors, space, text};

use super::meter::{self, BARS};
use super::{ElevenField, Prefer, Speaker, Status};
use crate::Lang;
use opencad_ui::i18n::{tr, trf};

/// Where the controls drawn are, by name.
pub type Spots = Vec<(String, egui::Rect)>;

/// The voice in the lesson pane, beside the step's number: the switch
/// (its name in its tooltip), what the voice is as an info icon, Stop
/// or Replay as icons. `dir` holds the settings (`voice.json`).
pub fn row(ui: &mut egui::Ui, v: &mut Speaker, dir: &Path, spots: &mut Spots) {
    {
        let c = colors(ui);
        let lang = crate::lang();
        v.load(dir);
        // What the voice is, or why there is none: an icon's tooltip, so
        // that the row is one line (ADR 0058).
        // Its spot as wide as an icon button's, so that the icons are
        // evenly apart; the icon centred in it.
        let about = |ui: &mut egui::Ui, line: &str| {
            let side = opencad_ui::size::BUTTON_MD;
            let (r, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            let ir = egui::Rect::from_center_size(r.center(), egui::vec2(16.0, 16.0));
            opencad_ui::icon::paint(ui.painter(), ir, Icon::Info, c.text_tertiary);
            resp.on_hover_text(line)
        };
        // In the row's own layout (`ui.scope`), not `ui.horizontal`: the
        // row's height is kept, each item centred on it.
        if !v.available(lang) {
            ui.scope(|ui| {
                let mut off = false;
                let r = about(ui, tr(Msg::NoVoice));
                spots.push(("Voice info".into(), r.rect));
                // The switch as far from the icon as the icons are apart.
                ui.add_space(space::SM);
                let r = ui.add_enabled_ui(false, |ui| toggle::switch(ui, &mut off, ""));
                let r = r.inner.on_disabled_hover_text(tr(Msg::ReadAloud));
                spots.push(("Read aloud".into(), r.rect));
            });
            return;
        }
        let notice = v.info();
        let mut on = v.on(lang);
        let mut asked = None;
        ui.scope(|ui| {
            let r = toggle::switch(ui, &mut on, "").on_hover_text(tr(Msg::ReadAloud));
            spots.push(("Read aloud".into(), r.rect));
            if r.changed() {
                asked = Some(if on { Ask::On } else { Ask::Off });
            }
            if !on {
                return;
            }
            // The icon as far from the switch as the icons are apart.
            ui.add_space(space::SM);
            let r = about(ui, &notice);
            spots.push(("Voice info".into(), r.rect));
            if v.status == Status::Preparing {
                spinner(ui, 12.0).on_hover_text(tr(Msg::Preparing));
            } else if matches!(v.status, Status::Loading { .. }) {
                let r = spinner(ui, 12.0).on_hover_text(&notice);
                spots.push(("Voice loading".into(), r.rect));
            } else if v.playing() {
                let r = ui.add(IconButton::new(Icon::Close, tr(Msg::Stop)));
                let r = r.on_hover_text(tr(Msg::Stop));
                spots.push(("Stop voice".into(), r.rect));
                if r.clicked() {
                    asked = Some(Ask::Stop);
                }
                // Pause and on again where Replay is once it has ended.
                let paused = v.paused();
                let tip = tr(if paused { Msg::Resume } else { Msg::Pause });
                let r = pause_button(ui, paused).on_hover_text(tip);
                spots.push(("Pause voice".into(), r.rect));
                if r.clicked() {
                    asked = Some(Ask::Pause(!paused));
                }
            } else if v.current.is_some() {
                let r = ui.add(IconButton::new(Icon::Refresh, tr(Msg::Replay)));
                let r = r.on_hover_text(tr(Msg::Replay));
                spots.push(("Replay voice".into(), r.rect));
                if r.clicked() {
                    asked = Some(Ask::Replay);
                }
            }
            if v.current.is_some() {
                let still = !super::animates(ui.ctx());
                let now = ui.input(|i| i.time);
                let playing = v.playing() && !v.paused();
                let bars = if playing && still {
                    STILL
                } else {
                    v.bars(now, still)
                };
                let r = speaker(ui, bars, playing);
                spots.push(("Voice level".into(), r.rect));
            }
        });
        match asked {
            Some(Ask::On) => v.switch(true),
            Some(Ask::Off) => v.switch(false),
            Some(Ask::Stop) => v.stop(),
            Some(Ask::Pause(p)) => v.pause(p),
            Some(Ask::Replay) => v.replay(),
            None => {}
        }
    }
}

/// While the step's audio plays: a thin bar under the step's row, as
/// far as it has played, `width` wide; a click or a drag in it jumps
/// there.
pub fn progress(ui: &mut egui::Ui, v: &mut Speaker, width: f32, spots: &mut Spots) {
    {
        let lang = crate::lang();
        if !v.available(lang) || !v.on(lang) {
            return;
        }
        let Some((at, total)) = v.progress() else {
            return;
        };
        let c = *colors(ui);
        // The time at the bar's end, not in the step's row, where it ran
        // into "Step 1 of 19".
        let line = format!("{} / {}", meter::clock_text(at), meter::clock_text(total));
        let galley = text::galley(ui.painter(), &line, Type::Caption, c.text_secondary);
        let time_w = galley.size().x + space::SM;
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(width, galley.size().y.max(10.0)),
            egui::Sense::hover(),
        );
        let time_rect =
            egui::Rect::from_min_max(egui::pos2(row.right() - time_w, row.top()), row.max);
        let r = ui.put(
            time_rect,
            egui::Label::new(text::rich(&line, Type::Caption, c.text_secondary).monospace()),
        );
        spots.push(("Voice time".into(), r.rect));
        let rect =
            egui::Rect::from_min_max(row.min, egui::pos2(row.right() - time_w, row.bottom()));
        let r = ui.interact(
            rect,
            ui.scope_id().with("voice progress"),
            egui::Sense::click_and_drag(),
        );
        let r = r.on_hover_text(tr(Msg::Seek));
        spots.push(("Voice progress".into(), rect));
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        if r.is_pointer_button_down_on()
            && (pressed || r.drag_delta().x != 0.0)
            && let Some(pos) = r.interact_pointer_pos()
        {
            let k = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
            v.seek(total.mul_f32(k));
        }
        // Where it was sought to, at once.
        let (at, total) = v.progress().unwrap_or((at, total));
        let k = if total.is_zero() {
            0.0
        } else {
            (at.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
        };
        let track = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 3.0));
        let p = ui.painter();
        p.rect_filled(track, 1.5, c.track);
        let mut done = track;
        done.set_width(track.width() * k);
        if k > 0.0 {
            p.rect_filled(done, 1.5, c.accent);
        }
        if r.hovered() || r.dragged() {
            p.circle_filled(egui::pos2(done.right(), track.center().y), 4.5, c.accent);
        }
    }
}

/// The bars of a speaker that plays without animations: still, but not
/// at rest, so that it is seen to play.
const STILL: [f32; BARS] = [0.35, 0.7, 0.5, 0.25];

/// The speaker: its cone and [`BARS`] bars as high as `bars` (0 to 1), in
/// the accent while `playing`.
fn speaker(ui: &mut egui::Ui, bars: [f32; BARS], playing: bool) -> egui::Response {
    let c = colors(ui);
    let side = opencad_ui::size::BUTTON_MD;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, side), egui::Sense::hover());
    let colour = if playing { c.accent } else { c.text_tertiary };
    let p = ui.painter();
    let mid = rect.center().y;
    let left = rect.left() + 1.0;
    // The cone: a small box and its flare.
    p.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(left, mid - 2.5),
            egui::pos2(left + 3.0, mid - 2.5),
            egui::pos2(left + 7.0, mid - 6.0),
            egui::pos2(left + 7.0, mid + 6.0),
            egui::pos2(left + 3.0, mid + 2.5),
            egui::pos2(left, mid + 2.5),
        ],
        colour,
        egui::Stroke::NONE,
    ));
    let (w, gap, low, high) = (2.0, 1.5, 2.0, 14.0);
    for (i, b) in bars.iter().enumerate() {
        let x = left + 9.5 + i as f32 * (w + gap);
        let h = low + (high - low) * b.clamp(0.0, 1.0);
        let bar = egui::Rect::from_center_size(egui::pos2(x + w / 2.0, mid), egui::vec2(w, h));
        p.rect_filled(bar, 1.0, colour);
    }
    r
}

/// Pause, or on again (the icon set has neither): two bars, or a play
/// triangle, in a button like the icon buttons beside it.
fn pause_button(ui: &mut egui::Ui, paused: bool) -> egui::Response {
    let c = *colors(ui);
    let side = opencad_ui::size::BUTTON_MD;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let s = opencad_ui::state::state(ui, &r);
    let hover = opencad_ui::state::hover_fill(ui, r.id, s, &c);
    let p = ui.painter();
    if hover.a() > 0 {
        p.rect_filled(rect, opencad_ui::radius::FIELD, hover);
    }
    let m = rect.center();
    if paused {
        p.add(egui::Shape::convex_polygon(
            vec![
                m + egui::vec2(-3.5, -5.0),
                m + egui::vec2(5.0, 0.0),
                m + egui::vec2(-3.5, 5.0),
            ],
            c.icon,
            egui::Stroke::NONE,
        ));
    } else {
        for dx in [-2.75, 2.75] {
            let bar = egui::Rect::from_center_size(m + egui::vec2(dx, 0.0), egui::vec2(2.5, 10.0));
            p.rect_filled(bar, 1.0, c.icon);
        }
    }
    if s.focused {
        opencad_ui::state::focus_ring(ui, rect, opencad_ui::radius::FIELD, &c);
    }
    r.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            tr(if paused { Msg::Resume } else { Msg::Pause }),
        )
    });
    r
}

/// Why reading aloud does not work, under the step's number, while it
/// is on: the worker failed, or the sound did not play. Plain, not
/// only in a tooltip, so that the voice is never silent without a
/// word.
pub fn failure(ui: &mut egui::Ui, v: &mut Speaker, spots: &mut Spots) {
    {
        let lang = crate::lang();
        if !v.available(lang) || !v.on(lang) {
            return;
        }
        let why = match &v.status {
            Status::Failed(e) => Some(e.clone()),
            _ => v.play_error(),
        };
        let Some(why) = why else {
            return;
        };
        let c = colors(ui);
        let line = trf(super::Msg::Failed, &[&why]);
        let r = ui.add(egui::Label::new(text::rich(line, Type::Caption, c.danger.text)).wrap());
        spots.push(("Voice failed".into(), r.rect));
    }
}

#[derive(Debug, Clone, Copy)]
enum Ask {
    On,
    Off,
    Stop,
    Pause(bool),
    Replay,
}

/// Lays out a labelled row of the settings.
pub type Row = dyn Fn(&mut egui::Ui, &str, &mut dyn FnMut(&mut egui::Ui));

/// The voice's rows in the settings: the voices folder, what was found,
/// and which voice where both speak. `row` lays out a labelled row.
pub fn settings_rows(ui: &mut egui::Ui, v: &mut Speaker, dir: &Path, row: &Row) {
    v.load(dir);
    let c = colors(ui);
    let shown = v
        .settings
        .voices
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut typed = ui
        .data(|d| d.get_temp::<String>(egui::Id::unique("voice-folder")))
        .unwrap_or_else(|| shown.clone());
    let mut commit = false;
    row(ui, tr(Msg::VoicesFolder), &mut |ui| {
        let r = TextField::new(&mut typed)
            .id_salt("settings-voices")
            .hint("~/dev/tts-voices")
            .look(FieldLook::Bordered)
            .width(200.0)
            .show(ui);
        commit = r.lost_focus();
    });
    if commit {
        let t = typed.trim();
        let path = if t.is_empty() {
            None
        } else if let Some(rest) = t.strip_prefix("~/") {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest))
        } else {
            Some(PathBuf::from(t))
        };
        v.set_voices(path);
        ui.data_mut(|d| d.remove::<String>(egui::Id::unique("voice-folder")));
    } else {
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("voice-folder"), typed));
    }
    // The recordings' ElevenLabs per language: the voice id (empty for
    // none), the model and the language they told it (empty: the script's).
    let mut eleven = Vec::new();
    row(ui, tr(Msg::Eleven), &mut |ui| {
        ui.vertical(|ui| {
            for lang in [Lang::De, Lang::En] {
                ui.horizontal(|ui| {
                    ui.label(text::rich(
                        lang.key().to_uppercase(),
                        Type::Caption,
                        c.text_tertiary,
                    ));
                    let fields = [
                        (
                            ElevenField::Voice,
                            v.settings.eleven(lang),
                            Msg::ElevenNone,
                            170.0,
                        ),
                        (
                            ElevenField::Model,
                            v.settings.eleven_model(lang),
                            Msg::ElevenModel,
                            110.0,
                        ),
                        (
                            ElevenField::Language,
                            v.settings.eleven_language(lang),
                            Msg::ElevenLanguage,
                            56.0,
                        ),
                    ];
                    for (field, shown, hint, width) in fields {
                        let shown = shown.unwrap_or_default().to_owned();
                        let id = egui::Id::unique(("voice-eleven", lang.key(), field as u8));
                        let mut typed = ui
                            .data(|d| d.get_temp::<String>(id))
                            .unwrap_or_else(|| shown.clone());
                        let r = TextField::new(&mut typed)
                            .id_salt(("settings-eleven", lang.key(), field as u8))
                            .hint(tr(hint))
                            .look(FieldLook::Bordered)
                            .width(width)
                            .show(ui);
                        if r.lost_focus() {
                            ui.data_mut(|d| d.remove::<String>(id));
                            if typed.trim() != shown {
                                eleven.push((lang, field, typed));
                            }
                        } else {
                            ui.data_mut(|d| d.insert_temp(id, typed));
                        }
                    }
                });
            }
        });
    });
    for (lang, field, value) in eleven {
        v.set_eleven(lang, field, &value);
    }
    let found = v.found().clone();
    let mut lines = Vec::new();
    if let Some(f) = &found.fish {
        let names: Vec<&str> = f.voices.iter().map(|x| x.name.as_str()).collect();
        lines.push(format!("Fish Audio S2 Pro: {}", names.join(", ")));
    }
    if found.kokoro.is_some() {
        lines.push(tr(Msg::KokoroFound).to_owned());
    }
    // Where the recordings' sections are looked for, in turn.
    let folders: Vec<String> = found
        .recorded
        .iter()
        .filter(|d| d.is_dir())
        .map(|d| d.display().to_string())
        .collect();
    let folders = if folders.is_empty() {
        super::cache::lasting_dir(v.config().app).map(|d| d.display().to_string())
    } else {
        Some(folders.join(", "))
    };
    if let Some(folders) = folders {
        lines.push(trf(Msg::ElevenFrom, &[&folders]));
    }
    if !found.any() {
        lines.extend(found.missing.iter().cloned());
    }
    let mut prefer = v.settings.prefer;
    row(ui, "", &mut |ui| {
        ui.vertical(|ui| {
            for l in &lines {
                ui.add(egui::Label::new(text::rich(l, Type::Caption, c.text_tertiary)).wrap());
            }
            if found.fish.is_some() && found.kokoro.is_some() {
                let segments = [
                    Segment::text(Prefer::Fish, "Fish Audio S2 Pro"),
                    Segment::text(Prefer::Kokoro, "Kokoro"),
                ];
                Segmented::new(&mut prefer, &segments).compact().show(ui);
            }
        });
    });
    v.set_prefer(prefer);
}

opencad_ui::messages! {
    enum Msg {
        ReadAloud: "Read aloud", "Vorlesen";
        NoVoice: "No voice on this computer. Reading aloud needs mlx-speech with Fish Audio S2 Pro and a voices folder (Settings › Tutorial voices), or Kokoro from make tutorial-venv. Nothing is downloaded, and no text leaves your computer.", "Keine Stimme auf diesem Computer. Zum Vorlesen brauchst du mlx-speech mit Fish Audio S2 Pro und einen Stimmen-Ordner (Einstellungen › Tutorial-Stimmen) oder Kokoro aus make tutorial-venv. Nichts wird heruntergeladen, und kein Text verlässt deinen Computer.";
        Preparing: "Preparing the voice …", "Die Stimme wird vorbereitet …";
        Stop: "Stop", "Stopp";
        Replay: "Replay", "Nochmal";
        Pause: "Pause", "Pause";
        Resume: "Play on", "Weiter abspielen";
        Seek: "Click or drag to jump to a spot", "Klicke oder ziehe, um an eine Stelle zu springen";
        Eleven: "ElevenLabs voice (from the recordings)", "ElevenLabs-Stimme (aus den Aufnahmen)";
        ElevenNone: "none", "keine";
        ElevenModel: "script's model", "Modell des Skripts";
        ElevenLanguage: "language", "Sprache";
        ElevenFrom: "ElevenLabs: only what the recordings spoke, played from {0}. The app never asks ElevenLabs.", "ElevenLabs: nur was die Aufnahmen gesprochen haben, abgespielt aus {0}. Die App fragt ElevenLabs nie an.";
        VoicesFolder: "Tutorial voices", "Tutorial-Stimmen";
        KokoroFound: "Kokoro (English, from the tutorial venv)", "Kokoro (Englisch, aus dem Tutorial-venv)";
    }
}
