//! What can speak on this machine (ADR 0056): Fish Audio S2 Pro by the
//! user's own mlx-speech with a voices folder, Kokoro in the tutorial venv
//! of a development setup. Only looked for: nothing is installed or
//! downloaded, and a model not here is no voice.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::key::{FISH_MODEL, KOKORO_REPO, generation_language, sha256};
use crate::{Config, Lang};

/// The voices of S2 Pro by language, as the runner has them
/// (`tutorial.FISH_VOICES`).
const FISH_VOICES: [(Lang, &str); 2] = [(Lang::En, "en-gb-female"), (Lang::De, "de-male")];

/// Where things are looked for: the real machine's, or a test's.
#[derive(Debug, Clone, Default)]
pub struct Places {
    /// The home folder.
    pub home: Option<PathBuf>,
    /// The `mlx-speech` command, should it be on the PATH.
    pub mlx_speech: Option<PathBuf>,
    /// `$FISH_SPEECH_PYTHON`.
    pub fish_python: Option<PathBuf>,
    /// The Hugging Face cache of the user (`$HF_HUB_CACHE`, `$HF_HOME/hub`,
    /// `~/.cache/huggingface/hub`).
    pub hf_hub: Option<PathBuf>,
    /// The voices folder with its `voices.yaml`.
    pub voices: Option<PathBuf>,
    /// The tutorials' work folder (`target/tutorials`): the venv, Kokoro's
    /// cache and the recordings' sections.
    pub tutorials: Option<PathBuf>,
    /// The recordings' caches of sections looked in before the tutorials'
    /// work folder's, in turn (`cache::recorded_dirs`); a downloaded
    /// bundle would be one more.
    pub recorded: Vec<PathBuf>,
}

/// The voices folder when none is set.
pub fn default_voices(home: Option<&Path>) -> Option<PathBuf> {
    std::env::var_os("TUTORIAL_VOICES")
        .or_else(|| std::env::var_os("SCREENCAST_VOICES"))
        .map(PathBuf::from)
        .or_else(|| Some(home?.join("dev/tts-voices")))
}

impl Places {
    /// This machine's for the app of `config`, the voices folder `voices`
    /// (none: the default).
    pub fn here(config: &Config, voices: Option<PathBuf>) -> Places {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = env("HOME").or_else(|| env("USERPROFILE"));
        let mlx_speech = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|d| d.join("mlx-speech"))
                .find(|p| p.is_file())
        });
        let hf_hub = env("HF_HUB_CACHE")
            .or_else(|| env("HF_HOME").map(|h| h.join("hub")))
            .or_else(|| Some(home.as_ref()?.join(".cache/huggingface/hub")));
        let tutorials = (config.voice.work_env).and_then(env).or_else(|| {
            let built = PathBuf::from(config.voice.work_dir?);
            built.is_dir().then_some(built)
        });
        Places {
            voices: voices.or_else(|| default_voices(home.as_deref())),
            home,
            mlx_speech,
            fish_python: env("FISH_SPEECH_PYTHON"),
            hf_hub,
            tutorials,
            recorded: super::cache::recorded_dirs(config.app),
        }
    }
}

/// A voice of S2 Pro: its clip, what is said in it, its language.
#[derive(Debug, Clone, PartialEq)]
pub struct FishVoice {
    pub lang: Lang,
    pub name: String,
    pub reference: PathBuf,
    pub transcript: String,
    /// What S2 Pro is told (`--language`): none for English.
    pub language: Option<String>,
    /// SHA-256 of the clip.
    pub hash: String,
}

/// S2 Pro by mlx-speech.
#[derive(Debug, Clone, PartialEq)]
pub struct Fish {
    /// mlx-speech's own Python.
    pub python: PathBuf,
    pub voices: Vec<FishVoice>,
}

/// Kokoro in the tutorial venv.
#[derive(Debug, Clone, PartialEq)]
pub struct Kokoro {
    pub python: PathBuf,
    /// Its Hugging Face home (`target/tutorials/hf`).
    pub hf_home: PathBuf,
}

/// What was found, and why the rest was not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub fish: Option<Fish>,
    pub kokoro: Option<Kokoro>,
    /// The tutorials' work folder: Kokoro's recorded sections.
    pub tutorials: Option<PathBuf>,
    /// The recordings' caches of S2 Pro's and ElevenLabs' sections, looked
    /// in in turn: the lasting one, `$SCREENCAST_VOICE_CACHE`, the
    /// tutorials' work folder's `voices`.
    pub recorded: Vec<PathBuf>,
    /// Why a voice is missing, a line each.
    pub missing: Vec<String>,
}

impl Found {
    /// Whether anything speaks `lang`.
    pub fn speaks(&self, lang: Lang) -> bool {
        self.fish
            .as_ref()
            .is_some_and(|f| f.voices.iter().any(|v| v.lang == lang))
            || (lang == Lang::En && self.kokoro.is_some())
    }

    pub fn any(&self) -> bool {
        self.fish.is_some() || self.kokoro.is_some()
    }
}

