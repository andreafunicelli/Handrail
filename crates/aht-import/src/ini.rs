//! Minimal INI parser preserving section and key order.

pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

pub fn parse(text: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            out.push(Section {
                name: line[1..line.len() - 1].to_string(),
                entries: Vec::new(),
            });
        } else if let Some((k, v)) = line.split_once('=') {
            if let Some(s) = out.last_mut() {
                s.entries.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}
