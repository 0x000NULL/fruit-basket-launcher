//! Disc probe: which console a disc image is for, read from a few KB of
//! it. Used only when more than one installed fruit claims a file in an
//! extra folder (`.iso` is Pomegranate's, Fig's, Kumquat's and Mango's);
//! a file in a fruit's own `games/` folder never gets here.
//!
//! - GameCube: the magic C2339F3D at 1Ch (`.iso`, `.gcm`), or in the first
//!   data block of a CISO.
//! - PSP: the ISO9660 volume's system identifier, `PSP GAME`.
//! - PS2 / PS1: SYSTEM.CNF in the root directory says `BOOT2` or `BOOT`.
//!   Cooked (2048) and raw (2352, mode 1 or 2) sectors are both read.
//! - Saturn: `SEGA SEGASATURN ` at the start of sector 0. Dreamcast:
//!   `SEGA SEGAKATANA ` at the start of IP.BIN, the first sector of track 3
//!   (the high-density data track). Cooked or raw (16-byte header) sectors.
//! - `.cue`: the first `FILE` it names, then (for a Dreamcast) track 3's.
//!   `.gdi`: track 3's file. `.zip`: its first disc-like entry, read in
//!   place when stored, inflated only as far as needed otherwise.
//! - `.chd`: a GD-ROM's track metadata says Dreamcast outright. A CD or
//!   DVD is read through the hunks the markers above need (the chd crate
//!   decompresses them; the last one is kept). Opening a CHD reads its
//!   whole hunk map, so one whose map is over `MAP_LIMIT` stays unknown.
//!
//! Probes run on a thread and the scan waits at most `WAIT` for them, so a
//! slow network share can't hang the window: what hasn't answered falls
//! back to feed order, and the answer, when it comes, is cached by path,
//! size and mtime. The scan then hands back a `Late`, and the Library scans
//! once more when it drains.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::feed::Fruit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    GameCube,
    Ps2,
    Ps1,
    Psp,
    Saturn,
    Dreamcast,
}

impl Kind {
    /// The feed's `system` for this console.
    pub fn system(self) -> &'static str {
        match self {
            Kind::GameCube => "GameCube",
            Kind::Ps2 => "PS2",
            Kind::Ps1 => "PS1",
            Kind::Psp => "PSP",
            Kind::Saturn => "Saturn",
            Kind::Dreamcast => "Dreamcast",
        }
    }
}

/// How long one scan waits for its probes.
const WAIT: Duration = Duration::from_secs(2);
/// The furthest into a deflated zip entry a probe inflates.
const INFLATE_LIMIT: u64 = 4 << 20;
/// Root directory sectors read when looking for SYSTEM.CNF.
const ROOT_SECTORS: u32 = 4;
const GC_MAGIC: [u8; 4] = [0xC2, 0x33, 0x9F, 0x3D];
/// Entries in a zip that may be a disc image.
const DISC_EXT: [&str; 5] = [".iso", ".gcm", ".ciso", ".bin", ".img"];
/// The largest CHD hunk map read. A dual-layer DVD (8.5 GB) in 2048-byte
/// hunks has 4.15 M of them, and compressed maps run 1.7 to 2.2 bytes a
/// hunk (measured on PSP CHDs), so about 9 MB: 16 MB holds every real disc
/// with room to spare, and a 2 s wait still covers reading it from a share.
const MAP_LIMIT: u64 = 16 << 20;
/// The most hunks a CHD may have. `Chd::open` allocates the expanded map,
/// 12 bytes a hunk, from the header's count before it reads a byte of the
/// map, so a corrupt count (say 2^40 hunks) would fail that allocation,
/// and a failed allocation aborts the process: `catch_unwind` can't catch
/// it. So the count is checked here first. 4.5 M covers the dual-layer DVD
/// above (54 MB, briefly).
const HUNK_COUNT_LIMIT: u64 = 4_500_000;
/// The largest hunk (CD hunks are 19584 bytes, DVD ones 2048 or 4096).
const HUNK_LIMIT: u32 = 1 << 20;
/// Hunks one probe may decompress.
const HUNK_READS: u32 = 32;

#[cfg(test)]
thread_local! {
    /// A test's own `WAIT`, for this thread's scans.
    pub static TEST_WAIT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

fn wait() -> Duration {
    #[cfg(test)]
    if let Some(w) = TEST_WAIT.with(std::cell::Cell::get) {
        return w;
    }
    WAIT
}

/// The probes a scan stopped waiting for, still running on their thread.
pub struct Late(Receiver<(PathBuf, Option<Kind>)>);

impl Late {
    /// True once the thread is done (its answers are in the cache).
    pub fn drained(&self) -> bool {
        loop {
            match self.0.try_recv() {
                Ok(_) => {}
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => return true,
            }
        }
    }
}

type Key = (PathBuf, u64, i64);
static CACHE: LazyLock<Mutex<HashMap<Key, Option<Kind>>>> = LazyLock::new(Mutex::default);
/// Paths a probe thread is still reading (maybe stuck on a share).
static BUSY: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Mutex::default);

