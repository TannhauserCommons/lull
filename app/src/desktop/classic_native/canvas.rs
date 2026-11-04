//! What Classic's windows are drawn into: one bitmap at 1x, 275 pixels wide and as tall as the
//! windows stacked (main, equaliser, playlist), premultiplied BGRA like the window's own. A pen
//! draws one window's sprites, fills and skin-font text at that window's place in it. The
//! playlist's song names are in a system font, drawn at the screen's own size: the canvas only
//! lists them (`TextRun`) for the window to draw once it has scaled the rest.

use super::skin::{Area, Bitmap, Mask, Rgb, Skin, Sprite, WIDTH, font};

pub struct Canvas {
    pub height: u32,
    pub pixels: Vec<u32>,
    pub texts: Vec<TextRun>,
}

/// A line of the playlist in the system font: where (canvas pixels, at 1x), what, in what colour.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub area: Area,
    pub text: String,
    pub color: Rgb,
    /// Right-aligned (the songs' times).
    pub right: bool,
}

impl Canvas {
    pub fn new(height: u32) -> Self {
        Self {
            height,
            pixels: vec![0; (WIDTH * height) as usize],
            texts: Vec::new(),
        }
    }

    /// The stack is `height` rows tall: a new height starts from nothing.
    pub fn resize(&mut self, height: u32) {
        if height != self.height {
            self.height = height;
            self.pixels.clear();
            self.pixels.resize((WIDTH * height) as usize, 0);
            self.texts.clear();
        }
    }

    /// Wipes the rows of a window about to be drawn again, its text with them.
    pub fn clear(&mut self, top: u32, height: u32) {
        let start = (top * WIDTH) as usize;
        let end = (((top + height) * WIDTH) as usize).min(self.pixels.len());
        self.pixels[start.min(end)..end].fill(0);
        self.texts.retain(|run| run.area.y < top || run.area.y >= top + height);
    }

    /// A pen for the window whose top row is `top`.
    pub fn pen<'a>(&'a mut self, skin: &'a Skin, top: u32) -> Pen<'a> {
        Pen {
            canvas: self,
            skin,
            top,
        }
    }
}

pub struct Pen<'a> {
    canvas: &'a mut Canvas,
    pub skin: &'a Skin,
    top: u32,
}

pub fn color(rgb: Rgb) -> u32 {
    let [r, g, b] = rgb;
    0xff00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

impl Pen<'_> {
    /// A sprite with its top left at `x`, `y` of this window. Whatever part of it the sheet doesn't
    /// hold (skins are not always the sizes the table expects) is left as it was, as Winamp did.
    pub fn sprite(&mut self, sprite: Sprite, x: u32, y: u32) {
        self.sprite_clipped(sprite, x, y, Area::new(0, 0, WIDTH, self.canvas.height));
    }

    pub fn at(&mut self, sprite: Sprite, area: Area) {
        self.sprite(sprite, area.x, area.y);
    }

    /// A sprite drawn only inside `clip` (this window's pixels): the tiles of a frame, cut to fit.
    pub fn sprite_clipped(&mut self, sprite: Sprite, x: u32, y: u32, clip: Area) {
        let Some(sheet) = self.skin.sheet(sprite.sheet) else {
            return;
        };
        let width = sprite.width.min(sheet.width.saturating_sub(sprite.x));
        let height = sprite.height.min(sheet.height.saturating_sub(sprite.y));
        let left = x.max(clip.x);
        let right = (x + width).min(clip.x + clip.width).min(WIDTH);
        if left >= right {
            return;
        }
        for row in 0..height {
            let (wy, sy) = (y + row, sprite.y + row);
            if wy < clip.y || wy >= clip.y + clip.height || self.top + wy >= self.canvas.height {
                continue;
            }
            let from = (sy * sheet.width + sprite.x + (left - x)) as usize;
            let to = ((self.top + wy) * WIDTH + left) as usize;
            let count = (right - left) as usize;
            self.canvas.pixels[to..to + count].copy_from_slice(&sheet.pixels[from..from + count]);
        }
    }

    pub fn fill(&mut self, x: u32, y: u32, width: u32, height: u32, rgb: Rgb) {
        let value = color(rgb);
        for row in y..y + height {
            let Some(start) = self.index(x, row) else {
                continue;
            };
            let end = start + width.min(WIDTH - x) as usize;
            self.canvas.pixels[start..end].fill(value);
        }
    }

    /// Text in the skin's font, a 5-pixel cell each, cut at the area's edge (a cell may show in
    /// part, as in Webamp's marquee); a blank cell leaves what is under it.
    pub fn text(&mut self, text: &str, area: Area) {
        for (i, cell) in font::cells(text).take(area.width.div_ceil(5) as usize).enumerate() {
            if let Some(glyph) = cell {
                self.sprite_clipped(glyph, area.x + 5 * i as u32, area.y, area);
            }
        }
    }

    /// A line in the system font, drawn later at the screen's size.
    pub fn system_text(&mut self, text: &str, area: Area, color: Rgb, right: bool) {
        self.canvas.texts.push(TextRun {
            area: Area::new(area.x, self.top + area.y, area.width, area.height),
            text: text.to_owned(),
            color,
            right,
        });
    }

    /// The colour of a sheet's pixel (the equaliser's curve takes its colours from one).
    pub fn sheet_color(&self, sprite: Sprite, x: u32, y: u32) -> Option<u32> {
        let sheet: &Bitmap = self.skin.sheet(sprite.sheet)?;
        let (sx, sy) = (sprite.x + x, sprite.y + y);
        (sx < sheet.width && sy < sheet.height).then(|| sheet.pixels[(sy * sheet.width + sx) as usize])
    }

    pub fn put(&mut self, x: u32, y: u32, value: u32) {
        if let Some(at) = self.index(x, y) {
            self.canvas.pixels[at] = value;
        }
    }

    /// This window cut to the skin's shape for it: outside, transparent.
    pub fn mask(&mut self, mask: Option<&Mask>, height: u32) {
        let Some(mask) = mask else {
            return;
        };
        let start = (self.top * WIDTH) as usize;
        let end = (((self.top + height) * WIDTH) as usize).min(self.canvas.pixels.len());
        for (i, pixel) in self.canvas.pixels[start..end].iter_mut().enumerate() {
            if !mask.contains(i) {
                *pixel = 0;
            }
        }
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < WIDTH && self.top + y < self.canvas.height).then(|| ((self.top + y) * WIDTH + x) as usize)
    }
}
