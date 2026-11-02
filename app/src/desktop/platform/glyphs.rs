//! The taskbar thumbnail buttons' glyphs (Previous, Play, Pause, Next), drawn at whatever size the
//! display asks for, so they stay crisp at every scale. Shapes are convex polygons on a 16-unit
//! grid; each pixel's coverage comes from 4 x 4 samples.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

type Polygon = &'static [(f32, f32)];

const PLAY: &[Polygon] = &[&[(5.0, 3.0), (13.0, 8.0), (5.0, 13.0)]];
const PAUSE: &[Polygon] = &[
    &[(4.0, 3.0), (7.0, 3.0), (7.0, 13.0), (4.0, 13.0)],
    &[(9.0, 3.0), (12.0, 3.0), (12.0, 13.0), (9.0, 13.0)],
];
/// Next; Previous is its mirror image.
const NEXT: &[Polygon] = &[
    &[(3.0, 3.0), (10.5, 8.0), (3.0, 13.0)],
    &[(11.0, 3.0), (13.0, 3.0), (13.0, 13.0), (11.0, 13.0)],
];

const GRID: f32 = 16.0;
const SAMPLES: u32 = 4;

/// `size` x `size` pixels, BGRA with straight alpha, in colour `rgb`.
pub fn render(glyph: Glyph, size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let (shapes, mirrored) = match glyph {
        Glyph::Previous => (NEXT, true),
        Glyph::Play => (PLAY, false),
        Glyph::Pause => (PAUSE, false),
        Glyph::Next => (NEXT, false),
    };
    let scale = GRID / size as f32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let gx = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let gy = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    let gx = if mirrored { GRID - gx } else { gx };
                    if shapes.iter().any(|p| inside(p, gx, gy)) {
                        hits += 1;
                    }
                }
            }
            let alpha = (hits * 255 / (SAMPLES * SAMPLES)) as u8;
            pixels.extend_from_slice(&[rgb[2], rgb[1], rgb[0], alpha]);
        }
    }
    pixels
}

/// Inside a convex polygon: on the same side of every edge.
fn inside(polygon: &[(f32, f32)], x: f32, y: f32) -> bool {
    let n = polygon.len();
    let side = |i: usize| {
        let (ax, ay) = polygon[i];
        let (bx, by) = polygon[(i + 1) % n];
        (bx - ax) * (y - ay) - (by - ay) * (x - ax)
    };
    (0..n).all(|i| side(i) >= 0.0) || (0..n).all(|i| side(i) <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(pixels: &[u8], size: u32, x: u32, y: u32) -> u8 {
        pixels[((y * size + x) * 4 + 3) as usize]
    }

    #[test]
    fn glyphs_fill_their_shapes_and_leave_the_rest_clear() {
        let play = render(Glyph::Play, 16, [255, 255, 255]);
        assert_eq!(alpha(&play, 16, 7, 8), 255);
        assert_eq!(alpha(&play, 16, 1, 1), 0);
        let pause = render(Glyph::Pause, 32, [255, 255, 255]);
        assert_eq!(alpha(&pause, 32, 16, 16), 0, "the gap between the bars");
        // Previous mirrors Next: its bar is on the left.
        let next = render(Glyph::Next, 16, [0, 0, 0]);
        let previous = render(Glyph::Previous, 16, [0, 0, 0]);
        assert_eq!(alpha(&next, 16, 12, 8), 255);
        assert_eq!(alpha(&previous, 16, 3, 8), 255);
    }
}