/// The Python of the `mlx-speech` command: its first line.
fn shebang(command: &Path) -> Option<PathBuf> {
    let text = std::fs::read(command).ok()?;
    let first = text.split(|b| *b == b'\n').next()?;
    let first = std::str::from_utf8(first).ok()?;
    let python = first.strip_prefix("#!")?.trim();
    // `#!/usr/bin/env python` is no Python of its own.
    (!python.contains(' ') && !python.is_empty()).then(|| PathBuf::from(python))
}

/// A model in a Hugging Face hub cache (`models--org--name/snapshots/…`).
fn in_hub(hub: &Path, repo: &str) -> bool {
    let dir = hub.join(format!("models--{}", repo.replace('/', "--")));
    std::fs::read_dir(dir.join("snapshots")).is_ok_and(|mut d| d.next().is_some())
}

/// The voices of `voices.yaml` in `dir`: the runner's for each language,
/// else the first of it.
fn fish_voices(dir: &Path) -> Result<Vec<FishVoice>, String> {
    let index = dir.join("voices.yaml");
    let text = std::fs::read_to_string(&index)
        .map_err(|_| format!("no voices.yaml in {}", dir.display()))?;
    let v: Value = serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", index.display()))?;
    let folder = dir.join(v.get("dir").and_then(Value::as_str).unwrap_or("."));
    let listed = v
        .get("voices")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let language_of = |e: &Value| e.get("language").and_then(Value::as_str).map(str::to_owned);
    let of_lang = |e: &Value, lang: Lang| {
        let l = language_of(e).unwrap_or_default().to_lowercase();
        match lang {
            Lang::De => l == "german" || l == "deutsch",
            _ => l == "english",
        }
    };
    let mut out = Vec::new();
    for (lang, preferred) in FISH_VOICES {
        let chosen = listed
            .get(preferred)
            .map(|e| (preferred.to_owned(), e))
            .or_else(|| {
                listed
                    .iter()
                    .find(|(_, e)| of_lang(e, lang))
                    .map(|(n, e)| (n.clone(), e))
            });
        let Some((name, e)) = chosen else {
            continue;
        };
        let Some(reference) = e.get("reference").and_then(Value::as_str) else {
            continue;
        };
        let reference = folder.join(reference);
        let transcript = e.get("transcript").and_then(Value::as_str).unwrap_or("");
        let Ok(bytes) = std::fs::read(&reference) else {
            continue;
        };
        if transcript.trim().is_empty() {
            continue;
        }
        // As the runner: the run's language for German, the voice's own
        // for English (told nothing).
        let language = match lang {
            Lang::De => Some("German".to_owned()),
            _ => generation_language(language_of(e).as_deref()),
        };
        out.push(FishVoice {
            lang,
            name,
            reference,
            transcript: transcript.to_owned(),
            language,
            hash: sha256(&bytes),
        });
    }
    if out.is_empty() {
        return Err(format!("no English or German voice in {}", index.display()));
    }
    Ok(out)
}

/// Everything that speaks at `places`.
pub fn discover(places: &Places) -> Found {
    let mut recorded = places.recorded.clone();
    recorded.extend(places.tutorials.as_ref().map(|t| t.join("voices")));
    let mut found = Found {
        tutorials: places.tutorials.clone(),
        recorded,
        ..Found::default()
    };
    // S2 Pro.
    let command = places.mlx_speech.clone().or_else(|| {
        let local = places.home.as_ref()?.join(".local/bin/mlx-speech");
        local.is_file().then_some(local)
    });
    let python = places
        .fish_python
        .clone()
        .or_else(|| shebang(command.as_deref()?));
    match python.filter(|p| p.is_file()) {
        None => found
            .missing
            .push("Fish Audio S2 Pro: mlx-speech is not installed".into()),
        Some(python) => {
            let model = places
                .hf_hub
                .as_deref()
                .is_some_and(|h| in_hub(h, FISH_MODEL));
            let voices = places
                .voices
                .as_deref()
                .map_or(Err("no voices folder".into()), fish_voices);
            match (model, voices) {
                (false, _) => found.missing.push(format!(
                    "Fish Audio S2 Pro: its model {FISH_MODEL} is not in the Hugging Face cache"
                )),
                (true, Err(e)) => found.missing.push(format!("Fish Audio S2 Pro: {e}")),
                (true, Ok(voices)) => found.fish = Some(Fish { python, voices }),
            }
        }
    }
    // Kokoro.
    match places.tutorials.as_deref() {
        None => found
            .missing
            .push("Kokoro: no tutorial venv (make tutorial-venv)".into()),
        Some(work) => {
            let python = work.join("venv/bin/python");
            let hf_home = work.join("hf");
            if !python.exists() {
                found
                    .missing
                    .push("Kokoro: no tutorial venv (make tutorial-venv)".into());
            } else if !in_hub(&hf_home.join("hub"), KOKORO_REPO) {
                found
                    .missing
                    .push("Kokoro: its weights are not downloaded (make tutorial)".into());
            } else {
                found.kokoro = Some(Kokoro { python, hf_home });
            }
        }
    }
    found
}
