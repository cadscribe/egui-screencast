//! Where a spoken section is (ADR 0056): the recordings' caches first,
//! then the app's own. The recordings keep S2 Pro's and ElevenLabs'
//! sections as `<folder>/<key>/audio.wav` (16-bit mono WAV, 24 kHz) with
//! `words.json` and `meta.json`, in a list of folders looked in in turn:
//! the lasting one in the user's data folder (`tutorial-voices`, where the
//! recordings write them so that a `make clean` never loses what was paid
//! for), `$SCREENCAST_VOICE_CACHE`, and the tutorials' work folder's
//! `voices`. Kokoro's are `<work>/audio/<key>.wav`. A downloaded bundle's
//! sections (`bundle.rs`) are unpacked into the lasting one, keyed alike,
//! their audio `audio.m4a` (AAC), Kokoro's too.
//! The app's own cache is laid out as the recordings' (`<key>/audio.wav`,
//! or `audio.m4a` for AAC a synth gave, and `meta.json`) and is the only
//! one it writes, but for the sections of a remote voice of the app's own,
//! which go into the lasting one: the recordings' sections are only read,
//! never pruned or changed.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::discover::Found;
use super::{Engine, Section};

fn env(k: &str) -> Option<PathBuf> {
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The app's cache of sections: `<OS cache dir>/<app>/voices`, the app's
/// name in lower case (`opensolve`).
pub fn app_dir(app: &str) -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Caches"))
    } else if cfg!(windows) {
        env("LOCALAPPDATA")
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    Some(base?.join(app.to_lowercase()).join("voices"))
}

/// The lasting cache of the recordings' sections, in the user's data
/// folder, not a cache folder the system may empty: on macOS
/// `~/Library/Application Support/<App>/tutorial-voices`, on Windows
/// `%APPDATA%\<App>\tutorial-voices`, else
/// `$XDG_DATA_HOME/<app>/tutorial-voices` (`~/.local/share/…`), `<App>`
/// the app's name (`OpenSolve`), `<app>` in lower case.
pub fn lasting_dir(app: &str) -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support").join(app))
    } else if cfg!(windows) {
        env("APPDATA").map(|a| a.join(app))
    } else {
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local/share")))
            .map(|d| d.join(app.to_lowercase()))
    };
    Some(base?.join("tutorial-voices"))
}

/// This machine's recordings' caches besides the tutorials' work folder:
/// the lasting one, then `$SCREENCAST_VOICE_CACHE`.
pub fn recorded_dirs(app: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = lasting_dir(app).into_iter().collect();
    if let Some(d) = env("SCREENCAST_VOICE_CACHE")
        && !out.contains(&d)
    {
        out.push(d);
    }
    out
}

/// The section `key`'s audio in the first of `folders` that has it: a
/// recording's `audio.wav`, or a downloaded bundle's `audio.m4a`.
pub fn in_folders(key: &str, folders: &[PathBuf]) -> Option<PathBuf> {
    folders
        .iter()
        .flat_map(|d| ["audio.wav", "audio.m4a"].map(|f| d.join(key).join(f)))
        .find(|p| p.is_file())
}

/// The section's audio, wherever it was spoken before: the recordings'
/// caches of what `found` has, then the app's.
pub fn find(s: &Section, found: &Found, app: Option<&Path>) -> Option<PathBuf> {
    let recorded = match s.engine {
        Engine::Fish | Engine::ElevenLabs | Engine::Remote => in_folders(&s.key, &found.recorded),
        Engine::Kokoro => (found.tutorials.as_deref())
            .map(|t| t.join("audio").join(format!("{}.wav", s.key)))
            .filter(|p| p.is_file())
            .or_else(|| in_folders(&s.key, &found.recorded)),
    };
    recorded.or_else(|| in_folders(&s.key, &[app?.to_path_buf()]))
}

/// Keeps `wav` as the section `key` in `root`: written into a folder of
/// its own, then renamed into place whole, as the recordings' cache does;
/// `audio.m4a` where its bytes are AAC in an MP4 ([`super::player::is_mp4`]),
/// else `audio.wav`. `root` is the app's own cache, or the lasting one for
/// a remote section, never another of the recordings'.
///
/// # Errors
///
/// A file cannot be written.
pub fn put(root: &Path, key: &str, wav: &Path, meta: &Value) -> Result<PathBuf, String> {
    let fail = |e: std::io::Error| format!("{}: {e}", root.display());
    let tmp = root.join(format!(".{key}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(fail)?;
    let name = if super::player::is_mp4_file(wav) {
        "audio.m4a"
    } else {
        "audio.wav"
    };
    let done = (|| {
        std::fs::rename(wav, tmp.join(name)).or_else(|_| {
            std::fs::copy(wav, tmp.join(name))?;
            std::fs::remove_file(wav)
        })?;
        let text = serde_json::to_string_pretty(meta).unwrap_or_default();
        std::fs::write(tmp.join("meta.json"), text)?;
        let to = root.join(key);
        let _ = std::fs::remove_dir_all(&to);
        std::fs::rename(&tmp, &to)?;
        Ok(to.join(name))
    })()
    .map_err(fail);
    let _ = std::fs::remove_dir_all(&tmp);
    done
}
