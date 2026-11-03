//! The main window drawn from the skin at 1x, the way Winamp 2 drew it: the background, then each
//! control's sprite at its place, the time in the skin's digits, the song in its bitmap font and
//! the analyser in its colours. What the skin's shape leaves out is transparent.

use super::skin::{Area, Rgb, Skin, Sprite, font, layout, sprites};
use super::sound::Now;
use super::vis::{BARS, Bar, ROWS};

/// The window at 1x: premultiplied BGRA, the same layout as the window's own bitmap.
pub struct Canvas {
    pub pixels: Vec<u32>,
}

/// The main window's controls, for hit testing and the pressed look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Options,
    Minimize,
    Shade,
    Close,
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Volume,
    Position,
    Time,
}

impl Control {
    pub fn area(self) -> Area {
        match self {
            Control::Options => layout::OPTIONS_BUTTON,
            Control::Minimize => layout::MINIMIZE_BUTTON,
            Control::Shade => layout::SHADE_BUTTON,
            Control::Close => layout::CLOSE_BUTTON,
            Control::Previous => layout::PREVIOUS,
            Control::Play => layout::PLAY,
            Control::Pause => layout::PAUSE,
            Control::Stop => layout::STOP,
            Control::Next => layout::NEXT,
            Control::Eject => layout::EJECT,
            Control::Volume => layout::VOLUME,
            Control::Position => layout::POSITION,
            Control::Time => Area::new(layout::MINUS_EX.x, layout::MINUS_EX.y, 63, 13),
        }
    }

    /// The control under a point of the window at 1x; `None` is the window itself, which drags.
    pub fn at(x: u32, y: u32) -> Option<Control> {
        const ALL: [Control; 13] = [
            Control::Options,
            Control::Minimize,
            Control::Shade,
            Control::Close,
            Control::Previous,
            Control::Play,
            Control::Pause,
            Control::Stop,
            Control::Next,
            Control::Eject,
            Control::Volume,
            Control::Position,
            Control::Time,
        ];
        ALL.into_iter().find(|c| c.area().contains(x, y))
    }
}

/// Everything the window shows beyond the skin.
pub struct View<'a> {
    pub now: &'a Now,
    pub marquee: &'a str,
    pub volume: f64,
    pub focused: bool,
    /// The button held down, drawn pressed while the pointer is on it.
    pub pressed: Option<Control>,
    /// The position the listener is dragging the seek bar to, as a fraction.
    pub seeking: Option<f64>,
    pub remaining: bool,
    /// Paused, the time blinks: this is its off half.
    pub blink_off: bool,
    pub bars: &'a [Bar; BARS],
    pub shuffle: bool,
    pub repeat: bool,
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; (super::skin::WIDTH * super::skin::HEIGHT) as usize],
        }
    }

    pub fn draw(&mut self, skin: &Skin, view: &View) {
        let mut pen = Pen { canvas: self, skin };
        pen.sprite(sprites::MAIN_BACKGROUND, 0, 0);
        pen.title_bar(view);
        pen.sprite(sprites::CLUTTER_BAR, layout::CLUTTER_BAR.x, layout::CLUTTER_BAR.y);
        pen.status(view.now);
        pen.time(view);
        pen.visualiser(view.bars);
        pen.text(view.marquee, layout::MARQUEE);
        pen.rates(view.now);
        pen.sliders(view);
        pen.transport(view);
        if let Some(mask) = &skin.mask {
            for (i, pixel) in self.pixels.iter_mut().enumerate() {
                if !mask.contains(i) {
                    *pixel = 0;
                }
            }
        }
    }
}

struct Pen<'a> {
    canvas: &'a mut Canvas,
    skin: &'a Skin,
}

