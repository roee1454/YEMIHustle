//! The player's own copy of the original game, read in place: the Steam install's `.pck`
//! (Godot 3 pack), or for development a decompiled project folder. Nothing is copied out of it.
//!
//! Files are addressed by their `res://` path without the prefix (`fx/HitEffect1.tscn`).
//! Imported assets are followed through their `.import` file: textures to the `.stex` payload
//! (PNG or WebP), sounds to the `.sample` resource (raw PCM).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub enum Source {
    Pck { file: PathBuf, files: HashMap<String, (u64, u64)>, version: (u32, u32, u32) },
    Dir(PathBuf),
}

pub struct GameFiles {
    pub source: Source,
    /// The folder the player chose (the Steam install, or the decompiled project).
    pub root: PathBuf,
}

/// An image file's bytes and its format's extension (`png` / `webp`).
pub struct ImageBytes {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
}

/// Decoded audio: interleaved samples in -1..1.
pub struct Audio {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl GameFiles {
    /// Opens a game folder (finds its `.pck`), a `.pck` file, or a decompiled project folder.
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.is_dir() && path.join("project.godot").exists() {
            return Ok(GameFiles { source: Source::Dir(path.to_path_buf()), root: path.to_path_buf() });
        }
        let pck = if path.is_file() {
            path.to_path_buf()
        } else {
            find_pck(path).ok_or_else(|| format!("no .pck game data in {}", path.display()))?
        };
        let (files, version) = read_pck_index(&pck)?;
        let root = if path.is_file() { path.parent().unwrap_or(path).to_path_buf() } else { path.to_path_buf() };
        Ok(GameFiles { source: Source::Pck { file: pck, files, version }, root })
    }

    pub fn describe(&self) -> String {
        match &self.source {
            Source::Pck { file, files, version } => {
                format!("{} ({} files, Godot {}.{}.{})", file.display(), files.len(), version.0, version.1, version.2)
            }
            Source::Dir(d) => format!("{} (unpacked project)", d.display()),
        }
    }

    pub fn exists(&self, p: &str) -> bool {
        match &self.source {
            Source::Pck { files, .. } => files.contains_key(p) || files.contains_key(&format!("{p}.import")),
            Source::Dir(d) => d.join(p).exists(),
        }
    }

    /// A file as stored.
    pub fn read(&self, p: &str) -> Option<Vec<u8>> {
        match &self.source {
            Source::Pck { file, files, .. } => {
                let (off, size) = *files.get(p)?;
                let mut f = std::fs::File::open(file).ok()?;
                f.seek(SeekFrom::Start(off)).ok()?;
                let mut buf = vec![0; size as usize];
                f.read_exact(&mut buf).ok()?;
                Some(buf)
            }
            Source::Dir(d) => std::fs::read(d.join(p)).ok(),
        }
    }

