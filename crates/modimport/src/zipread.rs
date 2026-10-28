//! A small zip reader: stored and deflated entries, enough for Workshop archives.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use flate2::read::DeflateDecoder;

#[derive(Clone, Debug)]
struct Entry {
    method: u16,
    comp_size: u64,
    size: u64,
    offset: u64,
}

pub struct Zip {
    data: Vec<u8>,
    entries: BTreeMap<String, Entry>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

impl Zip {
    pub fn open(path: &Path) -> Result<Zip, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Zip::from_bytes(data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Zip, String> {
        // the end-of-central-directory record is within the last 64 KiB
        let start = data.len().saturating_sub(65_557);
        let eocd = (start..data.len().saturating_sub(21))
            .rev()
            .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
            .ok_or("not a zip file")?;
        let count = u16_at(&data, eocd + 10).ok_or("truncated")? as usize;
        let mut at = u32_at(&data, eocd + 16).ok_or("truncated")? as usize;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            if data.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
                return Err("damaged central directory".into());
            }
            let method = u16_at(&data, at + 10).ok_or("truncated")?;
            let comp_size = u32_at(&data, at + 20).ok_or("truncated")? as u64;
            let size = u32_at(&data, at + 24).ok_or("truncated")? as u64;
            let name_len = u16_at(&data, at + 28).ok_or("truncated")? as usize;
            let extra_len = u16_at(&data, at + 30).ok_or("truncated")? as usize;
            let comment_len = u16_at(&data, at + 32).ok_or("truncated")? as usize;
            let offset = u32_at(&data, at + 42).ok_or("truncated")? as u64;
            let name = String::from_utf8_lossy(data.get(at + 46..at + 46 + name_len).ok_or("truncated")?).into_owned();
            entries.insert(name, Entry { method, comp_size, size, offset });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip { data, entries })
    }

    /// Every entry name (directories end in `/`), sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn size(&self, name: &str) -> Option<u64> {
        self.entries.get(name).map(|e| e.size)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let e = self.entries.get(name).ok_or_else(|| format!("{name}: not in the archive"))?;
        let at = e.offset as usize;
        if self.data.get(at..at + 4) != Some(&[0x50, 0x4b, 0x03, 0x04]) {
            return Err(format!("{name}: damaged local header"));
        }
        let n = u16_at(&self.data, at + 26).ok_or("truncated")? as usize;
        let x = u16_at(&self.data, at + 28).ok_or("truncated")? as usize;
        let start = at + 30 + n + x;
        let raw = self.data.get(start..start + e.comp_size as usize).ok_or_else(|| format!("{name}: truncated"))?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(e.size as usize);
                DeflateDecoder::new(raw).read_to_end(&mut out).map_err(|err| format!("{name}: {err}"))?;
                Ok(out)
            }
            m => Err(format!("{name}: unsupported compression {m}")),
        }
    }

    pub fn read_text(&self, name: &str) -> Result<String, String> {
        self.read(name).map(|b| String::from_utf8_lossy(&b).into_owned())
    }
}