impl Pen<'_> {
    /// A sprite with its top left at `x`, `y`. Whatever part of it the sheet doesn't hold (skins are
    /// not always the sizes the table expects) is left as it was, as Winamp did.
    fn sprite(&mut self, sprite: Sprite, x: u32, y: u32) {
        let Some(sheet) = self.skin.sheet(sprite.sheet) else {
            return;
        };
        let width = sprite.width.min(sheet.width.saturating_sub(sprite.x));
        let height = sprite.height.min(sheet.height.saturating_sub(sprite.y));
        let (w, h) = (super::skin::WIDTH, super::skin::HEIGHT);
        for row in 0..height.min(h.saturating_sub(y)) {
            let from = ((sprite.y + row) * sheet.width + sprite.x) as usize;
            let to = ((y + row) * w + x) as usize;
            let count = width.min(w.saturating_sub(x)) as usize;
            self.canvas.pixels[to..to + count].copy_from_slice(&sheet.pixels[from..from + count]);
        }
    }

    fn at(&mut self, sprite: Sprite, area: Area) {
        self.sprite(sprite, area.x, area.y);
    }

    fn fill(&mut self, x: u32, y: u32, width: u32, height: u32, [r, g, b]: Rgb) {
        let color = 0xff00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
        for row in y..y + height {
            let start = (row * super::skin::WIDTH + x) as usize;
            self.canvas.pixels[start..start + width as usize].fill(color);
        }
    }

    /// Text in the skin's font, one 5-pixel cell a character, cut at the area's width.
    fn text(&mut self, text: &str, area: Area) {
        for (i, c) in text.chars().take((area.width / 5) as usize).enumerate() {
            self.sprite(font::glyph(c), area.x + 5 * i as u32, area.y);
        }
    }

    fn title_bar(&mut self, view: &View) {
        let bar = if view.focused {
            sprites::TITLE_BAR_ACTIVE
        } else {
            sprites::TITLE_BAR_INACTIVE
        };
        self.at(bar, layout::TITLE_BAR);
        let buttons = [
            (
                Control::Options,
                sprites::OPTIONS_BUTTON,
                sprites::OPTIONS_BUTTON_PRESSED,
            ),
            (
                Control::Minimize,
                sprites::MINIMIZE_BUTTON,
                sprites::MINIMIZE_BUTTON_PRESSED,
            ),
            (Control::Shade, sprites::SHADE_BUTTON, sprites::SHADE_BUTTON_PRESSED),
            (Control::Close, sprites::CLOSE_BUTTON, sprites::CLOSE_BUTTON_PRESSED),
        ];
        for (control, normal, pressed) in buttons {
            let sprite = if view.pressed == Some(control) { pressed } else { normal };
            self.at(sprite, control.area());
        }
    }

    fn status(&mut self, now: &Now) {
        let status = if now.stopped {
            sprites::STATUS_STOPPED
        } else if now.playing {
            sprites::STATUS_PLAYING
        } else {
            sprites::STATUS_PAUSED
        };
        self.at(status, layout::STATUS);
        let working = if now.loading {
            sprites::WORK_INDICATOR_ON
        } else {
            sprites::WORK_INDICATOR_OFF
        };
        self.at(working, layout::WORK_INDICATOR);
        let sounding = !now.stopped && !now.loading;
        let (stereo, mono) = match (sounding, now.stereo) {
            (false, _) => (sprites::STEREO_OFF, sprites::MONO_OFF),
            (true, true) => (sprites::STEREO_ON, sprites::MONO_OFF),
            (true, false) => (sprites::STEREO_OFF, sprites::MONO_ON),
        };
        self.at(stereo, layout::STEREO);
        self.at(mono, layout::MONO);
    }

    /// Elapsed, or remaining with a minus sign; blank when stopped and in the off half of a pause's
    /// blink. The blank digit is drawn, not left out: under the digits is the skin's own idea of an
    /// empty display, which is not always empty.
    fn time(&mut self, view: &View) {
        let extended = self.skin.has_extended_digits();
        let blank = if extended {
            sprites::NUMS_EX_BLANK
        } else {
            sprites::NUMBERS_BLANK
        };
        let now = view.now;
        if now.stopped || (!now.playing && view.blink_off) {
            for cell in layout::TIME_DIGITS {
                self.at(blank, cell);
            }
            self.minus(extended, false);
            return;
        }
        let length = now.duration.unwrap_or(0.0);
        let position = view.seeking.map_or(now.position, |f| f * length);
        let remaining = view.remaining && length > 0.0;
        let shown = if remaining { length - position } else { position };
        let seconds = shown.max(0.0) as u32;
        let minutes = (seconds / 60).min(99);
        let seconds = seconds % 60;
        let digits = [minutes / 10, minutes % 10, seconds / 10, seconds % 10];
        for (value, cell) in digits.into_iter().zip(layout::TIME_DIGITS) {
            self.at(sprites::digit(value, extended), cell);
        }
        self.minus(extended, remaining);
    }

    fn minus(&mut self, extended: bool, on: bool) {
        match (extended, on) {
            (true, true) => self.at(sprites::NUMS_EX_MINUS, layout::MINUS_EX),
            (true, false) => self.at(sprites::NUMS_EX_BLANK, layout::MINUS_EX),
            (false, true) => self.at(sprites::NUMBERS_MINUS, layout::MINUS),
            (false, false) => self.at(sprites::NUMBERS_NO_MINUS, layout::MINUS),
        }
    }

    /// The analyser: the skin's background and dot grid, nineteen bars in its sixteen row colours,
    /// and the peaks.
    fn visualiser(&mut self, bars: &[Bar; BARS]) {
        let area = layout::VISUALIZER;
        let palette = self.skin.vis_colors;
        self.fill(area.x, area.y, area.width, area.height, palette[0]);
        for y in (0..area.height).step_by(2) {
            for x in (0..area.width).step_by(2) {
                self.fill(area.x + x, area.y + y, 1, 1, palette[1]);
            }
        }
        let row_at = |row: u8| u32::from(row) * area.height / u32::from(ROWS);
        let stride = 4;
        for (index, bar) in bars.iter().enumerate() {
            let x = area.x + stride * index as u32;
            for row in row_at(ROWS - bar.height)..area.height {
                self.fill(x, area.y + row, stride - 1, 1, palette[2 + row as usize]);
            }
            if let Some(peak) = bar.peak {
                self.fill(x, area.y + row_at(ROWS - peak), stride - 1, 1, palette[23]);
            }
        }
    }

    fn rates(&mut self, now: &Now) {
        if now.stopped || now.loading {
            return;
        }
        if let Some(kbps) = now.kbps {
            self.text(&format!("{:>3}", kbps.min(999)), layout::KBPS);
        }
        if let Some(khz) = now.khz {
            self.text(&format!("{:>2}", khz.min(99)), layout::KHZ);
        }
    }

    fn sliders(&mut self, view: &View) {
        let level = view.volume.clamp(0.0, 1.0);
        let frame = (level * f64::from(sprites::SLIDER_FRAMES - 1)).round() as u32;
        self.at(sprites::volume_frame(frame), layout::VOLUME);
        let thumb = if view.pressed == Some(Control::Volume) {
            sprites::VOLUME_THUMB_PRESSED
        } else {
            sprites::VOLUME_THUMB
        };
        let x = layout::VOLUME.x + (level * f64::from(layout::VOLUME_TRAVEL)).round() as u32;
        self.sprite(thumb, x, layout::VOLUME.y + 1);
        // Balance stays centred: Classic's player plays both channels as they are.
        self.at(sprites::balance_frame(0), layout::BALANCE);
        self.sprite(sprites::BALANCE_THUMB, layout::BALANCE.x + 12, layout::BALANCE.y + 1);
        self.at(sprites::EQ_OFF, layout::EQ_BUTTON);
        self.at(sprites::PLAYLIST_OFF, layout::PLAYLIST_BUTTON);
        self.at(sprites::POSITION_TRACK, layout::POSITION);
        let length = view.now.duration.unwrap_or(0.0);
        if !view.now.stopped && length > 0.0 {
            let fraction = view.seeking.unwrap_or(view.now.position / length).clamp(0.0, 1.0);
            let thumb = if view.seeking.is_some() {
                sprites::POSITION_THUMB_PRESSED
            } else {
                sprites::POSITION_THUMB
            };
            let x = layout::POSITION.x + (fraction * f64::from(layout::POSITION_TRAVEL)).round() as u32;
            self.sprite(thumb, x, layout::POSITION.y);
        }
    }

    fn transport(&mut self, view: &View) {
        let buttons = [
            (Control::Previous, sprites::PREVIOUS, sprites::PREVIOUS_PRESSED),
            (Control::Play, sprites::PLAY, sprites::PLAY_PRESSED),
            (Control::Pause, sprites::PAUSE, sprites::PAUSE_PRESSED),
            (Control::Stop, sprites::STOP, sprites::STOP_PRESSED),
            (Control::Next, sprites::NEXT, sprites::NEXT_PRESSED),
            (Control::Eject, sprites::EJECT, sprites::EJECT_PRESSED),
        ];
        for (control, normal, pressed) in buttons {
            let sprite = if view.pressed == Some(control) { pressed } else { normal };
            self.at(sprite, control.area());
        }
        let shuffle = if view.shuffle {
            sprites::SHUFFLE_ON
        } else {
            sprites::SHUFFLE_OFF
        };
        self.at(shuffle, layout::SHUFFLE);
        let repeat = if view.repeat {
            sprites::REPEAT_ON
        } else {
            sprites::REPEAT_OFF
        };
        self.at(repeat, layout::REPEAT);
    }
}