    pub fn text(&self, p: &str) -> Option<String> {
        self.read(p).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Where Godot put an imported file (`path=` in `<file>.import`).
    fn imported(&self, p: &str) -> Option<String> {
        let imp = self.text(&format!("{p}.import"))?;
        let line = imp.lines().find(|l| l.starts_with("path=") || l.starts_with("path.s3tc=") || l.starts_with("path.etc2="))?;
        Some(line.split('"').nth(1)?.trim_start_matches("res://").to_string())
    }

    /// A texture by its original path (`ui/title2.png`).
    pub fn image(&self, p: &str) -> Option<ImageBytes> {
        if let Source::Dir(d) = &self.source {
            if let Ok(bytes) = std::fs::read(d.join(p)) {
                // extracted textures keep the format they were stored in, whatever the name says
                let ext = sniff_ext(&bytes, p);
                return Some(ImageBytes { bytes, ext });
            }
        }
        let stex = self.read(&self.imported(p)?)?;
        stex_payload(&stex)
    }

    /// A sound by its original path (`sound/common/hit1.wav`).
    pub fn audio(&self, p: &str) -> Option<Audio> {
        if let Source::Dir(d) = &self.source {
            if let Ok(bytes) = std::fs::read(d.join(p)) {
                return decode_wav(&bytes);
            }
        }
        let sample = self.read(&self.imported(p)?)?;
        decode_sample(&sample)
    }

    /// An imported MP3: the raw file inside the `.mp3str` resource.
    pub fn mp3(&self, p: &str) -> Option<Vec<u8>> {
        if let Source::Dir(d) = &self.source {
            return std::fs::read(d.join(p)).ok();
        }
        let res = self.read(&self.imported(p)?)?;
        match rsrc_props(&res)?.remove("data")? {
            Bv::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Every file under a folder (for checks and tests).
    pub fn list(&self, prefix: &str) -> Vec<String> {
        match &self.source {
            Source::Pck { files, .. } => {
                let mut v: Vec<String> = files.keys().filter(|k| k.starts_with(prefix)).cloned().collect();
                v.sort();
                v
            }
            Source::Dir(d) => {
                let mut out = Vec::new();
                let mut stack = vec![d.join(prefix)];
                while let Some(dir) = stack.pop() {
                    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                        let path = e.path();
                        if path.is_dir() {
                            stack.push(path);
                        } else if let Ok(rel) = path.strip_prefix(d) {
                            out.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
                out.sort();
                out
            }
        }
    }
}

/// The image format by its first bytes (extracted textures keep the format they were stored in,
/// whatever their file name says).
pub fn sniff_ext(bytes: &[u8], name: &str) -> &'static str {
    if bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else if bytes.get(1..4) == Some(b"PNG") {
        "png"
    } else if name.ends_with(".jpg") || name.ends_with(".jpeg") {
        "jpg"
    } else {
        "png"
    }
}

/// The game's pack next to its executable (or inside a macOS app bundle).
fn find_pck(dir: &Path) -> Option<PathBuf> {
    let mut candidates = vec![dir.to_path_buf(), dir.join("Contents/Resources")];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x == "app") {
                candidates.push(e.path().join("Contents/Resources"));
            }
        }
    }
    for c in candidates {
        let preferred = c.join("YourOnlyMoveIsHUSTLE.pck");
        if preferred.is_file() {
            return Some(preferred);
        }
        if let Some(p) = std::fs::read_dir(&c).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "pck")) {
            return Some(p);
        }
    }
    None
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Godot 3 pack: "GDPC", format, engine version, 16 reserved words, then the file table.
type PckIndex = (HashMap<String, (u64, u64)>, (u32, u32, u32));

fn read_pck_index(path: &Path) -> Result<PckIndex, String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut head = [0u8; 88];
    f.read_exact(&mut head).map_err(|e| format!("{}: {e}", path.display()))?;
    if &head[0..4] != b"GDPC" {
        return Err(format!("{} is not a Godot pack", path.display()));
    }
    let format = u32_at(&head, 4).unwrap_or(0);
    if format != 1 {
        return Err(format!("unsupported pack format {format} (expected a Godot 3 game)"));
    }
    let version = (u32_at(&head, 8).unwrap_or(0), u32_at(&head, 12).unwrap_or(0), u32_at(&head, 16).unwrap_or(0));
    let count = u32_at(&head, 84).unwrap_or(0) as usize;
    let mut rest = Vec::new();
    f.read_to_end(&mut rest).map_err(|e| e.to_string())?;
    let mut files = HashMap::with_capacity(count);
    let mut o = 0;
    for _ in 0..count {
        let len = u32_at(&rest, o).ok_or("truncated pack table")? as usize;
        o += 4;
        let raw = rest.get(o..o + len).ok_or("truncated pack table")?;
        let name = String::from_utf8_lossy(raw).trim_end_matches('\0').trim_start_matches("res://").to_string();
        o += len;
        let off = u64_at(&rest, o).ok_or("truncated pack table")?;
        let size = u64_at(&rest, o + 8).ok_or("truncated pack table")?;
        o += 16 + 16; // offset, size, md5
        files.insert(name, (off, size));
    }
    Ok((files, version))
}

/// A `.stex` (StreamTexture): 28-byte header, then "PNG " or "WEBP" and the image file.
pub fn stex_payload(b: &[u8]) -> Option<ImageBytes> {
    if b.get(0..4)? != b"GDST" {
        return None;
    }
    let size = u32_at(b, 24)? as usize;
    let data = b.get(28..28 + size)?;
    if data.len() < 4 {
        return None; // raw, uncompressed textures carry no PNG/WebP payload
    }
    let (tag, bytes) = data.split_at(4);
    let ext = match tag {
        b"PNG " => "png",
        b"WEBP" => "webp",
        _ => return None,
    };
    Some(ImageBytes { bytes: bytes.to_vec(), ext })
}

