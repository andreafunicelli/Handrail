//! Just enough XML for S3 responses: element text by tag name. S3 writes these elements without
//! attributes, CDATA or comments, so a scan is enough and keeps a parser out of the build.

/// The raw contents of every `<tag>…</tag>`, in document order.
pub(crate) fn all<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let body = &rest[start + open.len()..];
        let Some(end) = body.find(&close) else { break };
        out.push(&body[..end]);
        rest = &body[end + close.len()..];
    }
    out
}

/// The unescaped text of the first `<tag>…</tag>`.
pub(crate) fn first(xml: &str, tag: &str) -> Option<String> {
    all(xml, tag).first().copied().map(unescape)
}

/// Resolve the predefined entities and character references.
pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let decoded = tail.find(';').and_then(|end| {
            let c = match &tail[1..end] {
                "lt" => '<',
                "gt" => '>',
                "amp" => '&',
                "quot" => '"',
                "apos" => '\'',
                e => {
                    let n = match e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => e.strip_prefix('#')?.parse().ok()?,
                    };
                    char::from_u32(n)?
                }
            };
            Some((c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_unescapes() {
        let x = "<R><Prefix>p/</Prefix><Contents><Key>a &amp; b&#x2F;c&#33;</Key></Contents>\
                 <Contents><Key>z</Key></Contents><CommonPrefixes><Prefix>p/d/</Prefix></CommonPrefixes></R>";
        assert_eq!(all(x, "Contents").len(), 2);
        assert_eq!(
            first(all(x, "Contents")[0], "Key").as_deref(),
            Some("a & b/c!")
        );
        assert_eq!(first(x, "Prefix").as_deref(), Some("p/"));
        assert_eq!(
            first(all(x, "CommonPrefixes")[0], "Prefix").as_deref(),
            Some("p/d/")
        );
        assert_eq!(first(x, "Missing"), None);
        assert_eq!(unescape("AT&T &bogus; &"), "AT&T &bogus; &");
    }
}
