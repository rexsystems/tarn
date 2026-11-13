//! On-disk LRU cache of fully downloaded audio streams.
//!
//! Tracks are stored under the app data dir (`tarn/audio-cache/`) and evicted
//! by least-recent access when the total size exceeds a configurable cap
//! (default 1 GiB). Partial files are kept so a dropped download can resume.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

const INDEX: &str = "index.json";
pub const DEFAULT_MAX_MB: u64 = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    file: String,
    size: u64,
    accessed: u64,
    complete: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Index {
    items: HashMap<String, Entry>,
}

pub struct AudioCache {
    dir: PathBuf,
    max_bytes: AtomicU64,
    index: Mutex<Index>,
}

impl AudioCache {
    pub fn open() -> Self {
        let dir = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tarn")
            .join("audio-cache");
        let _ = fs::create_dir_all(&dir);
        let index = load_index(&dir);
        Self {
            dir,
            max_bytes: AtomicU64::new(DEFAULT_MAX_MB * 1024 * 1024),
            index: Mutex::new(index),
        }
    }

    pub fn set_max_mb(&self, mb: u64) {
        let bytes = mb.saturating_mul(1024 * 1024);
        self.max_bytes.store(bytes, Ordering::Relaxed);
        if bytes == 0 {
            self.clear();
        } else {
            self.evict(0);
        }
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes.load(Ordering::Relaxed)
    }

    pub fn used_bytes(&self) -> u64 {
        self.index
            .lock()
            .unwrap()
            .items
            .values()
            .map(|e| e.size)
            .sum()
    }

    pub fn has_complete(&self, id: &str) -> bool {
        let g = self.index.lock().unwrap();
        g.items.get(id).is_some_and(|e| e.complete && e.size > 64)
    }

    /// Read a complete cached body, touching LRU.
    pub fn read_complete(&self, id: &str) -> Option<Vec<u8>> {
        let path = {
            let mut g = self.index.lock().unwrap();
            let e = g.items.get_mut(id)?;
            if !e.complete || e.size < 64 {
                return None;
            }
            e.accessed = now();
            let p = self.dir.join(&e.file);
            drop(g);
            let _ = self.persist();
            p
        };
        let bytes = fs::read(&path).ok()?;
        if bytes.len() < 64 {
            return None;
        }
        Some(bytes)
    }

    /// Path and current length of an incomplete part file, for Range resumes.
    pub fn partial(&self, id: &str) -> Option<(PathBuf, u64)> {
        let part = self.part_path(id);
        let meta = fs::metadata(&part).ok()?;
        if !meta.is_file() || meta.len() == 0 {
            return None;
        }
        Some((part, meta.len()))
    }

    pub fn part_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.part", safe(id)))
    }

    pub fn final_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.m4a", safe(id)))
    }

    /// Append a chunk to the part file. Creates it if needed.
    pub fn append_part(&self, id: &str, chunk: &[u8]) -> Result<()> {
        if chunk.is_empty() || self.max_bytes() == 0 {
            return Ok(());
        }
        let path = self.part_path(id);
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open cache part {}", path.display()))?;
        f.write_all(chunk)?;
        Ok(())
    }

    /// Seed a part file from already-in-memory bytes (overwrite).
    pub fn write_part(&self, id: &str, bytes: &[u8]) -> Result<()> {
        if self.max_bytes() == 0 {
            return Ok(());
        }
        fs::write(self.part_path(id), bytes)?;
        Ok(())
    }

    /// Mark the download complete: move `.part` (or the given bytes) into the
    /// final file and record it in the index. Evicts LRU entries if needed.
    pub fn commit(&self, id: &str, bytes: &[u8]) -> Result<()> {
        if self.max_bytes() == 0 {
            let _ = fs::remove_file(self.part_path(id));
            return Ok(());
        }
        if bytes.len() < 64 {
            return Err(anyhow!("cache commit too small"));
        }
        self.evict(bytes.len() as u64);
        let dest = self.final_path(id);
        let part = self.part_path(id);
        if part.exists() {
            // Prefer the on-disk part if it matches; otherwise write bytes.
            let on_disk = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if on_disk == bytes.len() as u64 {
                let _ = fs::rename(&part, &dest);
            } else {
                fs::write(&dest, bytes)?;
                let _ = fs::remove_file(&part);
            }
        } else {
            fs::write(&dest, bytes)?;
        }
        // If rename landed on a cross-device fail, dest may be missing.
        if !dest.exists() {
            fs::write(&dest, bytes)?;
        }
        let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(bytes.len() as u64);
        {
            let mut g = self.index.lock().unwrap();
            g.items.insert(
                id.to_string(),
                Entry {
                    file: dest
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    size,
                    accessed: now(),
                    complete: true,
                },
            );
        }
        self.persist()?;
        Ok(())
    }

    pub fn evict(&self, extra: u64) {
        let max = self.max_bytes();
        if max == 0 {
            return;
        }
        loop {
            let (used, victim) = {
                let g = self.index.lock().unwrap();
                let used: u64 = g.items.values().map(|e| e.size).sum();
                if used.saturating_add(extra) <= max {
                    return;
                }
                let v = g
                    .items
                    .iter()
                    .min_by_key(|(_, e)| e.accessed)
                    .map(|(k, e)| (k.clone(), e.file.clone()));
                (used, v)
            };
            let Some((id, file)) = victim else {
                let _ = used;
                return;
            };
            let _ = fs::remove_file(self.dir.join(&file));
            let _ = fs::remove_file(self.part_path(&id));
            self.index.lock().unwrap().items.remove(&id);
            let _ = self.persist();
        }
    }

    pub fn clear(&self) {
        if let Ok(rd) = fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    let _ = fs::remove_file(p);
                }
            }
        }
        *self.index.lock().unwrap() = Index::default();
        let _ = self.persist();
    }

    fn persist(&self) -> Result<()> {
        let g = self.index.lock().unwrap();
        let tmp = self.dir.join(".index.json.tmp");
        let data = serde_json::to_vec_pretty(&*g)?;
        fs::write(&tmp, data)?;
        fs::rename(&tmp, self.dir.join(INDEX))?;
        Ok(())
    }
}

fn load_index(dir: &Path) -> Index {
    let p = dir.join(INDEX);
    let Ok(s) = fs::read_to_string(p) else {
        return Index::default();
    };
    let mut idx: Index = serde_json::from_str(&s).unwrap_or_default();
    // Drop entries whose files vanished.
    idx.items.retain(|_, e| dir.join(&e.file).is_file());
    idx
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