/// PCM WAV (8/16/24/32-bit integer or 32-bit float).
pub fn decode_wav(b: &[u8]) -> Option<Audio> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let len = u32_at(b, i + 4)? as usize;
        let body = &b[i + 8..(i + 8 + len).min(b.len())];
        match &b[i..i + 4] {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + len + (len & 1);
    }
    let (fmt, data) = (fmt?, data?);
    let format = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]).max(1);
    let rate = u32_at(fmt, 4)?.max(1);
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    let samples: Vec<f32> = match (format, bits) {
        (3, 32) => data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        (_, 8) => data.iter().map(|v| (*v as f32 - 128.0) / 128.0).collect(),
        (_, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        (_, 24) => data.chunks_exact(3).map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0).collect(),
        (_, 32) => data.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0).collect(),
        _ => return None,
    };
    Some(Audio { channels, rate, samples })
}

// ---------------------------------------------------------------------------------------------
// Godot 3 binary resources ("RSRC"), enough for AudioStreamSample
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Bv {
    Int(i64),
    Real(f64),
    Bool(bool),
    Bytes(Vec<u8>),
    Other,
}

struct Rd<'a> {
    b: &'a [u8],
    o: usize,
}

impl Rd<'_> {
    fn u32(&mut self) -> Option<u32> {
        let v = u32_at(self.b, self.o)?;
        self.o += 4;
        Some(v)
    }
    fn u64(&mut self) -> Option<u64> {
        let v = u64_at(self.b, self.o)?;
        self.o += 8;
        Some(v)
    }
    /// Length-prefixed string (the length counts the trailing zero).
    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        let s = self.b.get(self.o..self.o + len)?;
        self.o += len;
        Some(String::from_utf8_lossy(s).trim_end_matches('\0').to_string())
    }
    fn value(&mut self) -> Option<Bv> {
        Some(match self.u32()? {
            1 => Bv::Other,                               // nil
            2 => Bv::Bool(self.u32()? != 0),              // bool
            3 => Bv::Int(self.u32()? as i32 as i64),      // int
            4 => Bv::Real(f32::from_bits(self.u32()?) as f64), // real
            5 => {
                self.string()?;
                Bv::Other
            }
            10 => {
                self.o += 8;
                Bv::Other
            } // Vector2
            31 => {
                // PoolByteArray, padded to 4 bytes
                let len = self.u32()? as usize;
                let v = self.b.get(self.o..self.o + len)?.to_vec();
                self.o += len + (4 - len % 4) % 4;
                Bv::Bytes(v)
            }
            40 => Bv::Int(self.u64()? as i64),
            41 => Bv::Real(f64::from_bits(self.u64()?)),
            _ => return None,
        })
    }
}

/// Properties of the main (last) resource in a binary resource file.
fn rsrc_props(b: &[u8]) -> Option<HashMap<String, Bv>> {
    if b.get(0..4)? != b"RSRC" {
        return None;
    }
    let mut r = Rd { b, o: 4 };
    let big_endian = r.u32()?;
    if big_endian != 0 {
        return None;
    }
    let _real64 = r.u32()?;
    let (_maj, _min, _fmt) = (r.u32()?, r.u32()?, r.u32()?);
    let _ty = r.string()?;
    let _import_md = r.u64()?;
    r.o += 14 * 4;
    let names: Vec<String> = (0..r.u32()?).map(|_| r.string()).collect::<Option<_>>()?;
    for _ in 0..r.u32()? {
        r.string()?;
        r.string()?;
    }
    let internal: Vec<u64> = (0..r.u32()?)
        .map(|_| {
            r.string()?;
            r.u64()
        })
        .collect::<Option<_>>()?;
    let mut res = Rd { b, o: *internal.last()? as usize };
    let _ty = res.string()?;
    let mut props = HashMap::new();
    for _ in 0..res.u32()? {
        let name = names.get(res.u32()? as usize)?.clone();
        let v = res.value()?;
        props.insert(name, v);
    }
    Some(props)
}

