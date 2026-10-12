//! Turn full framebuffers into the parts that changed.
//!
//! IronRDP's client hands over the whole desktop after every graphics update (8 MB at 1080p).
//! Comparing it with the previous frame costs about a millisecond; sending only the changed
//! 64-pixel tiles saves the webview from decoding megabytes per update.

use crate::proto::TILES;

pub const TILE: u16 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Default)]
pub struct Differ {
    prev: Vec<u32>,
    width: u16,
    height: u16,
}

impl Differ {
    /// Rectangles of `frame` (`0x00RRGGBB` pixels, row by row) that differ from the previous
    /// frame. Runs of changed tiles in a tile row merge into one rectangle. A new size, or the
    /// first frame, is one rectangle covering everything.
    pub fn diff(&mut self, frame: &[u32], width: u16, height: u16) -> Vec<Rect> {
        let (w, h) = (usize::from(width), usize::from(height));
        assert_eq!(frame.len(), w * h, "frame size");
        if width != self.width || height != self.height || self.prev.len() != frame.len() {
            self.prev = frame.to_vec();
            self.width = width;
            self.height = height;
            return if frame.is_empty() {
                vec![]
            } else {
                vec![Rect {
                    x: 0,
                    y: 0,
                    w: width,
                    h: height,
                }]
            };
        }
        let mut out = Vec::new();
        for ty in (0..height).step_by(TILE.into()) {
            let th = TILE.min(height - ty);
            let mut run: Option<Rect> = None;
            for tx in (0..width).step_by(TILE.into()) {
                let tw = TILE.min(width - tx);
                let changed = (ty..ty + th).any(|y| {
                    let start = usize::from(y) * w + usize::from(tx);
                    let end = start + usize::from(tw);
                    frame[start..end] != self.prev[start..end]
                });
                if changed {
                    match run.as_mut() {
                        Some(r) => r.w += tw,
                        None => {
                            run = Some(Rect {
                                x: tx,
                                y: ty,
                                w: tw,
                                h: th,
                            })
                        }
                    }
                } else if let Some(r) = run.take() {
                    out.push(r);
                }
            }
            out.extend(run);
        }
        for r in &out {
            for y in r.y..r.y + r.h {
                let start = usize::from(y) * w + usize::from(r.x);
                let end = start + usize::from(r.w);
                self.prev[start..end].copy_from_slice(&frame[start..end]);
            }
        }
        out
    }
}

/// `[u16 width][u16 height][u16 count]` then per rectangle `[u16 x][u16 y][u16 w][u16 h]` and
/// its pixels as RGBA, row by row. All little-endian. `None` when nothing changed.
pub fn encode(frame: &[u32], width: u16, rects: &[Rect]) -> Option<Vec<u8>> {
    if rects.is_empty() {
        return None;
    }
    let pixels: usize = rects
        .iter()
        .map(|r| usize::from(r.w) * usize::from(r.h))
        .sum();
    let mut out = Vec::with_capacity(6 + rects.len() * 8 + pixels * 4);
    let height = frame.len() / usize::from(width.max(1));
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(&(rects.len() as u16).to_le_bytes());
    for r in rects {
        for v in [r.x, r.y, r.w, r.h] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for y in r.y..r.y + r.h {
            let start = usize::from(y) * usize::from(width) + usize::from(r.x);
            for &px in &frame[start..start + usize::from(r.w)] {
                let [_, red, green, blue] = px.to_be_bytes();
                out.extend_from_slice(&[red, green, blue, 255]);
            }
        }
    }
    Some(out)
}

/// The kind byte that goes in front of [`encode`]'s payload.
pub const KIND: u8 = TILES;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u16, h: u16, f: impl Fn(u16, u16) -> u32) -> Vec<u32> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect()
    }

    #[test]
    fn first_frame_and_new_sizes_are_whole() {
        let mut d = Differ::default();
        let f = frame(100, 70, |_, _| 1);
        assert_eq!(
            d.diff(&f, 100, 70),
            vec![Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 70
            }]
        );
        assert!(
            d.diff(&f, 100, 70).is_empty(),
            "same frame, nothing to send"
        );
        let g = frame(70, 100, |_, _| 1);
        assert_eq!(
            d.diff(&g, 70, 100),
            vec![Rect {
                x: 0,
                y: 0,
                w: 70,
                h: 100
            }]
        );
    }

    #[test]
    fn only_changed_tiles_merged_per_row() {
        let (w, h) = (200, 150);
        let mut d = Differ::default();
        d.diff(&frame(w, h, |_, _| 0), w, h);
        // Change one pixel in tiles (0,0) and (1,0) (adjacent) and one in the last, partial tile.
        let f = frame(w, h, |x, y| {
            u32::from((x, y) == (10, 10) || (x, y) == (70, 3) || (x, y) == (199, 149))
        });
        assert_eq!(
            d.diff(&f, w, h),
            vec![
                Rect {
                    x: 0,
                    y: 0,
                    w: 128,
                    h: 64
                },
                Rect {
                    x: 192,
                    y: 128,
                    w: 8,
                    h: 22
                }
            ]
        );
        // The change is remembered: the same frame again sends nothing, reverting sends it back.
        assert!(d.diff(&f, w, h).is_empty());
        assert_eq!(d.diff(&frame(w, h, |_, _| 0), w, h).len(), 2);
    }

    #[test]
    fn encodes_rgba_rows_of_each_rect() {
        let f = frame(3, 2, |x, y| {
            0x00_10_20_30 + u32::from(x) + 0x100 * u32::from(y)
        });
        let out = encode(
            &f,
            3,
            &[Rect {
                x: 1,
                y: 0,
                w: 2,
                h: 2,
            }],
        )
        .unwrap();
        let header: Vec<u16> = out[..14]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(header, vec![3, 2, 1, 1, 0, 2, 2]);
        assert_eq!(
            &out[14..],
            &[
                0x10, 0x20, 0x31, 255, 0x10, 0x20, 0x32, 255, 0x10, 0x21, 0x31, 255, 0x10, 0x21,
                0x32, 255
            ]
        );
        assert!(encode(&f, 3, &[]).is_none());
    }
}
