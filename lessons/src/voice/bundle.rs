//! The lessons' voices as downloads (ADR 0056): a bundle per course and
//! language holds every section the lesson reads, as AAC in MP4 with its
//! `words.json` and `meta.json`. The manifest (`bundles.json`, written by
//! `scripts/voice_bundles.py`) is embedded: per bundle its file, SHA-256,
//! size and section keys. When a lesson opens and a key of its bundle is in
//! none of the recordings' caches, the bundle is fetched in the background
//! from the release (`base_url`, the app's variable over it),
//! checked against its SHA-256 and unpacked into the lasting cache, each
//! section into a folder of its own, renamed into place; a section already
//! there is left as it is. Once all its keys are there it is never fetched
//! again. A failure is said in the voice info and tried again the next
//! time a lesson opens. Only the file is asked for: no headers of the
//! app's, nothing about the user.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::{Config, Lang};

/// The files a section of a bundle has.
const FILES: [&str; 3] = ["audio.m4a", "words.json", "meta.json"];
/// The largest bundle taken (the largest is a few MB).
const MAX_SIZE: u64 = 256 << 20;
/// Connecting and each read, in seconds.
const TIMEOUT: u64 = 60;

/// A course's sections in one language, as a file of the release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub course: String,
    pub lang: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
    pub keys: Vec<String>,
}

/// The bundles and where they are.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub base_url: String,
    pub bundles: Vec<Bundle>,
}

fn hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A file name of the release: letters, digits, `.`, `-`, `_`, ending in
/// `.tar`; never a path.
fn plain_file(s: &str) -> bool {
    s.ends_with(".tar")
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

impl Manifest {
    /// Reads a manifest of format 1.
    ///
    /// # Errors
    ///
    /// It is not JSON, of another format, or a bundle in it is malformed.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("the manifest: {e}"))?;
        if v["format"].as_u64() != Some(1) {
            return Err(format!("the manifest's format {}: 1 is read", v["format"]));
        }
        let base_url = v["base_url"].as_str().unwrap_or_default().to_owned();
        let mut bundles = Vec::new();
        for b in v["bundles"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let s = |k: &str| b[k].as_str().unwrap_or_default().to_owned();
            let keys: Vec<String> = (b["keys"].as_array().map(Vec::as_slice))
                .unwrap_or_default()
                .iter()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect();
            let bundle = Bundle {
                course: s("course"),
                lang: s("lang"),
                file: s("file"),
                sha256: s("sha256").to_ascii_lowercase(),
                size: b["size"].as_u64().unwrap_or(0),
                keys,
            };
            if bundle.course.is_empty()
                || bundle.lang.is_empty()
                || !plain_file(&bundle.file)
                || bundle.sha256.len() != 64
                || !hex(&bundle.sha256)
                || bundle.size == 0
                || bundle.size > MAX_SIZE
                || bundle.keys.is_empty()
                || !bundle.keys.iter().all(|k| hex(k))
            {
                return Err(format!("the manifest: a malformed bundle {}", b["file"]));
            }
            bundles.push(bundle);
        }
        Ok(Manifest { base_url, bundles })
    }

    /// The bundle of `course` in `lang`.
    pub fn of(&self, course: &str, lang: Lang) -> Option<&Bundle> {
        (self.bundles.iter()).find(|b| b.course == course && b.lang == lang.key())
    }

    /// Whether any course has a bundle in `lang`.
    pub fn any(&self, lang: Lang) -> bool {
        self.bundles.iter().any(|b| b.lang == lang.key())
    }

    /// Where `b` is downloaded from: the manifest's base, then its file.
    pub fn url(&self, b: &Bundle) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), b.file)
    }
}