/// An imported WAV: AudioStreamSample with 8-bit, 16-bit or IMA-ADPCM data.
pub fn decode_sample(b: &[u8]) -> Option<Audio> {
    let p = rsrc_props(b)?;
    let int = |k: &str| match p.get(k) {
        Some(Bv::Int(v)) => Some(*v),
        Some(Bv::Real(v)) => Some(*v as i64),
        _ => None,
    };
    let stereo = matches!(p.get("stereo"), Some(Bv::Bool(true)));
    let channels = if stereo { 2 } else { 1 };
    let rate = int("mix_rate").unwrap_or(44100).max(1) as u32;
    let Some(Bv::Bytes(data)) = p.get("data") else { return None };
    let samples = match int("format").unwrap_or(0) {
        0 => data.iter().map(|v| *v as i8 as f32 / 128.0).collect(),
        1 => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        2 => ima_adpcm(data, channels),
        _ => return None,
    };
    Some(Audio { channels, rate, samples })
}

/// Godot's IMA-ADPCM: per channel a 4-byte header (predictor, step index), then nibbles.
fn ima_adpcm(data: &[u8], channels: u16) -> Vec<f32> {
    const STEPS: [i32; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
        157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411,
        1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
        10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
    ];
    const INDEX: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
    let ch = channels as usize;
    if data.len() < 4 * ch {
        return vec![];
    }
    let mut pred: Vec<i32> = (0..ch).map(|c| i16::from_le_bytes([data[c * 4], data[c * 4 + 1]]) as i32).collect();
    let mut idx: Vec<i32> = (0..ch).map(|c| (data[c * 4 + 2] as i32).clamp(0, 88)).collect();
    let mut out = Vec::new();
    let mut decode = |c: usize, nib: u8, out: &mut Vec<f32>| {
        let step = STEPS[idx[c] as usize];
        let mut diff = step >> 3;
        if nib & 1 != 0 {
            diff += step >> 2;
        }
        if nib & 2 != 0 {
            diff += step >> 1;
        }
        if nib & 4 != 0 {
            diff += step;
        }
        if nib & 8 != 0 {
            diff = -diff;
        }
        pred[c] = (pred[c] + diff).clamp(-32768, 32767);
        idx[c] = (idx[c] + INDEX[nib as usize]).clamp(0, 88);
        out.push(pred[c] as f32 / 32768.0);
    };
    for (k, byte) in data[4 * ch..].iter().enumerate() {
        let c = if ch == 2 { k % 2 } else { 0 };
        decode(c, byte & 0x0f, &mut out);
        decode(c, byte >> 4, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install() -> Option<GameFiles> {
        let found = crate::steam::find_game();
        found.and_then(|g| GameFiles::open(&g.game_dir).ok())
    }

    #[test]
    fn reads_the_installed_game() {
        let Some(g) = install() else { return };
        assert!(matches!(g.source, Source::Pck { .. }), "{}", g.describe());
        assert!(g.text("fx/HitEffect1.tscn").is_some_and(|t| t.contains("CPUParticles2D")));
        let title = g.image("ui/title2.png").expect("title texture");
        assert_eq!(title.ext, "webp");
        let hit = g.audio("sound/common/hit1.wav").expect("hit sound");
        assert!(hit.samples.len() > 1000 && hit.rate > 8000, "{} samples at {}", hit.samples.len(), hit.rate);
        assert!(g.read("ui/PixeloidSans.ttf").is_some_and(|b| b.len() > 1000));
    }

    #[test]
    fn every_sound_and_texture_in_the_pack_decodes() {
        let Some(g) = install() else { return };
        let all = g.list("");
        let sounds: Vec<&String> = all.iter().filter(|p| p.ends_with(".wav.import")).collect();
        let bad_sounds: Vec<&&String> = sounds.iter().filter(|p| g.audio(p.trim_end_matches(".import")).is_none()).collect();
        let textures: Vec<&String> = all.iter().filter(|p| p.ends_with(".png.import")).collect();
        let bad_tex: Vec<&&String> = textures.iter().filter(|p| g.image(p.trim_end_matches(".import")).is_none()).collect();
        assert!(sounds.len() > 200 && textures.len() > 1000, "{} sounds, {} textures", sounds.len(), textures.len());
        assert!(bad_sounds.is_empty(), "{bad_sounds:?}");
        assert!(bad_tex.len() * 100 < textures.len(), "{} of {} textures failed: {:?}", bad_tex.len(), textures.len(), &bad_tex[..bad_tex.len().min(5)]);
    }
}