/// Of `claims` (the fruits that play `path`, in feed order, at least
/// one), the one whose `system` is `kind`'s; else the first.
pub fn pick<'a>(claims: &[&'a Fruit], kind: Option<Kind>) -> &'a Fruit {
    kind.and_then(|k| claims.iter().find(|f| f.system.trim().eq_ignore_ascii_case(k.system())))
        .unwrap_or(&claims[0])
}

/// The kind of each of `paths`, from the cache or a probe thread; a path
/// missing from the result is unknown, or still being read: then the
/// `Late` says when the thread is done.
pub fn kinds(paths: Vec<PathBuf>) -> (HashMap<PathBuf, Option<Kind>>, Option<Late>) {
    let n = paths.len();
    if n == 0 {
        return (HashMap::new(), None);
    }
    let (tx, rx) = channel();
    thread::spawn(move || {
        for path in paths {
            if !BUSY.lock().unwrap().insert(path.clone()) {
                continue;
            }
            let kind = cached(&path);
            BUSY.lock().unwrap().remove(&path);
            if tx.send((path, kind)).is_err() {
                // The scan stopped waiting; keep filling the cache.
                continue;
            }
        }
    });
    let deadline = Instant::now() + wait();
    let mut out = HashMap::new();
    while out.len() < n {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
        match rx.recv_timeout(left) {
            Ok((path, kind)) => {
                out.insert(path, kind);
            }
            Err(_) => break,
        }
    }
    let late = (out.len() < n).then(|| Late(rx));
    (out, late)
}

fn cached(path: &Path) -> Option<Kind> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let key = (path.to_path_buf(), meta.len(), mtime);
    if let Some(k) = CACHE.lock().unwrap().get(&key) {
        return *k;
    }
    let kind = probe(path);
    CACHE.lock().unwrap().insert(key, kind);
    kind
}

/// Which console the file at `path` is for, if a few KB of it say so.
pub fn probe(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".zip") {
        zip(path)
    } else if name.ends_with(".cue") {
        cue(path)
    } else if name.ends_with(".gdi") {
        gdi(path)
    } else if name.ends_with(".chd") {
        chd(path)
    } else {
        image(&mut open(path)?)
    }
}

/// Random-ish access to an image: fill `buf` from `off`, or false.
trait At {
    fn at(&mut self, off: u64, buf: &mut [u8]) -> bool;
}

/// A file, or a stored zip entry starting at `base`.
struct Seekable {
    f: File,
    base: u64,
}

impl At for Seekable {
    fn at(&mut self, off: u64, buf: &mut [u8]) -> bool {
        self.f.seek(SeekFrom::Start(self.base + off)).is_ok() && self.f.read_exact(buf).is_ok()
    }
}

/// A deflated zip entry, inflated as far as asked and kept, never past
/// `INFLATE_LIMIT`.
struct Stream<R> {
    r: R,
    got: Vec<u8>,
}

impl<R: Read> At for Stream<R> {
    fn at(&mut self, off: u64, buf: &mut [u8]) -> bool {
        let end = off + buf.len() as u64;
        if end > INFLATE_LIMIT {
            return false;
        }
        let more = end.saturating_sub(self.got.len() as u64);
        if (&mut self.r).take(more).read_to_end(&mut self.got).is_err() || (self.got.len() as u64) < end {
            return false;
        }
        buf.copy_from_slice(&self.got[off as usize..end as usize]);
        true
    }
}

fn image(src: &mut impl At) -> Option<Kind> {
    gamecube(src).or_else(|| sega(src)).or_else(|| iso(src))
}

/// A Saturn or Dreamcast boot sector at the start of `src`, cooked or raw.
fn sega(src: &mut impl At) -> Option<Kind> {
    [0, 16].into_iter().find_map(|head| {
        let mut id = [0u8; 16];
        if !src.at(head, &mut id) {
            return None;
        }
        match &id {
            b"SEGA SEGASATURN " => Some(Kind::Saturn),
            b"SEGA SEGAKATANA " => Some(Kind::Dreamcast),
            _ => None,
        }
    })
}

fn open(path: &Path) -> Option<Seekable> {
    Some(Seekable { f: File::open(path).ok()?, base: 0 })
}

fn gamecube(src: &mut impl At) -> Option<Kind> {
    let mut head = [0u8; 0x20];
    if !src.at(0, &mut head) {
        return None;
    }
    if head[0x1C..0x20] == GC_MAGIC {
        return Some(Kind::GameCube);
    }
    // CISO: a 32 KB header (magic, block size, block map), then block 0.
    let mut magic = [0u8; 4];
    (&head[..4] == b"CISO" && head[8] == 1 && src.at(0x8000 + 0x1C, &mut magic) && magic == GC_MAGIC).then_some(Kind::GameCube)
}

