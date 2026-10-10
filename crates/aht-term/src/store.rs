//! Bounded store of full command outputs, paged by the agent via `read_output`.

use aht_core::terminal::OutputChunk;
use aht_core::{Error, OutputId, Result};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Default total capacity: 16 MiB.
pub const DEFAULT_CAPACITY: usize = 16 * 1024 * 1024;

struct Entry {
    text: String,
    chars: usize,
    complete: bool,
}

struct Inner {
    map: HashMap<OutputId, Entry>,
    /// LRU order, least recently used first.
    order: VecDeque<OutputId>,
    total: usize,
    cap: usize,
}

/// LRU-bounded output store. Thread-safe; share via `Arc` across sessions if desired.
pub struct OutputStore {
    inner: Mutex<Inner>,
}

impl Default for OutputStore {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl OutputStore {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                order: VecDeque::new(),
                total: 0,
                cap,
            }),
        }
    }

    pub fn create(&self) -> OutputId {
        let id = OutputId::new();
        let mut g = self.inner.lock().unwrap();
        g.map.insert(
            id,
            Entry {
                text: String::new(),
                chars: 0,
                complete: false,
            },
        );
        g.order.push_back(id);
        id
    }

    fn touch(g: &mut Inner, id: OutputId) {
        if let Some(p) = g.order.iter().position(|x| *x == id) {
            g.order.remove(p);
            g.order.push_back(id);
        }
    }

    pub fn append(&self, id: OutputId, s: &str) {
        let mut g = self.inner.lock().unwrap();
        if !g.map.contains_key(&id) {
            return;
        }
        Self::touch(&mut g, id);
        // Evict other entries (oldest first) until it fits.
        while g.total + s.len() > g.cap {
            let Some(victim) = g.order.iter().copied().find(|x| *x != id) else {
                return; // This entry alone exceeds the cap: drop further output.
            };
            g.order.retain(|x| *x != victim);
            if let Some(e) = g.map.remove(&victim) {
                g.total -= e.text.len();
            }
        }
        g.total += s.len();
        let e = g.map.get_mut(&id).expect("checked");
        e.text.push_str(s);
        e.chars += s.chars().count();
    }

    pub fn complete(&self, id: OutputId) {
        if let Some(e) = self.inner.lock().unwrap().map.get_mut(&id) {
            e.complete = true;
        }
    }

    /// Full text and completion flag.
    pub fn get(&self, id: OutputId) -> Option<(String, bool)> {
        let mut g = self.inner.lock().unwrap();
        Self::touch(&mut g, id);
        g.map.get(&id).map(|e| (e.text.clone(), e.complete))
    }

    /// Page by character offset/limit.
    pub fn read(&self, id: OutputId, offset: usize, limit: usize) -> Result<OutputChunk> {
        let mut g = self.inner.lock().unwrap();
        Self::touch(&mut g, id);
        let e = g
            .map
            .get(&id)
            .ok_or_else(|| Error::NotFound(format!("output {id}")))?;
        let data: String = e.text.chars().skip(offset).take(limit).collect();
        let end = offset.min(e.chars) + data.chars().count();
        Ok(OutputChunk {
            output_id: id,
            offset,
            data,
            total_len: e.chars,
            complete: e.complete && end == e.chars,
        })
    }
}

/// Head + tail excerpt when `text` exceeds `head + tail` bytes. Returns (excerpt, truncated).
pub fn excerpt(text: &str, head: usize, tail: usize) -> (String, bool) {
    if text.len() <= head + tail {
        return (text.to_string(), false);
    }
    let mut h = head;
    while !text.is_char_boundary(h) {
        h -= 1;
    }
    let mut t = text.len() - tail;
    while !text.is_char_boundary(t) {
        t += 1;
    }
    let omitted = text[h..t].chars().count();
    (
        format!(
            "{}\n[... {omitted} characters omitted; use read_output for the full output ...]\n{}",
            &text[..h],
            &text[t..]
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging_and_complete() {
        let s = OutputStore::default();
        let id = s.create();
        s.append(id, "héllo world");
        let c = s.read(id, 0, 5).unwrap();
        assert_eq!(
            (c.data.as_str(), c.total_len, c.complete),
            ("héllo", 11, false)
        );
        s.complete(id);
        let c = s.read(id, 6, 100).unwrap();
        assert_eq!((c.data.as_str(), c.complete), ("world", true));
        assert!(s.read(OutputId::new(), 0, 1).is_err());
    }

    #[test]
    fn lru_eviction() {
        let s = OutputStore::new(10);
        let a = s.create();
        s.append(a, "aaaaaa");
        let b = s.create();
        s.append(b, "bbbbbb");
        assert!(s.get(a).is_none());
        assert_eq!(s.get(b).unwrap().0, "bbbbbb");
        s.append(b, "bbbbbbbbbb"); // would exceed alone: dropped
        assert_eq!(s.get(b).unwrap().0, "bbbbbb");
    }

    #[test]
    fn excerpt_head_tail() {
        let t = "x".repeat(10_000);
        let (e, tr) = excerpt(&t, 4096, 4096);
        assert!(tr);
        assert!(e.starts_with(&"x".repeat(4096)));
        assert!(e.contains("1808 characters omitted"));
        assert_eq!(excerpt("short", 4, 4), ("short".into(), false));
    }
}
