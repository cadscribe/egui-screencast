//! The pictures written on threads of their own, so that the app does not
//! wait for PNG encoding.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use eframe::egui::ColorImage;

/// A picture to write.
pub(crate) struct Job {
    pub image: Arc<ColorImage>,
    pub path: PathBuf,
    /// Scaled to this width, else as taken.
    pub width: Option<u32>,
}

fn write_png(job: &Job) -> Result<(), String> {
    let [w, h] = job.image.size;
    let rgb: Vec<u8> = job
        .image
        .pixels
        .iter()
        .flat_map(|p| {
            let [r, g, b, _] = p.to_array();
            [r, g, b]
        })
        .collect();
    let img = image::RgbImage::from_raw(w as u32, h as u32, rgb).ok_or("a picture of no size")?;
    let img = match job.width.filter(|&x| x < w as u32) {
        Some(x) => {
            let y = (h as f64 * f64::from(x) / w as f64).round() as u32;
            image::imageops::resize(&img, x, y, image::imageops::FilterType::Lanczos3)
        }
        None => img,
    };
    let file =
        std::fs::File::create(&job.path).map_err(|e| format!("{}: {e}", job.path.display()))?;
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        std::io::BufWriter::new(file),
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    );
    img.write_with_encoder(encoder)
        .map_err(|e| format!("{}: {e}", job.path.display()))
}

/// The frames of the video so far: each file and how many frames it
/// stands for.
#[derive(Default)]
pub(crate) struct Frames {
    pub files: Vec<(String, u32)>,
    pub last: Option<Arc<ColorImage>>,
    /// Pictures asked for and not yet taken.
    pub pending: usize,
    /// The size of the frames, in pixels.
    pub size: [usize; 2],
}

/// Threads writing the pictures.
pub(crate) struct Writer {
    pub tx: Option<SyncSender<Job>>,
    threads: Vec<std::thread::JoinHandle<Vec<String>>>,
}

impl Writer {
    pub fn new() -> Writer {
        let (tx, rx) = sync_channel::<Job>(8);
        let rx = Arc::new(Mutex::new(rx));
        let n = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 6));
        let threads = (0..n)
            .map(|_| {
                let rx: Arc<Mutex<Receiver<Job>>> = rx.clone();
                std::thread::spawn(move || {
                    let mut errors = Vec::new();
                    loop {
                        let job = match rx.lock() {
                            Ok(r) => r.recv(),
                            Err(_) => break,
                        };
                        let Ok(job) = job else { break };
                        if let Err(e) = write_png(&job) {
                            errors.push(e);
                        }
                    }
                    errors
                })
            })
            .collect();
        Writer {
            tx: Some(tx),
            threads,
        }
    }

    /// Waits for every picture; the errors writing them.
    pub fn finish(&mut self) -> Vec<String> {
        self.tx = None;
        self.threads
            .drain(..)
            .flat_map(|t| t.join().unwrap_or_default())
            .collect()
    }
}