/// An ISO9660 disc: PSP by its system identifier, PS2 or PS1 by SYSTEM.CNF.
fn iso(src: &mut impl At) -> Option<Kind> {
    // Cooked sectors, then raw mode 1, then raw mode 2 (form 1).
    let (size, head) = [(2048u64, 0u64), (2352, 16), (2352, 24)].into_iter().find(|&(size, head)| {
        let mut id = [0u8; 6];
        src.at(16 * size + head, &mut id) && &id == b"\x01CD001"
    })?;
    let mut sector = |lba: u32, buf: &mut [u8; 2048]| src.at(lba as u64 * size + head, buf);
    let mut pvd = [0u8; 2048];
    if !sector(16, &mut pvd) {
        return None;
    }
    if pvd[8..40].starts_with(b"PSP GAME") {
        return Some(Kind::Psp);
    }
    let root = u32le(&pvd[158..]);
    let root_len = u32le(&pvd[166..]);
    let mut cnf = None;
    'dir: for i in 0..root_len.div_ceil(2048).min(ROOT_SECTORS) {
        let mut dir = [0u8; 2048];
        if !sector(root + i, &mut dir) {
            return None;
        }
        let mut at = 0;
        while at + 33 < dir.len() && dir[at] != 0 {
            let len = dir[at] as usize;
            let name_len = dir[at + 32] as usize;
            let name = dir.get(at + 33..at + 33 + name_len)?;
            let name = name.split(|&b| b == b';').next().unwrap_or(name);
            if name.eq_ignore_ascii_case(b"SYSTEM.CNF") {
                cnf = Some((u32le(&dir[at + 2..]), u32le(&dir[at + 10..])));
                break 'dir;
            }
            at += len;
        }
    }
    let (lba, len) = cnf?;
    let mut text = [0u8; 2048];
    if !sector(lba, &mut text) {
        return None;
    }
    let text = String::from_utf8_lossy(&text[..(len as usize).min(2048)]).to_ascii_uppercase();
    let keys: Vec<&str> = text.lines().map(|l| l.trim_start()).collect();
    if keys.iter().any(|l| l.starts_with("BOOT2")) {
        Some(Kind::Ps2)
    } else if keys.iter().any(|l| l.starts_with("BOOT")) {
        Some(Kind::Ps1)
    } else {
        None
    }
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes(b[..4].try_into().unwrap())
}

/// A cue sheet: probe the first file it names, beside it; failing that,
/// look for a Dreamcast's IP.BIN in track 3's file.
fn cue(path: &Path) -> Option<Kind> {
    let text = small_text(path)?;
    // (file, first track in it), in order.
    let mut files: Vec<(&str, u32)> = Vec::new();
    for line in text.lines().map(str::trim) {
        let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        if key.eq_ignore_ascii_case("FILE") {
            let name = match rest.strip_prefix('"') {
                Some(q) => q.split('"').next(),
                None => rest.split_whitespace().next(),
            };
            files.extend(name.map(|n| (n, 0)));
        } else if key.eq_ignore_ascii_case("TRACK")
            && let Some(last) = files.last_mut().filter(|f| f.1 == 0)
        {
            last.1 = rest.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    let dir = path.parent()?;
    if let Some(k) = image(&mut open(&dir.join(files.first()?.0))?) {
        return Some(k);
    }
    let (three, _) = files.iter().find(|f| f.1 == 3)?;
    sega(&mut open(&dir.join(three))?).filter(|k| *k == Kind::Dreamcast)
}

/// A GD-ROM sheet (`count`, then `track lba type sector_size file offset`
/// a line): IP.BIN starts track 3's file.
fn gdi(path: &Path) -> Option<Kind> {
    let text = small_text(path)?;
    let line = text.lines().skip(1).map(str::trim).find(|l| l.split_whitespace().next() == Some("3"))?;
    let name = match line.split_once('"') {
        Some((_, q)) => q.split('"').next()?,
        None => line.split_whitespace().nth(4)?,
    };
    sega(&mut open(&path.parent()?.join(name))?).filter(|k| *k == Kind::Dreamcast)
}

/// A cue or gdi sheet's text, never more than 64 KB of it.
fn small_text(path: &Path) -> Option<String> {
    let mut text = String::new();
    File::open(path).ok()?.take(64 << 10).read_to_string(&mut text).ok()?;
    Some(text)
}

/// A CHD: GD-ROM metadata is a Dreamcast; a CD or DVD is read through its
/// hunks like any other image. Unknown when the map is too big, and on any
/// error or panic inside the chd crate.
fn chd(path: &Path) -> Option<Kind> {
    let mut f = File::open(path).ok()?;
    let mut head = [0u8; 124];
    let n = f.read(&mut head).ok()?;
    let (map, hunks, hunk) = chd_sizes(&head[..n], &mut f)?;
    if map > MAP_LIMIT || hunks > HUNK_COUNT_LIMIT || hunk == 0 || hunk > HUNK_LIMIT {
        return None;
    }
    f.seek(SeekFrom::Start(0)).ok()?;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| chd_kind(f))).ok().flatten()
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn be64(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().unwrap())
}

