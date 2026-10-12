//! Pixel work on 32-bit images (blue, green, red, alpha bytes per pixel, rows top to
//! bottom). Pure, so it's tested here; `icons.rs` uses it to make app icons look sharp.

/// How an image's see-through information is stored.
#[derive(Debug, PartialEq)]
pub enum Alpha {
    /// Colors already darkened by their pixel's alpha: what Direct2D (and GDI's `AlphaBlend`)
    /// expect.
    Premultiplied,
    /// Colors at full strength, alpha separate: soft edges look too bright if drawn as is.
    Straight,
    /// Every alpha is 0: the image has no transparency at all and is meant to be solid.
    Missing,
}

/// Works out how `pixels` stores transparency. A premultiplied pixel's colors can never
/// be brighter than its alpha, so one that is proves the image is straight.
pub fn alpha_kind(pixels: &[u8]) -> Alpha {
    let quads = pixels.as_chunks::<4>().0;
    let brighter_than_alpha = |p: &[u8; 4]| p[..3].iter().any(|&c| c > p[3]);
    if quads.iter().all(|p| p[3] == 0) && quads.iter().any(brighter_than_alpha) {
        Alpha::Missing
    } else if quads.iter().any(brighter_than_alpha) {
        Alpha::Straight
    } else {
        Alpha::Premultiplied
    }
}

/// Converts `pixels` to premultiplied alpha, whatever `kind` it was.
pub fn premultiply(pixels: &mut [u8], kind: &Alpha) {
    for p in pixels.as_chunks_mut::<4>().0 {
        match kind {
            Alpha::Premultiplied => {}
            Alpha::Missing => p[3] = 255,
            Alpha::Straight => {
                let a = u32::from(p[3]);
                for c in &mut p[..3] {
                    // Rounded: (c × a) / 255.
                    *c = ((u32::from(*c) * a + 127) / 255) as u8;
                }
            }
        }
    }
}

/// Shrinks a `w` × `h` premultiplied image to `to_w` × `to_h` (each no bigger than the
/// original). Every output pixel is the average of the original pixels it covers, with
/// partly covered ones counting partly: a high-quality filter for shrinking that keeps
/// thin lines and soft edges instead of skipping pixels. Done across, then down.
pub fn shrink(pixels: &[u8], w: usize, h: usize, to_w: usize, to_h: usize) -> Vec<u8> {
    let (across, down) = (weights(w, to_w), weights(h, to_h));
    // Across: `to_w` × `h`, kept as fractions until the end so nothing is rounded twice.
    let mut wide = vec![0f32; to_w * h * 4];
    for y in 0..h {
        for (x, ws) in across.iter().enumerate() {
            for &(from_x, weight) in ws {
                for c in 0..4 {
                    wide[(y * to_w + x) * 4 + c] +=
                        f32::from(pixels[(y * w + from_x) * 4 + c]) * weight;
                }
            }
        }
    }
    // Down: `to_w` × `to_h`.
    let mut out = vec![0u8; to_w * to_h * 4];
    for (y, ws) in down.iter().enumerate() {
        for x in 0..to_w {
            for c in 0..4 {
                let sum: f32 = ws
                    .iter()
                    .map(|&(from_y, weight)| wide[(from_y * to_w + x) * 4 + c] * weight)
                    .sum();
                out[(y * to_w + x) * 4 + c] = sum.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// For each of `to` output pixels in a line of `from` pixels: which original pixels it
/// covers and how much each counts (the shares add up to 1).
fn weights(from: usize, to: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = from as f32 / to as f32;
    (0..to)
        .map(|i| {
            let (start, end) = (i as f32 * scale, (i + 1) as f32 * scale);
            (start.floor() as usize..(end.ceil() as usize).min(from))
                .map(|p| {
                    let covered = end.min(p as f32 + 1.0) - start.max(p as f32);
                    (p, covered / scale)
                })
                .filter(|&(_, weight)| weight > 0.0)
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_kind_is_detected() {
        // Half see-through gray, stored premultiplied (64 ≤ 128) or straight (200 > 128).
        assert_eq!(
            alpha_kind(&[64, 64, 64, 128, 0, 0, 0, 0]),
            Alpha::Premultiplied
        );
        assert_eq!(
            alpha_kind(&[200, 200, 200, 128, 0, 0, 0, 0]),
            Alpha::Straight
        );
        assert_eq!(alpha_kind(&[200, 10, 30, 0, 5, 5, 5, 0]), Alpha::Missing);
    }

    #[test]
    fn premultiply_darkens_by_alpha_or_makes_solid() {
        let mut straight = [200, 100, 0, 128];
        premultiply(&mut straight, &Alpha::Straight);
        assert_eq!(straight, [100, 50, 0, 128]);
        let mut missing = [200, 100, 0, 0];
        premultiply(&mut missing, &Alpha::Missing);
        assert_eq!(missing, [200, 100, 0, 255]);
    }

    #[test]
    fn shrinking_averages_the_pixels_covered() {
        // 2×2 -> 1×1: the average of all four.
        let square = [
            0, 0, 0, 0, 100, 100, 100, 100, 200, 200, 200, 200, 100, 100, 100, 100,
        ];
        assert_eq!(shrink(&square, 2, 2, 1, 1), [100, 100, 100, 100]);
        // 3 -> 2 across: the middle pixel is shared half and half.
        let line = [30, 0, 0, 255, 90, 0, 0, 255, 150, 0, 0, 255];
        assert_eq!(shrink(&line, 3, 1, 2, 1), [50, 0, 0, 255, 130, 0, 0, 255]);
        // Same size: unchanged.
        assert_eq!(shrink(&line, 3, 1, 3, 1), line);
    }

    #[test]
    fn shares_always_add_up_to_one() {
        for (from, to) in [(256, 32), (48, 32), (256, 40), (7, 3)] {
            for ws in weights(from, to) {
                let total: f32 = ws.iter().map(|&(_, w)| w).sum();
                assert!((total - 1.0).abs() < 1e-4, "{from} -> {to}: {total}");
            }
        }
    }
}
