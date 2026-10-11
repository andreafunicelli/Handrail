//! AWS Signature Version 4 for S3 requests, and the UTC dates it needs.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use std::fmt::Write;

/// SHA-256 of an empty body.
pub(crate) const EMPTY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// Payload hash for streamed uploads (allowed by S3 and the compatible stores).
pub(crate) const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

/// What the signature covers.
pub(crate) struct Request<'a> {
    pub method: &'a str,
    /// Already URI-encoded path.
    pub path: &'a str,
    /// Canonical (encoded, sorted) query string.
    pub query: &'a str,
    /// Lowercase names with trimmed values, sorted by name; must include `host`,
    /// `x-amz-content-sha256` and `x-amz-date`.
    pub headers: &'a [(String, String)],
    pub payload_hash: &'a str,
    /// `YYYYMMDDTHHMMSSZ`
    pub amz_date: &'a str,
    pub region: &'a str,
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .expect("HMAC takes keys of any length")
        .chain_update(data.as_bytes())
        .finalize()
        .into_bytes()
        .to_vec()
}

/// The `Authorization` header value for `r`.
pub(crate) fn authorization(r: &Request<'_>) -> String {
    let date = &r.amz_date[..8];
    let signed = r
        .headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut canonical = format!("{}\n{}\n{}\n", r.method, r.path, r.query);
    for (k, v) in r.headers {
        let _ = writeln!(canonical, "{k}:{v}");
    }
    let _ = write!(canonical, "\n{signed}\n{}", r.payload_hash);
    let scope = format!("{date}/{}/s3/aws4_request", r.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        r.amz_date,
        hex::encode(Sha256::digest(canonical.as_bytes()))
    );
    let key = [date, r.region, "s3", "aws4_request"].iter().fold(
        format!("AWS4{}", r.secret_access_key).into_bytes(),
        |k, part| hmac(&k, part),
    );
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={}",
        r.access_key_id,
        hex::encode(hmac(&key, &to_sign))
    )
}

/// Percent-encode everything but the unreserved characters (and `/` when `keep_slash`).
pub(crate) fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Encoded `k=v` pairs sorted by name, joined with `&`.
pub(crate) fn canonical_query(pairs: &[(&str, &str)]) -> String {
    let mut enc: Vec<(String, String)> = pairs
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    enc.sort();
    enc.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time.
pub(crate) fn amz_date(unix: u64) -> String {
    let (y, m, d) = civil_from_days((unix / 86_400) as i64);
    let s = unix % 86_400;
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// Unix time of an S3 timestamp such as `2009-10-12T17:50:30.000Z`.
pub(crate) fn parse_timestamp(s: &str) -> Option<u32> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<u32>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y as i64, m, d);
    u32::try_from(days * 86_400 + (hh * 3600 + mm * 60 + ss) as i64).ok()
}

// Howard Hinnant's civil calendar algorithms.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn headers(extra: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut h: Vec<(String, String)> = [
            ("host", "examplebucket.s3.amazonaws.com"),
            ("x-amz-content-sha256", EMPTY_SHA256),
            ("x-amz-date", "20130524T000000Z"),
        ]
        .iter()
        .chain(extra)
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        h.sort();
        h
    }

    fn sign(path: &str, query: &str, headers: &[(String, String)]) -> String {
        authorization(&Request {
            method: "GET",
            path,
            query,
            headers,
            payload_hash: EMPTY_SHA256,
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret_access_key: SECRET,
        })
    }

    // The examples of the S3 "Signature Version 4: header-based auth" documentation.
    #[test]
    fn matches_the_aws_examples() {
        let get = sign("/test.txt", "", &headers(&[("range", "bytes=0-9")]));
        assert_eq!(
            get,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        let query = canonical_query(&[("prefix", "J"), ("max-keys", "2")]);
        assert_eq!(query, "max-keys=2&prefix=J");
        assert!(sign("/", &query, &headers(&[])).ends_with(
            "Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        ));
    }

    #[test]
    fn encodes_like_s3() {
        assert_eq!(uri_encode("a b/c+d~é", true), "a%20b/c%2Bd~%C3%A9");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(amz_date(951_782_400), "20000229T000000Z");
        assert_eq!(
            parse_timestamp("2009-10-12T17:50:30.000Z"),
            Some(1_255_369_830)
        );
        assert_eq!(parse_timestamp("2013-05-24T00:00:00Z"), Some(1_369_353_600));
        assert_eq!(parse_timestamp("yesterday"), None);
    }
}