/// From a CHD header: the bytes its map takes, its hunk count and hunk size.
fn chd_sizes(head: &[u8], f: &mut File) -> Option<(u64, u64, u32)> {
    if head.len() < 64 || &head[..8] != b"MComprHD" {
        return None;
    }
    match be32(head, 12) {
        5 if head.len() >= 124 => {
            let hunk = be32(head, 56);
            let hunks = be64(head, 32).div_ceil(hunk.max(1) as u64);
            // No compressors: 4 bytes a hunk. Otherwise the compressed
            // map's own header says how long it is.
            let map = if be32(head, 16) == 0 {
                hunks * 4
            } else {
                let mut len = [0u8; 4];
                f.seek(SeekFrom::Start(be64(head, 40))).ok()?;
                f.read_exact(&mut len).ok()?;
                16 + u32::from_be_bytes(len) as u64
            };
            Some((map, hunks, hunk))
        }
        3 | 4 => {
            let hunks = be32(head, 24) as u64;
            let hunk = be32(head, if be32(head, 12) == 3 { 76 } else { 44 });
            Some((hunks * 16, hunks, hunk))
        }
        _ => None,
    }
}

const fn tag(t: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*t)
}

fn chd_kind(f: File) -> Option<Kind> {
    let mut chd = chd::Chd::open(io::BufReader::new(f), None).ok()?;
    let refs: Vec<_> = chd.metadata_refs().collect();
    // (virtual sector, bytes per stored frame).
    let mut layout = None;
    for r in &refs {
        use chd::metadata::MetadataTag;
        match r.metatag() {
            t if t == tag(b"CHGD") || t == tag(b"CHGT") => return Some(Kind::Dreamcast),
            t if t == tag(b"DVD ") => layout = layout.or(Some((2048, 2048))),
            t if t == tag(b"CHT2") || t == tag(b"CHTR") => {
                if layout.is_none() {
                    // The first track: a cooked one keeps 2048 bytes a frame.
                    let text = r.read(chd.inner()).ok()?.value;
                    let text = String::from_utf8_lossy(&text);
                    let kind = text.split_whitespace().find_map(|w| w.strip_prefix("TYPE:")).unwrap_or("");
                    let cooked = kind == "MODE1" || kind == "MODE2_FORM1";
                    layout = Some((if cooked { 2048 } else { 2352 }, 2448));
                }
            }
            t if t == tag(b"CHCD") => layout = layout.or(Some((2352, 2448))),
            _ => {}
        }
    }
    let (sector, frame) = layout?;
    let buf = chd.get_hunksized_buffer();
    image(&mut ChdAt { chd, sector, frame, hunk: None, buf, cmp: Vec::new(), reads: 0 })
}

/// A CHD's data as a plain image: `sector`-byte sectors, each stored at the
/// start of a `frame`-byte frame (a CD frame carries 96 subcode bytes).
struct ChdAt<F: Read + Seek> {
    chd: chd::Chd<F>,
    sector: u64,
    frame: u64,
    hunk: Option<u32>,
    buf: Vec<u8>,
    cmp: Vec<u8>,
    reads: u32,
}

impl<F: Read + Seek> ChdAt<F> {
    /// Fill `out` from byte `off` of the CHD's logical data.
    fn raw(&mut self, mut off: u64, out: &mut [u8]) -> bool {
        let size = self.buf.len() as u64;
        let mut done = 0;
        while done < out.len() {
            let h = u32::try_from(off / size).unwrap_or(u32::MAX);
            if self.hunk != Some(h) {
                self.hunk = None;
                if self.reads >= HUNK_READS {
                    return false;
                }
                self.reads += 1;
                let Ok(mut hunk) = self.chd.hunk(h) else { return false };
                if hunk.read_hunk_in(&mut self.cmp, &mut self.buf).is_err() {
                    return false;
                }
                self.hunk = Some(h);
            }
            let at = (off % size) as usize;
            let n = (out.len() - done).min(self.buf.len() - at);
            out[done..done + n].copy_from_slice(&self.buf[at..at + n]);
            done += n;
            off += n as u64;
        }
        true
    }
}

impl<F: Read + Seek> At for ChdAt<F> {
    fn at(&mut self, off: u64, buf: &mut [u8]) -> bool {
        let mut done = 0;
        while done < buf.len() {
            let pos = off + done as u64;
            let (s, within) = (pos / self.sector, pos % self.sector);
            let n = (buf.len() - done).min((self.sector - within) as usize);
            if !self.raw(s * self.frame + within, &mut buf[done..done + n]) {
                return false;
            }
            done += n;
        }
        true
    }
}