/// SHA-256 of `bytes`, in hex.
fn sha256(bytes: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, bytes);
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Unpacks the bundle `b`, downloaded as `bytes`, into `root`: checked
/// against its size and SHA-256 first; only its own keys and files, each
/// section written into a folder of its own and renamed into place, one
/// already there left as it is. The number of sections written.
///
/// # Errors
///
/// It is not the bundle the manifest says, or a file cannot be written.
pub fn unpack(bytes: &[u8], b: &Bundle, root: &Path) -> Result<usize, String> {
    if bytes.len() as u64 != b.size || sha256(bytes) != b.sha256 {
        return Err(format!("{}: not the file expected (checksum)", b.file));
    }
    let bad = |e: std::io::Error| format!("{}: {e}", b.file);
    let mut sections: HashMap<String, Vec<(String, Vec<u8>)>> = HashMap::new();
    let mut archive = tar::Archive::new(bytes);
    for entry in archive.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(bad)?.into_owned();
        let parts: Vec<String> = (path.components())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let [key, name] = parts.as_slice() else {
            return Err(format!("{}: {} is no section's", b.file, path.display()));
        };
        if !b.keys.contains(key) || !FILES.contains(&name.as_str()) {
            return Err(format!("{}: {} is no section's", b.file, path.display()));
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(bad)?;
        sections
            .entry(key.clone())
            .or_default()
            .push((name.clone(), data));
    }
    for k in &b.keys {
        let has_audio = (sections.get(k)).is_some_and(|f| f.iter().any(|(n, _)| n == FILES[0]));
        if !has_audio {
            return Err(format!("{}: no audio for {k}", b.file));
        }
    }
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut written = 0;
    for (key, files) in sections {
        let to = root.join(&key);
        if to.exists() {
            continue;
        }
        let fail = |e: std::io::Error| format!("{}: {e}", root.display());
        let tmp = root.join(format!(".{key}.{}.bundle.tmp", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(fail)?;
        let done = (|| {
            for (name, data) in &files {
                std::fs::write(tmp.join(name), data)?;
            }
            std::fs::rename(&tmp, &to)
        })();
        if let Err(e) = done {
            let _ = std::fs::remove_dir_all(&tmp);
            // Another app put it there meanwhile: as good.
            if !to.join(FILES[0]).is_file() {
                return Err(fail(e));
            }
        } else {
            written += 1;
        }
    }
    Ok(written)
}

/// Fetches a URL: the body, with how much has come and of how much (if
/// said) along the way.
pub type Fetch =
    Arc<dyn Fn(&str, &dyn Fn(u64, Option<u64>)) -> Result<Vec<u8>, String> + Send + Sync>;

/// A GET of `url` by minreq (HTTPS with the system's trust store,
/// redirects followed): no headers but the request's own.
///
/// # Errors
///
/// No connection, a status but 200, or more than 256 MB.
pub fn http_get(url: &str, progress: &dyn Fn(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let mut r = minreq::get(url)
        .with_timeout(TIMEOUT)
        .send_lazy()
        .map_err(|e| e.to_string())?;
    if r.status_code != 200 {
        return Err(format!("HTTP {} {}", r.status_code, r.reason_phrase));
    }
    let total = (r.headers.iter())
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_SIZE) {
        return Err("too large".into());
    }
    let mut body = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        let n = r.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() as u64 > MAX_SIZE {
            return Err("too large".into());
        }
        progress(body.len() as u64, total);
    }
    Ok(body)
}

/// How a bundle's download goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loading {
    /// So many bytes of so many.
    Running {
        done: u64,
        total: u64,
    },
    /// Unpacked into the cache.
    Done,
    Failed(String),
}

/// The bundles being downloaded, each on a thread of its own.
pub struct Downloads {
    pub manifest: Manifest,
    /// The lasting cache the bundles are unpacked into.
    pub root: PathBuf,
    /// The app's name in lower case, for its log lines.
    log: String,
    fetch: Fetch,
    jobs: Arc<Mutex<HashMap<String, Loading>>>,
}

impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Downloads")
            .field("root", &self.root)
            .field("bundles", &self.manifest.bundles.len())
            .finish_non_exhaustive()
    }
}

impl Downloads {
    pub fn new(manifest: Manifest, root: PathBuf, fetch: Fetch) -> Downloads {
        Downloads {
            manifest,
            root,
            log: "egui-lessons".into(),
            fetch,
            jobs: Arc::default(),
        }
    }

    /// This machine's for the app of `config`: its manifest (the base its
    /// variable says, if set), into its lasting cache. None without a
    /// manifest, or with one that does not read (the app's test says so).
    pub fn here(config: &Config) -> Option<Downloads> {
        if config.voice.bundles.trim().is_empty() {
            return None;
        }
        let root = super::cache::lasting_dir(config.app)?;
        let mut manifest = Manifest::parse(config.voice.bundles).ok()?;
        if let Some(base) = (config.voice.bundles_env)
            .and_then(|k| std::env::var(k).ok())
            .filter(|s| !s.trim().is_empty())
        {
            manifest.base_url = base;
        }
        let mut d = Downloads::new(manifest, root, Arc::new(http_get));
        d.log = config.lower();
        Some(d)
    }

    /// How the download of `file` goes, if it was asked for.
    pub fn state(&self, file: &str) -> Option<Loading> {
        self.jobs.lock().ok()?.get(file).cloned()
    }

    /// `b` downloaded and unpacked in the background, unless it is
    /// already, or it failed since [`Downloads::forget_failed`].
    pub fn want(&self, b: &Bundle) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        if jobs.contains_key(&b.file) {
            return;
        }
        jobs.insert(
            b.file.clone(),
            Loading::Running {
                done: 0,
                total: b.size,
            },
        );
        drop(jobs);
        let (b, url) = (b.clone(), self.manifest.url(b));
        let (root, fetch, jobs) = (
            self.root.clone(),
            Arc::clone(&self.fetch),
            Arc::clone(&self.jobs),
        );
        let file = b.file.clone();
        let log = self.log.clone();
        let set = move |state: Loading| {
            if let Ok(mut j) = jobs.lock() {
                j.insert(file.clone(), state);
            }
        };
        let progress = {
            let set = set.clone();
            let size = b.size;
            move |done: u64, total: Option<u64>| {
                set(Loading::Running {
                    done,
                    total: total.unwrap_or(size),
                });
            }
        };
        let spawned = std::thread::Builder::new()
            .name("voice download".into())
            .spawn(move || {
                let got = fetch(&url, &progress).and_then(|bytes| unpack(&bytes, &b, &root));
                match got {
                    Ok(_) => set(Loading::Done),
                    Err(e) => {
                        eprintln!("{log}: voice download: {e}");
                        set(Loading::Failed(e));
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("{}: voice download: {e}", self.log);
        }
    }

    /// The failed downloads forgotten: tried again when next wanted (the
    /// next time a lesson opens).
    pub fn forget_failed(&self) {
        if let Ok(mut j) = self.jobs.lock() {
            j.retain(|_, s| !matches!(s, Loading::Failed(_)));
        }
    }
}
