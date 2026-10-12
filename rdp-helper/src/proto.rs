//! The stdio protocol between Handrail and this helper.
//!
//! Every message is `[u32 LE length][u8 type][payload]`, where the length counts the type byte
//! and the payload. Handrail → helper messages carry JSON; helper → Handrail messages are binary
//! where they carry pixels, and Handrail forwards them to the UI untouched.

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Larger messages are refused (a full 8192×8192 frame is 256 MiB; nothing legitimate comes
/// close in the Handrail → helper direction).
pub const MAX_INBOUND: u32 = 1 << 20;

// Handrail → helper.
pub const START: u8 = 0x01;
pub const INPUT: u8 = 0x02;
pub const RESIZE: u8 = 0x03;
pub const CLOSE: u8 = 0x04;

// Helper → Handrail (and on to the UI).
pub const TILES: u8 = 0x11;
pub const POINTER: u8 = 0x12;
pub const ERROR: u8 = 0x13;
pub const TERMINATED: u8 = 0x14;

pub const POINTER_DEFAULT: u8 = 0;
pub const POINTER_HIDDEN: u8 = 1;
pub const POINTER_BITMAP: u8 = 3;

/// First message: where to connect and as whom. Sent over stdin so the password never shows
/// up in a process listing.
#[derive(Debug, Deserialize)]
pub struct Start {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub domain: Option<String>,
    pub password: String,
    pub width: u16,
    pub height: u16,
    /// Percent (100 = no scaling).
    #[serde(default = "default_scale")]
    pub scale: u32,
}

fn default_scale() -> u32 {
    100
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Input {
    Move {
        x: u16,
        y: u16,
    },
    /// `button` as in the DOM: 0 left, 1 middle, 2 right, 3 back, 4 forward.
    Down {
        button: u8,
    },
    Up {
        button: u8,
    },
    /// Positive `units` scroll up / right; 120 is one notch.
    Wheel {
        vertical: bool,
        units: i16,
    },
    /// A PC/AT set-1 scancode; extended keys carry 0xE0 in the high byte (0xE01D = right Ctrl).
    Key {
        code: u16,
        down: bool,
    },
    /// Release every key and button held (the view lost focus).
    ReleaseAll,
}

#[derive(Debug, Deserialize)]
pub struct Resize {
    pub width: u16,
    pub height: u16,
    #[serde(default = "default_scale")]
    pub scale: u32,
}

/// Read one message; `None` at a clean end of input.
pub async fn read<R: AsyncRead + Unpin>(r: &mut R) -> anyhow::Result<Option<(u8, Vec<u8>)>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len);
    anyhow::ensure!((1..=MAX_INBOUND).contains(&len), "bad message length {len}");
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    let kind = buf.remove(0);
    Ok(Some((kind, buf)))
}

pub async fn write<W: AsyncWrite + Unpin>(
    w: &mut W,
    kind: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = u32::try_from(payload.len() + 1).map_err(std::io::Error::other)?;
    w.write_all(&len.to_le_bytes()).await?;
    w.write_all(&[kind]).await?;
    w.write_all(payload).await?;
    w.flush().await
}

/// A pointer shape: `[kind]`, and for a bitmap `[u16 w][u16 h][u16 hot_x][u16 hot_y][RGBA…]`
/// (premultiplied alpha, as IronRDP decodes it).
pub fn pointer_bitmap(width: u16, height: u16, hot_x: u16, hot_y: u16, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + rgba.len());
    out.push(POINTER_BITMAP);
    for v in [width, height, hot_x, hot_y] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(rgba);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_and_rejects_bad_lengths() {
        let mut buf = Vec::new();
        write(&mut buf, INPUT, br#"[{"t":"move","x":3,"y":4}]"#)
            .await
            .unwrap();
        write(&mut buf, CLOSE, b"").await.unwrap();
        let mut r = &buf[..];
        let (k, p) = read(&mut r).await.unwrap().unwrap();
        assert_eq!(k, INPUT);
        let ops: Vec<Input> = serde_json::from_slice(&p).unwrap();
        assert_eq!(ops, vec![Input::Move { x: 3, y: 4 }]);
        assert_eq!(read(&mut r).await.unwrap().unwrap(), (CLOSE, vec![]));
        assert!(read(&mut r).await.unwrap().is_none());

        let mut bad = &(MAX_INBOUND + 1).to_le_bytes()[..];
        assert!(read(&mut bad).await.is_err());
        let mut zero = &0u32.to_le_bytes()[..];
        assert!(read(&mut zero).await.is_err());
    }

    #[test]
    fn parses_every_input_kind() {
        let ops: Vec<Input> = serde_json::from_str(
            r#"[{"t":"down","button":2},{"t":"up","button":0},{"t":"wheel","vertical":true,"units":-120},
                {"t":"key","code":57373,"down":true},{"t":"release_all"}]"#,
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                Input::Down { button: 2 },
                Input::Up { button: 0 },
                Input::Wheel {
                    vertical: true,
                    units: -120
                },
                Input::Key {
                    code: 0xE01D,
                    down: true
                },
                Input::ReleaseAll,
            ]
        );
    }

    #[test]
    fn start_keeps_defaults() {
        let s: Start = serde_json::from_str(
            r#"{"host":"h","port":3389,"username":"u","password":"p","width":800,"height":600}"#,
        )
        .unwrap();
        assert_eq!((s.scale, s.domain), (100, None));
    }
}
