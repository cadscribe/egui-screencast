//! The sections asked to be spoken, one at a time on a thread of their own
//! (ADR 0056): the list asked last replaces the one before, the step shown
//! first; what is spoken goes into the app's cache (a remote section into
//! the lasting cache, where one is given) and is said back. Dropped, the
//! queue kills its worker.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use super::{Section, Speech, cache};

/// What speaks a section into a WAV: the worker process, or a test's.
pub trait Synth: Send {
    /// `section` spoken into `out`: a 16-bit WAV, or AAC in an MP4 (told
    /// by its bytes, whatever `out` is called).
    ///
    /// # Errors
    ///
    /// The voice failed or is gone.
    fn speak(&mut self, section: &Section, out: &Path) -> Result<(), String>;
    /// Its process ended.
    fn close(&mut self);
}

/// Kills a synth's process from another thread, while it speaks.
pub type Kill = Arc<dyn Fn() + Send + Sync>;

/// A section spoken, or why not.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub key: String,
    pub result: Result<PathBuf, String>,
}

#[derive(Debug, Default)]
struct State {
    pending: VecDeque<Section>,
    busy: Option<String>,
    quit: bool,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

/// The queue and its thread.
pub struct Queue {
    shared: Arc<Shared>,
    done: Receiver<Done>,
    kill: Kill,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue")
            .field("pending", &self.pending())
            .field("busy", &self.busy())
            .finish()
    }
}

impl Queue {
    /// A queue of the app `app` speaking by `synth` into `root`.
    pub fn new(app: &str, root: PathBuf, synth: Box<dyn Synth>, kill: Kill) -> Queue {
        Queue::keeping(app, root, None, synth, kill)
    }

    /// A queue as [`Queue::new`] that keeps the remote sections
    /// ([`Speech::Remote`]) in `lasting`, where one is given.
    pub fn keeping(
        app: &str,
        root: PathBuf,
        lasting: Option<PathBuf>,
        synth: Box<dyn Synth>,
        kill: Kill,
    ) -> Queue {
        let shared = Arc::new(Shared::default());
        let (tx, done) = channel();
        let s = Arc::clone(&shared);
        let by = format!("{app}'s lessons");
        let spawned = std::thread::Builder::new()
            .name(format!("{}-voice", app.to_lowercase()))
            .spawn(move || run(&s, &root, lasting.as_deref(), &by, synth, &tx));
        if let Err(e) = spawned {
            eprintln!("{}: voice: {e}", app.to_lowercase());
        }
        Queue { shared, done, kill }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// These sections, in this order, instead of those asked before; the
    /// one being spoken is not asked again.
    pub fn want(&self, sections: Vec<Section>) {
        let mut st = self.state();
        let mut seen: Vec<String> = st.busy.iter().cloned().collect();
        st.pending.clear();
        for s in sections {
            if !seen.contains(&s.key) {
                seen.push(s.key.clone());
                st.pending.push_back(s);
            }
        }
        drop(st);
        self.shared.wake.notify_all();
    }

    /// The keys waiting, in order.
    pub fn pending(&self) -> Vec<String> {
        self.state().pending.iter().map(|s| s.key.clone()).collect()
    }

    /// The key being spoken.
    pub fn busy(&self) -> Option<String> {
        self.state().busy.clone()
    }

    /// What was spoken since asked last.
    pub fn poll(&self) -> Vec<Done> {
        self.done.try_iter().collect()
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        {
            let mut st = self.state();
            st.quit = true;
            st.pending.clear();
        }
        self.shared.wake.notify_all();
        // A section being spoken is not waited for: its process is ended.
        (self.kill)();
    }
}

fn run(
    shared: &Shared,
    root: &Path,
    lasting: Option<&Path>,
    by: &str,
    mut synth: Box<dyn Synth>,
    done: &Sender<Done>,
) {
    loop {
        let section = {
            let mut st = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if st.quit {
                    drop(st);
                    synth.close();
                    return;
                }
                if let Some(s) = st.pending.pop_front() {
                    st.busy = Some(s.key.clone());
                    break s;
                }
                st = shared
                    .wake
                    .wait(st)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let into = match (&section.speech, lasting) {
            (Speech::Remote { .. }, Some(l)) => l,
            _ => root,
        };
        let result = speak(into, by, synth.as_mut(), &section);
        let quit = {
            let mut st = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            st.busy = None;
            st.quit
        };
        if quit {
            synth.close();
            return;
        }
        let _ = done.send(Done {
            key: section.key.clone(),
            result,
        });
    }
}

/// One section spoken into the cache `root`.
fn speak(root: &Path, by: &str, synth: &mut dyn Synth, s: &Section) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let out = root.join(format!(".{}.{}.wav", s.key, std::process::id()));
    let spoken = synth.speak(s, &out);
    if let Err(e) = spoken {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    let mut meta = json!({
        "text": s.text,
        "backend": s.engine.backend(),
        "voice": s.voice,
        "language": s.lang.key(),
        "by": by,
        "created": today(),
    });
    if let Speech::Remote { backend, model, .. } = &s.speech {
        meta["backend"] = backend.as_str().into();
        meta["model"] = model.as_str().into();
    }
    cache::put(root, &s.key, &out, &meta)
}

/// Today as `YYYY-MM-DD` (UTC).
pub fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (y, m, d) = civil((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The day `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}