/// A zip: probe its first disc-like entry.
fn zip(path: &Path) -> Option<Kind> {
    let mut z = zip::ZipArchive::new(io::BufReader::new(File::open(path).ok()?)).ok()?;
    let i = (0..z.len()).find(|&i| {
        z.name_for_index(i).is_some_and(|n| {
            let n = n.to_ascii_lowercase();
            DISC_EXT.iter().any(|e| n.ends_with(e))
        })
    })?;
    let entry = z.by_index(i).ok()?;
    match entry.compression() {
        zip::CompressionMethod::Stored => {
            let base = entry.data_start()?;
            image(&mut Seekable { f: File::open(path).ok()?, base })
        }
        _ => image(&mut Stream { r: entry, got: Vec::new() }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    /// A cooked ISO9660 image: the PVD at sector 16 with system id `sys`,
    /// the root directory at 18 and, when `cnf` is given, SYSTEM.CNF at 19.
    pub(crate) fn iso_image(sys: &str, cnf: Option<&str>) -> Vec<u8> {
        sectors(2048, 0, sys, cnf)
    }

    /// The same image in raw 2352-byte sectors with a `head`-byte header.
    pub(crate) fn raw_image(head: usize, sys: &str, cnf: Option<&str>) -> Vec<u8> {
        sectors(2352, head, sys, cnf)
    }

    fn sectors(size: usize, head: usize, sys: &str, cnf: Option<&str>) -> Vec<u8> {
        let mut data = vec![[0u8; 2048]; 20];
        let pvd = &mut data[16];
        pvd[..6].copy_from_slice(b"\x01CD001");
        pvd[8..8 + sys.len()].copy_from_slice(sys.as_bytes());
        pvd[158..162].copy_from_slice(&18u32.to_le_bytes());
        pvd[166..170].copy_from_slice(&2048u32.to_le_bytes());
        let mut at = 0;
        let mut record = |dir: &mut [u8; 2048], name: &[u8], lba: u32, len: u32| {
            let rl = 33 + name.len() + (name.len() + 1) % 2;
            dir[at] = rl as u8;
            dir[at + 2..at + 6].copy_from_slice(&lba.to_le_bytes());
            dir[at + 10..at + 14].copy_from_slice(&len.to_le_bytes());
            dir[at + 32] = name.len() as u8;
            dir[at + 33..at + 33 + name.len()].copy_from_slice(name);
            at += rl;
        };
        let mut dir = [0u8; 2048];
        record(&mut dir, b"\0", 18, 2048);
        record(&mut dir, b"\x01", 18, 2048);
        record(&mut dir, b"README.TXT;1", 0, 0);
        if let Some(text) = cnf {
            record(&mut dir, b"SYSTEM.CNF;1", 19, text.len() as u32);
            data[19][..text.len()].copy_from_slice(text.as_bytes());
        }
        data[18] = dir;
        data.iter().flat_map(|s| {
            let mut out = vec![0u8; size];
            out[head..head + 2048].copy_from_slice(s);
            out
        }).collect()
    }

    pub(crate) fn gc_image() -> Vec<u8> {
        let mut img = vec![0u8; 0x440];
        img[..6].copy_from_slice(b"GALE01");
        img[0x1C..0x20].copy_from_slice(&GC_MAGIC);
        img
    }

    pub(crate) const PS2_CNF: &str = "BOOT2 = cdrom0:\\SLUS_203.12;1\r\nVER = 1.00\r\nVMODE = NTSC\r\n";
    pub(crate) const PS1_CNF: &str = "BOOT = cdrom:\\SLUS_007.08;1\r\nTCB = 4\r\nEVENT = 10\r\nSTACK = 801FFF00\r\n";

    fn ciso(gc: bool) -> Vec<u8> {
        let mut img = vec![0u8; 0x8000];
        img[..4].copy_from_slice(b"CISO");
        img[4..8].copy_from_slice(&0x200000u32.to_le_bytes());
        img[8] = 1;
        img.extend(if gc { gc_image() } else { vec![0u8; 0x440] });
        img
    }

    fn zip_one(path: &Path, name: &str, data: &[u8], method: zip::CompressionMethod) {
        let mut w = zip::ZipWriter::new(fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(method);
        w.start_file("readme.txt", opts).unwrap();
        w.write_all(b"hi").unwrap();
        w.start_file(name, opts).unwrap();
        w.write_all(data).unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn probes_disc_images() {
        let t = tempfile::tempdir().unwrap();
        let put = |n: &str, d: &[u8]| {
            let p = t.path().join(n);
            fs::write(&p, d).unwrap();
            probe(&p)
        };
        assert_eq!(put("melee.iso", &gc_image()), Some(Kind::GameCube));
        assert_eq!(put("melee.GCM", &gc_image()), Some(Kind::GameCube));
        assert_eq!(put("melee.ciso", &ciso(true)), Some(Kind::GameCube));
        assert_eq!(put("other.ciso", &ciso(false)), None);
        assert_eq!(put("ffx.iso", &iso_image("PLAYSTATION", Some(PS2_CNF))), Some(Kind::Ps2));
        assert_eq!(put("ff7.iso", &iso_image("PLAYSTATION", Some(PS1_CNF))), Some(Kind::Ps1));
        assert_eq!(put("ff7raw.iso", &raw_image(24, "PLAYSTATION", Some(PS1_CNF))), Some(Kind::Ps1));
        assert_eq!(put("mode1.iso", &raw_image(16, "PLAYSTATION", Some(PS2_CNF))), Some(Kind::Ps2));
        assert_eq!(put("lumines.iso", &iso_image("PSP GAME", None)), Some(Kind::Psp));
        // An ISO9660 disc without SYSTEM.CNF, a short file, an empty one.
        assert_eq!(put("data.iso", &iso_image("WIN32", None)), None);
        assert_eq!(put("cnf-no-boot.iso", &iso_image("PLAYSTATION", Some("VMODE = PAL\r\n"))), None);
        assert_eq!(put("short.iso", b"CD001"), None);
        assert_eq!(put("empty.iso", b""), None);
        assert_eq!(probe(&t.path().join("missing.iso")), None);
    }

    #[test]
    fn probes_cue_sheets() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("FF7 (Track 1).bin"), raw_image(24, "PLAYSTATION", Some(PS1_CNF))).unwrap();
        fs::write(t.path().join("ff7.cue"), "FILE \"FF7 (Track 1).bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n").unwrap();
        assert_eq!(probe(&t.path().join("ff7.cue")), Some(Kind::Ps1));
        fs::write(t.path().join("dq8.bin"), raw_image(24, "PLAYSTATION", Some(PS2_CNF))).unwrap();
        fs::write(t.path().join("dq8.cue"), "REM x\nfile dq8.bin BINARY\n").unwrap();
        assert_eq!(probe(&t.path().join("dq8.cue")), Some(Kind::Ps2));
        fs::write(t.path().join("lost.cue"), "FILE \"gone.bin\" BINARY\n").unwrap();
        assert_eq!(probe(&t.path().join("lost.cue")), None);
    }

    /// A Saturn or Dreamcast boot sector, `head` bytes into sector 0.
    pub(crate) fn sega_sector(id: &[u8; 16], head: usize) -> Vec<u8> {
        let size = if head == 0 { 2048 } else { 2352 };
        let mut img = vec![0u8; size * 16];
        img[head..head + 16].copy_from_slice(id);
        img
    }

    #[test]
    fn probes_saturn_and_dreamcast() {
        let t = tempfile::tempdir().unwrap();
        let p = |n: &str| t.path().join(n);
        let put = |n: &str, d: &[u8]| fs::write(p(n), d).unwrap();
        put("Nights.iso", &sega_sector(b"SEGA SEGASATURN ", 0));
        assert_eq!(probe(&p("Nights.iso")), Some(Kind::Saturn));
        put("Nights (Track 1).bin", &sega_sector(b"SEGA SEGASATURN ", 16));
        put("Nights (Track 2).bin", &[0u8; 4704]);
        put("Nights.cue", b"FILE \"Nights (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"Nights (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n");
        assert_eq!(probe(&p("Nights.cue")), Some(Kind::Saturn));

        // Redump's Dreamcast cue: a low-density data track, audio, then
        // the high-density track whose first sector is IP.BIN.
        put("SA (Track 1).bin", &iso_image("", None));
        put("SA (Track 2).bin", &[0u8; 4704]);
        put("SA (Track 3).bin", &sega_sector(b"SEGA SEGAKATANA ", 16));
        put(
            "SA.cue",
            b"FILE \"SA (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\nFILE \"SA (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\nFILE \"SA (Track 3).bin\" BINARY\n  TRACK 03 MODE1/2352\n",
        );
        assert_eq!(probe(&p("SA.cue")), Some(Kind::Dreamcast));
        put("SA.gdi", b"3\r\n1 0 4 2352 \"SA (Track 1).bin\" 0\r\n2 756 0 2352 \"SA (Track 2).bin\" 0\r\n3 45000 4 2352 \"SA (Track 3).bin\" 0\r\n");
        assert_eq!(probe(&p("SA.gdi")), Some(Kind::Dreamcast));
        put("track03.iso", &sega_sector(b"SEGA SEGAKATANA ", 0));
        put("cooked.gdi", b"3\n1 0 4 2048 track01.iso 0\n2 600 0 2352 track02.raw 0\n3 45000 4 2048 track03.iso 0\n");
        assert_eq!(probe(&p("cooked.gdi")), Some(Kind::Dreamcast));

        // Track 3 of something else is not a Dreamcast; a gdi with no track 3.
        put("Other (Track 3).bin", &sega_sector(b"SEGA SEGASATURN ", 16));
        put("Other.cue", b"FILE \"SA (Track 2).bin\" BINARY\n  TRACK 01 AUDIO\nFILE \"Other (Track 3).bin\" BINARY\n  TRACK 03 MODE1/2352\n");
        assert_eq!(probe(&p("Other.cue")), None);
        put("short.gdi", b"1\n1 0 4 2352 \"SA (Track 3).bin\" 0\n");
        assert_eq!(probe(&p("short.gdi")), None);
    }

    /// A CHD v5 with no compressors (an uncompressed map) holding `data`
    /// in `hunk`-byte hunks, and one metadata entry `meta` tagged `tag`.
    pub(crate) fn chd_image(data: &[u8], hunk: u32, unit: u32, meta_tag: &[u8; 4], meta: &[u8]) -> Vec<u8> {
        let hunks = (data.len() as u64).div_ceil(hunk as u64) as usize;
        let map_at = 124u64;
        let meta_at = map_at + 4 * hunks as u64;
        let data_at = (meta_at + 16 + meta.len() as u64).div_ceil(hunk as u64) * hunk as u64;
        let mut f = vec![0u8; 124];
        f[..8].copy_from_slice(b"MComprHD");
        f[8..12].copy_from_slice(&124u32.to_be_bytes());
        f[12..16].copy_from_slice(&5u32.to_be_bytes());
        f[32..40].copy_from_slice(&(data.len() as u64).to_be_bytes());
        f[40..48].copy_from_slice(&map_at.to_be_bytes());
        f[48..56].copy_from_slice(&meta_at.to_be_bytes());
        f[56..60].copy_from_slice(&hunk.to_be_bytes());
        f[60..64].copy_from_slice(&unit.to_be_bytes());
        for i in 0..hunks {
            f.extend_from_slice(&((data_at / hunk as u64) as u32 + i as u32).to_be_bytes());
        }
        f.extend_from_slice(meta_tag);
        f.extend_from_slice(&(meta.len() as u32).to_be_bytes());
        f.extend_from_slice(&0u64.to_be_bytes());
        f.extend_from_slice(meta);
        f.resize(data_at as usize, 0);
        f.extend_from_slice(data);
        f.resize(data_at as usize + hunks * hunk as usize, 0);
        f
    }

    /// Raw 2352-byte sectors as CD frames: 96 subcode bytes after each.
    fn frames(raw: &[u8]) -> Vec<u8> {
        raw.chunks(2352).flat_map(|s| s.iter().copied().chain([0u8; 96])).collect()
    }

    const CHT2: &[u8] = b"TRACK:1 TYPE:MODE2_RAW SUBTYPE:NONE FRAMES:24 PREGAP:0 PGTYPE:MODE1 PGSUB:RW POSTGAP:0\0";

    #[test]
    fn probes_chds() {
        let t = tempfile::tempdir().unwrap();
        let p = |n: &str| t.path().join(n);
        let put = |n: &str, d: &[u8]| {
            fs::write(p(n), d).unwrap();
            probe(&p(n))
        };
        let cd = |raw: &[u8], meta: &[u8]| chd_image(&frames(raw), 2448 * 8, 2448, b"CHT2", meta);
        assert_eq!(put("ff7.chd", &cd(&raw_image(24, "PLAYSTATION", Some(PS1_CNF)), CHT2)), Some(Kind::Ps1));
        assert_eq!(put("ico-cd.chd", &cd(&raw_image(24, "PLAYSTATION", Some(PS2_CNF)), CHT2)), Some(Kind::Ps2));
        let mode1 = String::from_utf8(CHT2.to_vec()).unwrap().replace("MODE2_RAW", "MODE1_RAW");
        assert_eq!(put("nights.chd", &cd(&sega_sector(b"SEGA SEGASATURN ", 16), mode1.as_bytes())), Some(Kind::Saturn));
        // A cooked track keeps 2048 bytes at the start of each frame.
        let cooked: Vec<u8> = iso_image("PLAYSTATION", Some(PS1_CNF)).chunks(2048).flat_map(|s| s.iter().copied().chain([0u8; 400])).collect();
        let cooked_meta = String::from_utf8(CHT2.to_vec()).unwrap().replace("MODE2_RAW", "MODE1");
        assert_eq!(put("cooked.chd", &chd_image(&cooked, 2448 * 8, 2448, b"CHT2", cooked_meta.as_bytes())), Some(Kind::Ps1));
        let dvd = |img: &[u8]| chd_image(img, 4096, 2048, b"DVD ", b"\0");
        assert_eq!(put("ffx.chd", &dvd(&iso_image("PLAYSTATION", Some(PS2_CNF)))), Some(Kind::Ps2));
        assert_eq!(put("lumines.chd", &dvd(&iso_image("PSP GAME", None))), Some(Kind::Psp));
        // GD-ROM metadata alone: no hunk is read (these hunks hold nothing).
        let gd = b"TRACK:1 TYPE:MODE1 SUBTYPE:NONE FRAMES:300 PAD:0 PREGAP:0 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0\0";
        assert_eq!(put("sa.chd", &chd_image(&[0u8; 2448 * 8], 2448 * 8, 2448, b"CHGD", gd)), Some(Kind::Dreamcast));
        // A hard disk image, a disc that says nothing.
        assert_eq!(put("hdd.chd", &chd_image(&[0u8; 4096], 4096, 512, b"GDDD", b"CYLS:1,HEADS:1,SECS:8,BPS:512\0")), None);
        assert_eq!(put("blank.chd", &dvd(&[0u8; 40960])), None);
    }

    #[test]
    fn corrupt_chds_are_unknown() {
        let t = tempfile::tempdir().unwrap();
        let p = |n: &str| t.path().join(n);
        let put = |n: &str, d: &[u8]| {
            fs::write(p(n), d).unwrap();
            probe(&p(n))
        };
        let good = chd_image(&iso_image("PSP GAME", None), 4096, 2048, b"DVD ", b"\0");
        assert_eq!(put("good.chd", &good), Some(Kind::Psp));
        assert_eq!(put("magic.chd", b"MComprHD"), None);
        assert_eq!(put("empty.chd", b""), None);
        assert_eq!(put("header.chd", &good[..100]), None);
        // Cut off in the map, in the metadata, and in the data.
        assert_eq!(put("map.chd", &good[..130]), None);
        assert_eq!(put("meta.chd", &good[..124 + 4 * 10 + 8]), None);
        assert_eq!(put("data.chd", &good[..good.len() - 30000]), None);
        // Garbage everywhere after a valid header.
        let mut junk = good.clone();
        for (i, b) in junk.iter_mut().enumerate().skip(124) {
            *b = (i * 7919 % 251) as u8;
        }
        assert_eq!(put("junk.chd", &junk), None);
        // A compressed map with a garbage header and table.
        let mut comp = good.clone();
        comp[16..20].copy_from_slice(b"zlib");
        assert_eq!(put("comp.chd", &comp), None);
        // Sizes past the limits are refused before the crate sees them.
        let mut big = good.clone();
        big[32..40].copy_from_slice(&(1u64 << 40).to_be_bytes());
        assert_eq!(put("hunks.chd", &big), None);
        let mut huge = good.clone();
        huge[56..60].copy_from_slice(&(64u32 << 20).to_be_bytes());
        assert_eq!(put("hunk.chd", &huge), None);
        let mut map = good.clone();
        map[16..20].copy_from_slice(b"zlib");
        let at = be64(&map, 40) as usize;
        map[at..at + 4].copy_from_slice(&(MAP_LIMIT as u32).to_be_bytes());
        assert_eq!(put("bigmap.chd", &map), None);
        let mut v9 = good;
        v9[12..16].copy_from_slice(&9u32.to_be_bytes());
        assert_eq!(put("v9.chd", &v9), None);
    }

    #[test]
    fn probes_zips_stored_and_deflated() {
        let t = tempfile::tempdir().unwrap();
        let p = |n: &str| t.path().join(n);
        use zip::CompressionMethod::{Deflated, Stored};
        zip_one(&p("melee.zip"), "Melee (USA).iso", &gc_image(), Stored);
        zip_one(&p("melee-d.zip"), "Melee (USA).gcm", &gc_image(), Deflated);
        zip_one(&p("ffx.zip"), "FFX.iso", &iso_image("PLAYSTATION", Some(PS2_CNF)), Stored);
        zip_one(&p("ff7.zip"), "FF7.bin", &raw_image(24, "PLAYSTATION", Some(PS1_CNF)), Deflated);
        zip_one(&p("gba.zip"), "Golden Sun.gba", &gc_image(), Deflated);
        fs::write(p("broken.zip"), b"PK not really").unwrap();
        assert_eq!(probe(&p("melee.zip")), Some(Kind::GameCube));
        assert_eq!(probe(&p("melee-d.zip")), Some(Kind::GameCube));
        assert_eq!(probe(&p("ffx.zip")), Some(Kind::Ps2));
        assert_eq!(probe(&p("ff7.zip")), Some(Kind::Ps1));
        assert_eq!(probe(&p("gba.zip")), None, "no disc-like entry");
        assert_eq!(probe(&p("broken.zip")), None);
    }

    #[test]
    fn deflated_entries_are_inflated_only_so_far() {
        let mut s = Stream { r: io::repeat(7), got: Vec::new() };
        let mut buf = [0u8; 16];
        assert!(s.at(1000, &mut buf));
        assert_eq!(s.got.len(), 1016);
        assert!(s.at(10, &mut buf), "kept");
        assert!(!s.at(INFLATE_LIMIT - 8, &mut buf), "never past the limit");
        assert!(s.at(INFLATE_LIMIT - 16, &mut buf));
        let mut short = Stream { r: &b"tiny"[..], got: Vec::new() };
        assert!(!short.at(0, &mut buf));
    }

    #[test]
    fn kinds_are_cached_by_size_and_mtime() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("disc.iso");
        fs::write(&p, gc_image()).unwrap();
        let (got, late) = kinds(vec![p.clone(), t.path().join("none.iso")]);
        assert!(late.is_none(), "all answered in time");
        assert_eq!(got[&p], Some(Kind::GameCube));
        assert_eq!(got[&t.path().join("none.iso")], None);
        // Same size and mtime: the cached answer, even though the bytes changed.
        let mtime = fs::metadata(&p).unwrap().modified().unwrap();
        let mut ps1 = gc_image();
        ps1[0x1C] = 0;
        fs::write(&p, &ps1).unwrap();
        fs::File::options().write(true).open(&p).unwrap().set_modified(mtime).unwrap();
        assert_eq!(kinds(vec![p.clone()]).0[&p], Some(Kind::GameCube));
        // A different size: probed again.
        fs::write(&p, iso_image("PLAYSTATION", Some(PS1_CNF))).unwrap();
        assert_eq!(kinds(vec![p.clone()]).0[&p], Some(Kind::Ps1));
    }

    #[test]
    fn a_scan_that_stops_waiting_gets_a_late() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("late.iso");
        fs::write(&p, gc_image()).unwrap();
        TEST_WAIT.with(|w| w.set(Some(Duration::ZERO)));
        let (got, late) = kinds(vec![p.clone()]);
        TEST_WAIT.with(|w| w.set(None));
        assert!(got.is_empty());
        let late = late.expect("a late for the unanswered path");
        let start = Instant::now();
        while !late.drained() {
            assert!(start.elapsed() < Duration::from_secs(10), "never drained");
            thread::sleep(Duration::from_millis(5));
        }
        // The thread filled the cache: the next scan has the answer at once.
        let (got, late) = kinds(vec![p.clone()]);
        assert_eq!(got[&p], Some(Kind::GameCube));
        assert!(late.is_none());
    }
}
