//! The voice worker: `worker.py` in mlx-speech's Python (S2 Pro) or the
//! tutorial venv's (Kokoro), started on the first section to speak and
//! kept for the next (ADR 0056). One process at a time: S2 Pro holds about
//! 10 GB. Offline, at a low priority, its log beside the cache. The app's
//! remote sections go to its own synth instead ([`Dispatch`]).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::discover::Found;
use super::queue::{Kill, Synth};
use super::{Engine, Section, Speech};
use opencad_ui::i18n::trf;

/// The worker, built in.
const WORKER: &str = include_str!("worker.py");

/// The process and its pipes.
struct Running {
    engine: Engine,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// The real synth.
pub struct Process {
    /// The app's name: the worker's file name.
    app: &'static str,
    found: Found,
    /// The app's cache: the worker's script and log go there.
    root: PathBuf,
    child: Arc<Mutex<Option<Child>>>,
    running: Option<Running>,
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process")
            .field("found", &self.found)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Process {
    /// A synth of the app `app` of what `found` has, and what kills its
    /// process.
    pub fn make(app: &'static str, found: Found, root: PathBuf) -> (Box<dyn Synth>, Kill) {
        let child: Arc<Mutex<Option<Child>>> = Arc::default();
        let c = Arc::clone(&child);
        let kill: Kill = Arc::new(move || {
            if let Some(mut ch) = c.lock().ok().and_then(|mut g| g.take()) {
                let _ = ch.kill();
                let _ = ch.wait();
            }
        });
        let p = Process {
            app,
            found,
            root,
            child,
            running: None,
        };
        (Box::new(p), kill)
    }

    fn start(&mut self, engine: Engine) -> Result<(), String> {
        self.close();
        let (python, name) = match engine {
            Engine::Fish => (self.found.fish.as_ref().map(|f| f.python.clone()), "fish"),
            Engine::Kokoro => (
                self.found.kokoro.as_ref().map(|k| k.python.clone()),
                "kokoro",
            ),
            // Only played from the recordings: the app never asks it; a
            // remote voice is the app's.
            Engine::ElevenLabs => (None, "elevenlabs"),
            Engine::Remote => (None, "remote"),
        };
        let python = python.ok_or_else(|| trf(Msg::NotFound, &[&engine.label()]))?;
        std::fs::create_dir_all(&self.root).map_err(|e| format!("{}: {e}", self.root.display()))?;
        let script = (self.root).join(format!("{}-voice-worker.py", self.app.to_lowercase()));
        if std::fs::read_to_string(&script).ok().as_deref() != Some(WORKER) {
            std::fs::write(&script, WORKER).map_err(|e| format!("{}: {e}", script.display()))?;
        }
        let log = std::fs::File::create(self.root.join("worker.log")).ok();
        // At a low priority, where `nice` is.
        let nice = Path::new("/usr/bin/nice");
        let mut cmd = if nice.exists() {
            let mut c = Command::new(nice);
            c.args(["-n", "10"]).arg(&python);
            c
        } else {
            Command::new(&python)
        };
        cmd.arg("-I").arg(&script).arg(name);
        cmd.env("HF_HUB_OFFLINE", "1");
        if let (Engine::Kokoro, Some(k)) = (engine, &self.found.kokoro) {
            cmd.env("HF_HOME", &k.hf_home);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        cmd.stderr(log.map_or_else(Stdio::null, Stdio::from));
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("{}: {e}", python.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        if let Ok(mut g) = self.child.lock() {
            *g = Some(child);
        }
        self.running = Some(Running {
            engine,
            stdin,
            stdout,
        });
        Ok(())
    }
}

impl Synth for Process {
    fn speak(&mut self, s: &Section, out: &Path) -> Result<(), String> {
        if matches!(s.speech, Speech::Recorded | Speech::Remote { .. }) {
            return Err(trf(Msg::NotFound, &[&s.engine.label()]));
        }
        let alive = self.child.lock().is_ok_and(|g| g.is_some());
        if !alive || self.running.as_ref().is_none_or(|r| r.engine != s.engine) {
            self.start(s.engine)?;
        }
        let out_str = out.display().to_string();
        let request = match &s.speech {
            Speech::Fish {
                reference,
                transcript,
                language,
                seed,
            } => json!({
                "text": s.text,
                "out": out_str,
                "reference_audio": reference.display().to_string(),
                "reference_text": transcript,
                "language": language,
                "seed": seed,
            }),
            Speech::Kokoro { voice, speed } => json!({
                "text": s.text,
                "out": out_str,
                "voice": voice,
                "speed": speed,
            }),
            Speech::Recorded | Speech::Remote { .. } => {
                return Err(trf(Msg::NotFound, &[&s.engine.label()]));
            }
        };
        let r = self.running.as_mut().ok_or("no worker")?;
        let sent = writeln!(r.stdin, "{request}").and_then(|()| r.stdin.flush());
        let mut line = String::new();
        let read = sent.and_then(|()| r.stdout.read_line(&mut line));
        if read.is_err() || line.is_empty() {
            self.close();
            return Err(trf(Msg::Ended, &[&s.engine.label()]));
        }
        let answer: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if answer["ok"].as_bool() == Some(true) && out.is_file() {
            Ok(())
        } else {
            let e = answer["error"].as_str().unwrap_or("no audio");
            Err(format!("{}: {e}", s.engine.label()))
        }
    }

    fn close(&mut self) {
        // Its stdin closed, the worker ends; it is not waited for.
        self.running = None;
        if let Some(mut ch) = self.child.lock().ok().and_then(|mut g| g.take()) {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

/// The worker of a speaker: the remote sections ([`Speech::Remote`]) by
/// the app's synth ([`super::Narrate::remote`]), the others by the local
/// one.
pub struct Dispatch {
    pub local: Box<dyn Synth>,
    pub remote: Option<Box<dyn Synth>>,
}

impl std::fmt::Debug for Dispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatch")
            .field("remote", &self.remote.is_some())
            .finish_non_exhaustive()
    }
}

impl Dispatch {
    /// The worker of `local` and `remote` (each with what stops it), and
    /// what stops both.
    pub fn make(
        local: (Box<dyn Synth>, Kill),
        remote: Option<(Box<dyn Synth>, Kill)>,
    ) -> (Box<dyn Synth>, Kill) {
        let (local, local_kill) = local;
        let (remote, remote_kill) = remote.unzip();
        let kill: Kill = Arc::new(move || {
            if let Some(k) = &remote_kill {
                k();
            }
            local_kill();
        });
        (Box::new(Dispatch { local, remote }), kill)
    }
}

impl Synth for Dispatch {
    fn speak(&mut self, s: &Section, out: &Path) -> Result<(), String> {
        match (&s.speech, &mut self.remote) {
            (Speech::Remote { .. }, Some(r)) => r.speak(s, out),
            (Speech::Remote { label, .. }, None) => Err(trf(Msg::NotFound, &[label])),
            _ => self.local.speak(s, out),
        }
    }

    fn close(&mut self) {
        self.local.close();
        if let Some(r) = &mut self.remote {
            r.close();
        }
    }
}

opencad_ui::messages! {
    enum Msg {
        Ended: "{0} stopped, see worker.log in the voice cache", "{0} hat aufgehört, mehr steht in worker.log im Stimmen-Cache";
        NotFound: "{0} is not on this computer", "{0} ist nicht auf diesem Computer";
    }
}
