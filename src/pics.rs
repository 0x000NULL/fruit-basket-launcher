//! Pictures for covers and save slots: PNGs decoded on a thread, shrunk,
//! and kept by path. Drawing never waits for one; a picture that isn't
//! back yet draws the placeholder.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use tiny_skia::Pixmap;

use crate::art;

/// Pictures are kept at most this tall; covers draw smaller than that.
const MAX_H: u32 = 320;
/// A bigger file is not a picture worth reading.
const MAX_BYTES: u64 = 16 << 20;

pub struct Pics {
    /// Decoded pictures; `None` for a file that didn't decode.
    map: HashMap<PathBuf, Option<Pixmap>>,
    /// Asked for and not back yet.
    pending: HashSet<PathBuf>,
    tx: Sender<PathBuf>,
    rx: Receiver<(PathBuf, Option<Pixmap>)>,
}

impl Default for Pics {
    fn default() -> Pics {
        let (tx, jobs) = channel::<PathBuf>();
        let (done, rx) = channel();
        thread::spawn(move || {
            for path in jobs {
                let pic = load(&path);
                if done.send((path, pic)).is_err() {
                    break;
                }
            }
        });
        Pics { map: HashMap::new(), pending: HashSet::new(), tx, rx }
    }
}

impl Pics {
    /// Ask for `path`, once; `get` has it when the thread is done.
    pub fn want(&mut self, path: &Path) {
        if !self.map.contains_key(path) && self.pending.insert(path.to_path_buf()) {
            let _ = self.tx.send(path.to_path_buf());
        }
    }

    pub fn get(&self, path: &Path) -> Option<&Pixmap> {
        self.map.get(path)?.as_ref()
    }

    /// Read every picture again (a game wrote new saves); the old ones
    /// draw until the new ones are back.
    pub fn reload(&mut self) {
        for p in self.map.keys() {
            if self.pending.insert(p.clone()) {
                let _ = self.tx.send(p.clone());
            }
        }
    }

    /// Take what the thread finished. True if anything arrived.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        while let Ok((path, pic)) = self.rx.try_recv() {
            self.pending.remove(&path);
            self.map.insert(path, pic);
            got = true;
        }
        got
    }

    /// Wait for everything asked for: the render tests draw what's loaded.
    #[cfg(test)]
    pub fn settle(&mut self) {
        while !self.pending.is_empty() {
            let Ok((path, pic)) = self.rx.recv_timeout(std::time::Duration::from_secs(10)) else { break };
            self.pending.remove(&path);
            self.map.insert(path, pic);
        }
    }
}

/// Read and decode a PNG, shrunk by a whole factor to at most `MAX_H` tall.
pub fn load(path: &Path) -> Option<Pixmap> {
    if std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let pm = art::decode_png(&std::fs::read(path).ok()?)?;
    let f = pm.height().div_ceil(MAX_H);
    Some(if f > 1 { art::shrink(&pm, f) } else { pm })
}

/// A test picture: `w`×`h` in one colour, as PNG bytes.
#[cfg(test)]
pub fn png_bytes(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w8 = enc.write_header().unwrap();
        let data: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
        w8.write_image_data(&data).unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_shrinks_and_skips_bad_files() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big.png");
        std::fs::write(&big, png_bytes(640, 448, [200, 40, 40])).unwrap();
        let pm = load(&big).unwrap();
        assert_eq!((pm.width(), pm.height()), (320, 224), "halved to fit 320");
        let bad = tmp.path().join("bad.png");
        std::fs::write(&bad, "not a png").unwrap();
        assert!(load(&bad).is_none());

        let mut pics = Pics::default();
        pics.want(&big);
        pics.want(&bad);
        pics.want(&big);
        pics.settle();
        assert!(pics.get(&big).is_some());
        assert!(pics.get(&bad).is_none());
        assert!(pics.pending.is_empty());
    }
}
