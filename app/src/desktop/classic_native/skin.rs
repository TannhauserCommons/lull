//! A Winamp 2 skin (`.wsz`): the sprite sheets of the main, equaliser and playlist windows, their
//! shapes (`region.txt`), the visualiser's colours (`viscolor.txt`), the bitmap font (`text.bmp`)
//! and the playlist's colours and font (`pledit.txt`).
//!
//! The sprite and layout tables, the region and colour readers and the font's folding are adapted
//! from Spotifast's `src/skin/` (https://github.com/crmne/spotifast, commit 672d665), which took the
//! numbers from Webamp's `skinSprites.ts` and `main-window.css`. MIT License, Copyright (c) 2026
//! Carmine Paolino; the notice is in app/licenses/THIRD-PARTY.md.

use std::collections::HashMap;
use std::io::Read;

/// The main window, in skin pixels.
pub const WIDTH: u32 = 275;
pub const HEIGHT: u32 = 116;

/// The longest side a skin bitmap may have, checked before its pixels are allocated: a crafted
/// skin can't ask for more.
const MAX_SIDE: u32 = 4096;
/// A skin's files together, unpacked: real skins are well under 1 MB.
const MAX_UNPACKED: u64 = 32 * 1024 * 1024;

/// A decoded bitmap: premultiplied BGRA (`0xAARRGGBB`), rows top to bottom, ready to copy into the
/// window's own bitmap.
#[derive(Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Bitmap {
    /// A BMP or PNG, whatever the file was called: skins mislabel them. Anything unreadable is
    /// `None`, as Winamp silently skipped it.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_SIDE);
        limits.max_image_height = Some(MAX_SIDE);
        reader.limits(limits);
        let image = reader.decode().ok()?.into_rgba8();
        let (width, height) = image.dimensions();
        let pixels = image.pixels().map(|p| premultiplied(p.0)).collect();
        Some(Self { width, height, pixels })
    }
}

fn premultiplied([r, g, b, a]: [u8; 4]) -> u32 {
    let scale = |c: u8| (u32::from(c) * u32::from(a) + 127) / 255;
    u32::from(a) << 24 | scale(r) << 16 | scale(g) << 8 | scale(b)
}

/// One of the main window's bitmaps, by the file it comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sheet {
    Main,
    CButtons,
    TitleBar,
    ShufRep,
    PosBar,
    Volume,
    Balance,
    MonoSter,
    PlayPaus,
    Numbers,
    /// Digits with a blank cell and a minus sign; newer skins ship it.
    NumsEx,
    /// The 5x6 bitmap font.
    Text,
    EqMain,
    /// The equaliser rolled up; skins from Winamp 2.9 on ship it.
    EqEx,
    PlEdit,
}

impl Sheet {
    const ALL: [Sheet; 15] = [
        Sheet::Main,
        Sheet::CButtons,
        Sheet::TitleBar,
        Sheet::ShufRep,
        Sheet::PosBar,
        Sheet::Volume,
        Sheet::Balance,
        Sheet::MonoSter,
        Sheet::PlayPaus,
        Sheet::Numbers,
        Sheet::NumsEx,
        Sheet::Text,
        Sheet::EqMain,
        Sheet::EqEx,
        Sheet::PlEdit,
    ];

    fn file_stem(self) -> &'static str {
        match self {
            Sheet::Main => "main",
            Sheet::CButtons => "cbuttons",
            Sheet::TitleBar => "titlebar",
            Sheet::ShufRep => "shufrep",
            Sheet::PosBar => "posbar",
            Sheet::Volume => "volume",
            Sheet::Balance => "balance",
            Sheet::MonoSter => "monoster",
            Sheet::PlayPaus => "playpaus",
            Sheet::Numbers => "numbers",
            Sheet::NumsEx => "nums_ex",
            Sheet::Text => "text",
            Sheet::EqMain => "eqmain",
            Sheet::EqEx => "eq_ex",
            Sheet::PlEdit => "pledit",
        }
    }

    /// What stands in when the skin lacks this sheet, as Winamp did: the volume slider's pictures
    /// serve for balance (`Skin::has_balance`), and either digit sheet for the other.
    fn substitute(self) -> Option<Sheet> {
        match self {
            Sheet::Balance => Some(Sheet::Volume),
            Sheet::Numbers => Some(Sheet::NumsEx),
            Sheet::NumsEx => Some(Sheet::Numbers),
            _ => None,
        }
    }
}

/// A rectangle inside a sheet, in skin pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sprite {
    pub sheet: Sheet,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Sprite {
    pub const fn new(sheet: Sheet, x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            sheet,
            x,
            y,
            width,
            height,
        }
    }
}

/// Where each piece of the main window lives in its sheets (Webamp's `skinSprites.ts`).
pub mod sprites {
    use super::{Sheet, Sprite};

    macro_rules! sprites {
        ($($name:ident = ($sheet:ident, $x:expr, $y:expr, $w:expr, $h:expr);)*) => {
            $(pub const $name: Sprite = Sprite::new(Sheet::$sheet, $x, $y, $w, $h);)*
        };
    }

    sprites! {
        MAIN_BACKGROUND = (Main, 0, 0, 275, 116);

        PREVIOUS = (CButtons, 0, 0, 23, 18);
        PREVIOUS_PRESSED = (CButtons, 0, 18, 23, 18);
        PLAY = (CButtons, 23, 0, 23, 18);
        PLAY_PRESSED = (CButtons, 23, 18, 23, 18);
        PAUSE = (CButtons, 46, 0, 23, 18);
        PAUSE_PRESSED = (CButtons, 46, 18, 23, 18);
        STOP = (CButtons, 69, 0, 23, 18);
        STOP_PRESSED = (CButtons, 69, 18, 23, 18);
        NEXT = (CButtons, 92, 0, 22, 18);
        NEXT_PRESSED = (CButtons, 92, 18, 22, 18);
        EJECT = (CButtons, 114, 0, 22, 16);
        EJECT_PRESSED = (CButtons, 114, 16, 22, 16);

        TITLE_BAR_ACTIVE = (TitleBar, 27, 0, 275, 14);
        TITLE_BAR_INACTIVE = (TitleBar, 27, 15, 275, 14);
        OPTIONS_BUTTON_PRESSED = (TitleBar, 0, 9, 9, 9);
        MINIMIZE_BUTTON_PRESSED = (TitleBar, 9, 9, 9, 9);
        SHADE_BUTTON_PRESSED = (TitleBar, 9, 18, 9, 9);
        CLOSE_BUTTON_PRESSED = (TitleBar, 18, 9, 9, 9);
        CLUTTER_BAR = (TitleBar, 304, 0, 8, 43);

        SHUFFLE_OFF = (ShufRep, 28, 0, 47, 15);
        SHUFFLE_OFF_PRESSED = (ShufRep, 28, 15, 47, 15);
        SHUFFLE_ON = (ShufRep, 28, 30, 47, 15);
        SHUFFLE_ON_PRESSED = (ShufRep, 28, 45, 47, 15);
        REPEAT_OFF = (ShufRep, 0, 0, 28, 15);
        REPEAT_OFF_PRESSED = (ShufRep, 0, 15, 28, 15);
        REPEAT_ON = (ShufRep, 0, 30, 28, 15);
        REPEAT_ON_PRESSED = (ShufRep, 0, 45, 28, 15);
        EQ_OFF = (ShufRep, 0, 61, 23, 12);
        PLAYLIST_OFF = (ShufRep, 23, 61, 23, 12);

        POSITION_TRACK = (PosBar, 0, 0, 248, 10);
        POSITION_THUMB = (PosBar, 248, 0, 29, 10);
        POSITION_THUMB_PRESSED = (PosBar, 278, 0, 29, 10);

        VOLUME_THUMB = (Volume, 15, 422, 14, 11);
        VOLUME_THUMB_PRESSED = (Volume, 0, 422, 14, 11);
        BALANCE_THUMB = (Balance, 15, 422, 14, 11);
        BALANCE_THUMB_PRESSED = (Balance, 0, 422, 14, 11);

        STEREO_ON = (MonoSter, 0, 0, 29, 12);
        STEREO_OFF = (MonoSter, 0, 12, 29, 12);
        MONO_ON = (MonoSter, 29, 0, 27, 12);
        MONO_OFF = (MonoSter, 29, 12, 27, 12);

        STATUS_PLAYING = (PlayPaus, 0, 0, 9, 9);
        STATUS_PAUSED = (PlayPaus, 9, 0, 9, 9);
        STATUS_STOPPED = (PlayPaus, 18, 0, 9, 9);
        WORK_INDICATOR_OFF = (PlayPaus, 36, 0, 3, 9);
        WORK_INDICATOR_ON = (PlayPaus, 39, 0, 3, 9);

        // Skins without `nums_ex` have no minus sign: Winamp borrowed the middle bar of the 2.
        NUMBERS_MINUS = (Numbers, 20, 6, 5, 1);
        NUMS_EX_MINUS = (NumsEx, 99, 0, 9, 13);

        EQ_ON = (ShufRep, 0, 73, 23, 12);
        EQ_OFF_PRESSED = (ShufRep, 46, 61, 23, 12);
        EQ_ON_PRESSED = (ShufRep, 46, 73, 23, 12);
        PLAYLIST_ON = (ShufRep, 23, 73, 23, 12);
        PLAYLIST_OFF_PRESSED = (ShufRep, 69, 61, 23, 12);
        PLAYLIST_ON_PRESSED = (ShufRep, 69, 73, 23, 12);

        SHADE_BAR_ACTIVE = (TitleBar, 27, 29, 275, 14);
        SHADE_BAR_INACTIVE = (TitleBar, 27, 42, 275, 14);
        UNSHADE_BUTTON = (TitleBar, 0, 27, 9, 9);
        UNSHADE_BUTTON_PRESSED = (TitleBar, 9, 27, 9, 9);
        SHADE_POSITION_TRACK = (TitleBar, 0, 36, 17, 7);
        SHADE_POSITION_THUMB = (TitleBar, 20, 36, 3, 7);
        SHADE_POSITION_THUMB_LEFT = (TitleBar, 17, 36, 3, 7);
        SHADE_POSITION_THUMB_RIGHT = (TitleBar, 23, 36, 3, 7);

        EQ_BACKGROUND = (EqMain, 0, 0, 275, 116);
        EQ_TITLE_BAR_ACTIVE = (EqMain, 0, 134, 275, 14);
        EQ_TITLE_BAR_INACTIVE = (EqMain, 0, 149, 275, 14);
        EQ_CLOSE_BUTTON_PRESSED = (EqMain, 0, 125, 9, 9);
        EQ_ON_OFF = (EqMain, 10, 119, 26, 12);
        EQ_ON_OFF_PRESSED = (EqMain, 128, 119, 26, 12);
        EQ_ON_ON = (EqMain, 69, 119, 26, 12);
        EQ_ON_ON_PRESSED = (EqMain, 187, 119, 26, 12);
        EQ_AUTO_OFF = (EqMain, 36, 119, 32, 12);
        EQ_AUTO_OFF_PRESSED = (EqMain, 154, 119, 32, 12);
        EQ_AUTO_ON = (EqMain, 95, 119, 32, 12);
        EQ_AUTO_ON_PRESSED = (EqMain, 213, 119, 32, 12);
        EQ_PRESETS = (EqMain, 224, 164, 44, 12);
        EQ_PRESETS_PRESSED = (EqMain, 224, 176, 44, 12);
        EQ_GRAPH = (EqMain, 0, 294, 113, 19);
        // One pixel wide: the colour the curve takes at each row.
        EQ_GRAPH_LINE = (EqMain, 115, 294, 1, 19);
        EQ_PREAMP_LINE = (EqMain, 0, 314, 113, 1);
        EQ_THUMB = (EqMain, 0, 164, 11, 11);
        EQ_THUMB_PRESSED = (EqMain, 0, 176, 11, 11);
        EQ_SHADE_BAR_ACTIVE = (EqEx, 0, 0, 275, 14);
        EQ_SHADE_BAR_INACTIVE = (EqEx, 0, 15, 275, 14);
        EQ_SHADE_BUTTON_PRESSED = (EqEx, 1, 38, 9, 9);
        EQ_UNSHADE_BUTTON_PRESSED = (EqEx, 1, 47, 9, 9);
        EQ_SHADE_CLOSE_BUTTON_PRESSED = (EqEx, 11, 47, 9, 9);
        EQ_SHADE_VOLUME_THUMB_LOW = (EqEx, 1, 30, 3, 7);
        EQ_SHADE_VOLUME_THUMB_MIDDLE = (EqEx, 4, 30, 3, 7);
        EQ_SHADE_VOLUME_THUMB_HIGH = (EqEx, 7, 30, 3, 7);
        EQ_SHADE_BALANCE_THUMB_LEFT = (EqEx, 11, 30, 3, 7);
        EQ_SHADE_BALANCE_THUMB_MIDDLE = (EqEx, 14, 30, 3, 7);
        EQ_SHADE_BALANCE_THUMB_RIGHT = (EqEx, 17, 30, 3, 7);

        PLAYLIST_TOP_LEFT_ACTIVE = (PlEdit, 0, 0, 25, 20);
        PLAYLIST_TITLE_ACTIVE = (PlEdit, 26, 0, 100, 20);
        PLAYLIST_TOP_TILE_ACTIVE = (PlEdit, 127, 0, 25, 20);
        PLAYLIST_TOP_RIGHT_ACTIVE = (PlEdit, 153, 0, 25, 20);
        PLAYLIST_TOP_LEFT = (PlEdit, 0, 21, 25, 20);
        PLAYLIST_TITLE = (PlEdit, 26, 21, 100, 20);
        PLAYLIST_TOP_TILE = (PlEdit, 127, 21, 25, 20);
        PLAYLIST_TOP_RIGHT = (PlEdit, 153, 21, 25, 20);
        PLAYLIST_LEFT_TILE = (PlEdit, 0, 42, 12, 29);
        PLAYLIST_RIGHT_TILE = (PlEdit, 31, 42, 20, 29);
        PLAYLIST_BOTTOM_LEFT = (PlEdit, 0, 72, 125, 38);
        PLAYLIST_BOTTOM_RIGHT = (PlEdit, 126, 72, 150, 38);
        PLAYLIST_BOTTOM_TILE = (PlEdit, 179, 0, 25, 38);
        PLAYLIST_VISUALIZER = (PlEdit, 205, 0, 75, 38);
        PLAYLIST_SCROLL_HANDLE = (PlEdit, 52, 53, 8, 18);
        PLAYLIST_SCROLL_HANDLE_PRESSED = (PlEdit, 61, 53, 8, 18);
        PLAYLIST_CLOSE_PRESSED = (PlEdit, 52, 42, 9, 9);
        PLAYLIST_SHADE_PRESSED = (PlEdit, 62, 42, 9, 9);
        // The pop-out menus along the bottom: each button as it rests and under the pointer, and
        // the bar beside each column.
        PLAYLIST_ADD_URL = (PlEdit, 0, 111, 22, 18);
        PLAYLIST_ADD_URL_HOVER = (PlEdit, 23, 111, 22, 18);
        PLAYLIST_ADD_DIR = (PlEdit, 0, 130, 22, 18);
        PLAYLIST_ADD_DIR_HOVER = (PlEdit, 23, 130, 22, 18);
        PLAYLIST_ADD_FILE = (PlEdit, 0, 149, 22, 18);
        PLAYLIST_ADD_FILE_HOVER = (PlEdit, 23, 149, 22, 18);
        PLAYLIST_REMOVE_MISC = (PlEdit, 54, 168, 22, 18);
        PLAYLIST_REMOVE_MISC_HOVER = (PlEdit, 77, 168, 22, 18);
        PLAYLIST_REMOVE_ALL = (PlEdit, 54, 111, 22, 18);
        PLAYLIST_REMOVE_ALL_HOVER = (PlEdit, 77, 111, 22, 18);
        PLAYLIST_REMOVE_CROP = (PlEdit, 54, 130, 22, 18);
        PLAYLIST_REMOVE_CROP_HOVER = (PlEdit, 77, 130, 22, 18);
        PLAYLIST_REMOVE_SELECTED = (PlEdit, 54, 149, 22, 18);
        PLAYLIST_REMOVE_SELECTED_HOVER = (PlEdit, 77, 149, 22, 18);
        PLAYLIST_SELECT_INVERT = (PlEdit, 104, 111, 22, 18);
        PLAYLIST_SELECT_INVERT_HOVER = (PlEdit, 127, 111, 22, 18);
        PLAYLIST_SELECT_ZERO = (PlEdit, 104, 130, 22, 18);
        PLAYLIST_SELECT_ZERO_HOVER = (PlEdit, 127, 130, 22, 18);
        PLAYLIST_SELECT_ALL = (PlEdit, 104, 149, 22, 18);
        PLAYLIST_SELECT_ALL_HOVER = (PlEdit, 127, 149, 22, 18);
        PLAYLIST_MISC_SORT = (PlEdit, 154, 111, 22, 18);
        PLAYLIST_MISC_SORT_HOVER = (PlEdit, 177, 111, 22, 18);
        PLAYLIST_MISC_INFO = (PlEdit, 154, 130, 22, 18);
        PLAYLIST_MISC_INFO_HOVER = (PlEdit, 177, 130, 22, 18);
        PLAYLIST_MISC_OPTIONS = (PlEdit, 154, 149, 22, 18);
        PLAYLIST_MISC_OPTIONS_HOVER = (PlEdit, 177, 149, 22, 18);
        PLAYLIST_LIST_NEW = (PlEdit, 204, 111, 22, 18);
        PLAYLIST_LIST_NEW_HOVER = (PlEdit, 227, 111, 22, 18);
        PLAYLIST_LIST_SAVE = (PlEdit, 204, 130, 22, 18);
        PLAYLIST_LIST_SAVE_HOVER = (PlEdit, 227, 130, 22, 18);
        PLAYLIST_LIST_LOAD = (PlEdit, 204, 149, 22, 18);
        PLAYLIST_LIST_LOAD_HOVER = (PlEdit, 227, 149, 22, 18);
        PLAYLIST_ADD_BAR = (PlEdit, 48, 111, 3, 54);
        PLAYLIST_REMOVE_BAR = (PlEdit, 100, 111, 3, 72);
        PLAYLIST_SELECT_BAR = (PlEdit, 150, 111, 3, 54);
        PLAYLIST_MISC_BAR = (PlEdit, 200, 111, 3, 54);
        PLAYLIST_LIST_BAR = (PlEdit, 250, 111, 3, 54);
        PLAYLIST_UNSHADE_PRESSED = (PlEdit, 150, 42, 9, 9);
        PLAYLIST_SHADE_LEFT = (PlEdit, 72, 42, 25, 14);
        PLAYLIST_SHADE_TILE = (PlEdit, 72, 57, 25, 14);
        PLAYLIST_SHADE_RIGHT = (PlEdit, 99, 57, 50, 14);
        PLAYLIST_SHADE_RIGHT_ACTIVE = (PlEdit, 99, 42, 50, 14);
    }

    /// The equaliser's sliders are drawn in this many steps.
    pub const EQ_SLIDER_FRAMES: u32 = 28;

    /// An equaliser slider's track at a position, `frame` 0 (all the way down) to 27: two rows of
    /// fourteen, fifteen pixels apart.
    pub fn eq_slider_frame(frame: u32) -> Sprite {
        let frame = frame.min(EQ_SLIDER_FRAMES - 1);
        Sprite::new(Sheet::EqMain, 13 + (frame % 14) * 15, 164 + (frame / 14) * 65, 14, 63)
    }

    /// The volume and balance sliders are drawn in this many steps.
    pub const SLIDER_FRAMES: u32 = 28;

    /// The volume track at a fill, `frame` 0 (silent) to 27 (full).
    pub fn volume_frame(frame: u32) -> Sprite {
        Sprite::new(Sheet::Volume, 0, 15 * frame.min(SLIDER_FRAMES - 1), 68, 13)
    }

    /// The balance track at a deflection, `frame` 0 (centred) to 27.
    pub fn balance_frame(frame: u32) -> Sprite {
        Sprite::new(Sheet::Balance, 9, 15 * frame.min(SLIDER_FRAMES - 1), 38, 13)
    }

    pub fn digit(value: u32, extended: bool) -> Sprite {
        let sheet = if extended { Sheet::NumsEx } else { Sheet::Numbers };
        Sprite::new(sheet, 9 * value.min(9), 0, 9, 13)
    }

    /// A cell of the bitmap font: rows of 5x6 glyphs.
    pub fn glyph(row: u32, column: u32) -> Sprite {
        Sprite::new(Sheet::Text, 5 * column, 6 * row, 5, 6)
    }
}

/// A rectangle in the main window, in skin pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// Where each control sits in the main window: Winamp's own positions, via Webamp's
/// `main-window.css`. Every classic skin paints its background to match.
pub mod layout {
    use super::Area;

    macro_rules! areas {
        ($($name:ident = ($x:expr, $y:expr, $w:expr, $h:expr);)*) => {
            $(pub const $name: Area = Area::new($x, $y, $w, $h);)*
        };
    }

    areas! {
        TITLE_BAR = (0, 0, 275, 14);
        OPTIONS_BUTTON = (6, 3, 9, 9);
        MINIMIZE_BUTTON = (244, 3, 9, 9);
        SHADE_BUTTON = (254, 3, 9, 9);
        CLOSE_BUTTON = (264, 3, 9, 9);
        CLUTTER_BAR = (10, 22, 8, 43);
        WORK_INDICATOR = (24, 28, 3, 9);
        STATUS = (26, 28, 9, 9);
        // The minus sign when the skin has `nums_ex` (a whole digit cell), and when it's borrowed
        // from the digit sheet (one row of pixels).
        MINUS_EX = (36, 26, 9, 13);
        MINUS = (38, 32, 5, 1);
        MINUTE_TENS = (48, 26, 9, 13);
        MINUTE_ONES = (60, 26, 9, 13);
        SECOND_TENS = (78, 26, 9, 13);
        SECOND_ONES = (90, 26, 9, 13);
        MARQUEE = (111, 27, 154, 6);
        KBPS = (111, 43, 15, 6);
        KHZ = (156, 43, 10, 6);
        VISUALIZER = (24, 43, 76, 16);
        MONO = (212, 41, 27, 12);
        STEREO = (239, 41, 29, 12);
        VOLUME = (107, 57, 68, 13);
        BALANCE = (177, 57, 38, 13);
        EQ_BUTTON = (219, 58, 23, 12);
        PLAYLIST_BUTTON = (242, 58, 23, 12);
        POSITION = (16, 72, 248, 10);
        PREVIOUS = (16, 88, 23, 18);
        PLAY = (39, 88, 23, 18);
        PAUSE = (62, 88, 23, 18);
        STOP = (85, 88, 23, 18);
        NEXT = (108, 88, 22, 18);
        EJECT = (136, 89, 22, 16);
        SHUFFLE = (164, 89, 47, 15);
        REPEAT = (210, 89, 28, 15);

        SHADE_VISUALIZER = (79, 5, 38, 5);
        SHADE_TIME = (127, 4, 30, 6);
        SHADE_PREVIOUS = (169, 2, 7, 10);
        SHADE_PLAY = (176, 2, 10, 10);
        SHADE_PAUSE = (186, 2, 9, 10);
        SHADE_STOP = (195, 2, 9, 10);
        SHADE_NEXT = (204, 2, 10, 10);
        SHADE_EJECT = (215, 2, 10, 10);
        SHADE_POSITION = (226, 4, 17, 7);

        EQ_TITLE_BAR = (0, 0, 275, 14);
        EQ_CLOSE = (264, 3, 9, 9);
        EQ_SHADE = (254, 3, 9, 9);
        EQ_ON = (14, 18, 26, 12);
        EQ_AUTO = (40, 18, 32, 12);
        EQ_PRESETS = (217, 18, 44, 12);
        EQ_GRAPH = (86, 17, 113, 19);
        EQ_PREAMP = (21, 38, 14, 63);
        EQ_SHADE_VOLUME = (61, 4, 97, 7);
        EQ_SHADE_BALANCE = (164, 4, 43, 7);

        PLAYLIST_CLOSE = (264, 3, 9, 9);
        PLAYLIST_SHADE = (254, 3, 9, 9);
    }

    /// The window rolled up to its title bar (shade mode).
    pub const SHADE_HEIGHT: u32 = 14;

    /// An equaliser band's slider, the ten eighteen pixels apart.
    pub const fn eq_band(band: usize) -> Area {
        Area::new(78 + 18 * band as u32, 38, 14, 63)
    }

    /// The playlist's frame: its title bar and bottom, its sides, the tiles they repeat, a row.
    pub const PLAYLIST_TITLE_HEIGHT: u32 = 20;
    pub const PLAYLIST_BOTTOM_HEIGHT: u32 = 38;
    pub const PLAYLIST_LEFT_WIDTH: u32 = 12;
    pub const PLAYLIST_RIGHT_WIDTH: u32 = 20;
    pub const PLAYLIST_TILE_WIDTH: u32 = 25;
    pub const PLAYLIST_TILE_HEIGHT: u32 = 29;
    pub const PLAYLIST_ROW_HEIGHT: u32 = 13;
    pub const PLAYLIST_SCROLL_X: u32 = 260;
    pub const PLAYLIST_SCROLL_HANDLE_HEIGHT: u32 = 18;
    /// In the bottom bar: the running time, the song's time, and the little transport.
    pub const PLAYLIST_RUNNING_TIME: (u32, u32) = (132, 10);
    pub const PLAYLIST_TRACK_TIME: (u32, u32) = (191, 23);
    pub const PLAYLIST_MINI_TRANSPORT: (u32, u32) = (128, 22);
    /// The menu buttons along the bottom (ADD, REM, SEL, MISC, LIST), 22 by 18 from eight rows
    /// into the bottom bar.
    pub const PLAYLIST_MENUS: [u32; 5] = [14, 43, 72, 101, 231];

    /// The time display's digit cells, left to right.
    pub const TIME_DIGITS: [Area; 4] = [MINUTE_TENS, MINUTE_ONES, SECOND_TENS, SECOND_ONES];
    /// How far a slider's thumb travels: the track's width less the thumb's; Webamp's volume
    /// slider is 3 pixels narrower than the track's picture.
    pub const VOLUME_TRAVEL: u32 = VOLUME.width - 3 - 14;
    pub const BALANCE_TRAVEL: u32 = BALANCE.width - 14;
    pub const POSITION_TRAVEL: u32 = POSITION.width - 29;
}

pub type Rgb = [u8; 3];

/// The visualiser's palette: background, dot grid, 16 analyser rows from the top down, five
/// oscilloscope colours, and the analyser's peaks.
pub type VisColors = [Rgb; 24];

/// Winamp's own palette, for skins without `viscolor.txt`.
const DEFAULT_VIS_COLORS: VisColors = [
    [0, 0, 0],
    [24, 33, 41],
    [239, 49, 16],
    [206, 41, 16],
    [214, 90, 0],
    [214, 102, 0],
    [214, 115, 0],
    [198, 123, 8],
    [222, 165, 24],
    [214, 181, 33],
    [189, 222, 41],
    [148, 222, 33],
    [41, 206, 16],
    [50, 190, 16],
    [57, 181, 16],
    [49, 156, 8],
    [41, 148, 0],
    [24, 132, 8],
    [255, 255, 255],
    [214, 214, 222],
    [181, 189, 189],
    [160, 170, 175],
    [148, 156, 165],
    [150, 150, 150],
];

/// The main window's shape from `region.txt`: whether each pixel is in it. Whatever it leaves out
/// is see-through, and clicks there reach the window behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    inside: Vec<bool>,
}

impl Mask {
    /// The pixels inside any of the polygons, filled the even-odd way at the pixels' centres, in a
    /// window `height` rows tall.
    fn from_polygons(height: u32, polygons: &[Vec<(i32, i32)>]) -> Self {
        let mut inside = vec![false; (WIDTH * height) as usize];
        for (y, row) in inside.as_chunks_mut::<{ WIDTH as usize }>().0.iter_mut().enumerate() {
            let centre_y = y as f64 + 0.5;
            for polygon in polygons.iter().filter(|polygon| polygon.len() >= 3) {
                let mut crossings = Vec::new();
                for (index, &(x0, y0)) in polygon.iter().enumerate() {
                    let (x1, y1) = polygon[(index + 1) % polygon.len()];
                    let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
                    if y0 == y1 || centre_y < y0.min(y1) || centre_y >= y0.max(y1) {
                        continue;
                    }
                    crossings.push(x0 + (centre_y - y0) * (x1 - x0) / (y1 - y0));
                }
                crossings.sort_by(f64::total_cmp);
                for pair in crossings.as_chunks::<2>().0 {
                    let start = (pair[0] - 0.5).ceil().max(0.0) as usize;
                    let end = ((pair[1] - 0.5).ceil().max(0.0) as usize).min(WIDTH as usize);
                    for pixel in &mut row[start.min(end)..end] {
                        *pixel = true;
                    }
                }
            }
        }
        Self { inside }
    }

    /// Whether the pixel at this index of its window (row by row, at 1x) is in the shape.
    pub fn contains(&self, index: usize) -> bool {
        self.inside.get(index).copied().unwrap_or(false)
    }

    fn is_everything(&self) -> bool {
        self.inside.iter().all(|&pixel| pixel)
    }
}

/// The skin's main window, loaded.
pub struct Skin {
    sheets: HashMap<Sheet, Bitmap>,
    pub vis_colors: VisColors,
    pub regions: Regions,
    pub playlist: PlaylistStyle,
}

/// The windows' shapes; `None` is the whole rectangle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Regions {
    pub main: Option<Mask>,
    pub main_shade: Option<Mask>,
    pub equaliser: Option<Mask>,
    pub equaliser_shade: Option<Mask>,
}

/// The playlist's colours and font (`pledit.txt`'s `[Text]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistStyle {
    pub normal: Rgb,
    pub current: Rgb,
    pub normal_background: Rgb,
    pub selected_background: Rgb,
    pub font: String,
}

impl Default for PlaylistStyle {
    fn default() -> Self {
        Self {
            normal: [0x00, 0xff, 0x00],
            current: [0xff, 0xff, 0xff],
            normal_background: [0x00, 0x00, 0x00],
            selected_background: [0x00, 0x00, 0xff],
            font: "Arial".into(),
        }
    }
}

impl Skin {
    /// Reads a `.wsz` (a zip). File names match without regard to case or folder; when a name
    /// repeats, the last copy wins, as unpacking onto Winamp's folder did. Sheets the skin lacks
    /// come from `base`, Winamp's own skin, as Winamp drew them.
    pub fn from_archive(bytes: &[u8], base: Option<&Skin>) -> Result<Self, String> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a skin: {e}"))?;
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        let mut unpacked = 0u64;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| format!("not a skin: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry
                .name()
                .map_err(|e| format!("not a skin: {e}"))?
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !wanted(&name) {
                continue;
            }
            let mut file = Vec::new();
            let room = MAX_UNPACKED.saturating_sub(unpacked);
            (&mut entry)
                .take(room + 1)
                .read_to_end(&mut file)
                .map_err(|e| format!("can't unpack {name}: {e}"))?;
            unpacked += file.len() as u64;
            if unpacked > MAX_UNPACKED {
                return Err("the skin unpacks to more than 32 MB".into());
            }
            files.insert(name, file);
        }
        let mut sheets = HashMap::new();
        for sheet in Sheet::ALL {
            let stem = sheet.file_stem();
            let decoded = files
                .get(&format!("{stem}.bmp"))
                .or_else(|| files.get(&format!("{stem}.png")))
                .and_then(|bytes| Bitmap::decode(bytes));
            if let Some(bitmap) = decoded {
                sheets.insert(sheet, bitmap);
            }
        }
        if !sheets.contains_key(&Sheet::Main) && base.is_none() {
            return Err("the skin has no main window".into());
        }
        for sheet in Sheet::ALL {
            let has = |s: Sheet| sheets.contains_key(&s);
            if has(sheet) || sheet.substitute().is_some_and(has) {
                continue;
            }
            if let Some(bitmap) = base.and_then(|b| b.sheets.get(&sheet)) {
                sheets.insert(sheet, bitmap.clone());
            }
        }
        let text = |file: &str| {
            files.get(file).map(|bytes| {
                // A skin edited on Windows may start with a byte order mark, which `trim` keeps.
                let text = String::from_utf8_lossy(bytes);
                text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
            })
        };
        Ok(Self {
            sheets,
            vis_colors: text("viscolor.txt").map_or(DEFAULT_VIS_COLORS, |t| parse_vis_colors(&t)),
            regions: text("region.txt").map(|t| parse_regions(&t)).unwrap_or_default(),
            playlist: text("pledit.txt").map(|t| parse_playlist(&t)).unwrap_or_default(),
        })
    }

    /// Whether every sheet the windows draw is here, its own or a stand-in.
    pub fn complete(&self) -> bool {
        Sheet::ALL
            .iter()
            .all(|&s| self.sheets.contains_key(&s) || s.substitute().is_some_and(|t| self.sheets.contains_key(&t)))
    }

    /// Whether the skin has a balance slider of its own; if not, the volume slider's pictures stand
    /// in, whole, as Webamp has them.
    pub fn has_balance(&self) -> bool {
        self.sheets.contains_key(&Sheet::Balance)
    }

    pub fn has_extended_digits(&self) -> bool {
        self.sheets.contains_key(&Sheet::NumsEx)
    }

    /// The bitmap a sprite is cut from, or its stand-in. `None` when neither the skin nor the base
    /// has it: nothing is drawn there, as Winamp drew nothing.
    pub fn sheet(&self, sheet: Sheet) -> Option<&Bitmap> {
        self.sheets
            .get(&sheet)
            .or_else(|| sheet.substitute().and_then(|s| self.sheets.get(&s)))
    }
}

/// The files of a skin that the main window reads.
fn wanted(name: &str) -> bool {
    matches!(name, "viscolor.txt" | "region.txt" | "pledit.txt")
        || Sheet::ALL.iter().any(|s| {
            name.strip_prefix(s.file_stem())
                .is_some_and(|ext| ext == ".bmp" || ext == ".png")
        })
}

/// Reads `viscolor.txt`: one `r,g,b` per line, in order, whatever follows on the line. Missing
/// lines keep Winamp's colour.
fn parse_vis_colors(text: &str) -> VisColors {
    let mut colors = DEFAULT_VIS_COLORS;
    let parsed = text.lines().filter_map(rgb_line);
    for (slot, color) in colors.iter_mut().zip(parsed) {
        *slot = color;
    }
    colors
}

fn rgb_line(line: &str) -> Option<Rgb> {
    let mut channels = line.split(',').map(|field| {
        let digits = field.trim().split(|c: char| !c.is_ascii_digit()).next().unwrap_or("");
        digits.parse::<u32>().ok().map(|value| value.min(255) as u8)
    });
    Some([channels.next()??, channels.next()??, channels.next()??])
}

/// Reads `region.txt`: a section per window (`[Normal]`, `[WindowShade]`, `[Equalizer]`,
/// `[EqualizerWS]`), each with `NumPoints` (the polygons' sizes) and `PointList` (their corners).
/// Skin tools were loose with it, so a polygon short of points is dropped, and a shape that covers
/// everything is no shape.
fn parse_regions(text: &str) -> Regions {
    let mut sections: HashMap<String, (Vec<usize>, Vec<i32>)> = HashMap::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            section = name.trim_end_matches(']').trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let numbers = value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|field| field.trim().parse::<i32>().ok());
        let (counts, points) = sections.entry(section.clone()).or_default();
        match key.trim().to_ascii_lowercase().as_str() {
            "numpoints" => counts.extend(numbers.map(|n| n.max(0) as usize)),
            "pointlist" => points.extend(numbers),
            _ => {}
        }
    }
    let mask = |name: &str, height: u32| {
        let (counts, points) = sections.get(name)?;
        let pairs: Vec<(i32, i32)> = points.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
        let mut polygons = Vec::new();
        let mut at = 0;
        for &count in counts {
            let polygon: Vec<(i32, i32)> = pairs.iter().skip(at).take(count).copied().collect();
            at += count;
            if polygon.len() == count && count >= 3 {
                polygons.push(polygon);
            }
        }
        if polygons.is_empty() {
            return None;
        }
        Some(Mask::from_polygons(height, &polygons)).filter(|mask| !mask.is_everything())
    };
    Regions {
        main: mask("normal", HEIGHT),
        main_shade: mask("windowshade", layout::SHADE_HEIGHT),
        equaliser: mask("equalizer", HEIGHT),
        equaliser_shade: mask("equalizerws", layout::SHADE_HEIGHT),
    }
}

/// Reads `pledit.txt`: an INI file whose `[Text]` section has the colours (`#RRGGBB`, the hash and
/// extra digits allowed, as Winamp allowed them) and the font.
fn parse_playlist(text: &str) -> PlaylistStyle {
    let mut style = PlaylistStyle::default();
    let mut in_text = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[') {
            in_text = section.trim_end_matches(']').trim().eq_ignore_ascii_case("text");
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_text) else {
            continue;
        };
        let value = value.trim();
        let slot = match key.trim().to_ascii_lowercase().as_str() {
            "normal" => &mut style.normal,
            "current" => &mut style.current,
            "normalbg" => &mut style.normal_background,
            "selectedbg" => &mut style.selected_background,
            "font" if !value.is_empty() => {
                style.font = value.to_owned();
                continue;
            }
            _ => continue,
        };
        if let Some(color) = hex(value) {
            *slot = color;
        }
    }
    style
}

/// A `#RRGGBB` colour; anything but six hex digits at the front is none.
fn hex(value: &str) -> Option<Rgb> {
    let digits = value.trim().trim_start_matches('#').as_bytes();
    let nibble = |at: usize| digits.get(at).and_then(|byte| (*byte as char).to_digit(16));
    let channel = |at: usize| Some((nibble(at)? << 4 | nibble(at + 1)?) as u8);
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The skin's bitmap font: `text.bmp`, read as Webamp reads it. Every UTF-16 unit of a line is a
/// cell (Webamp splits its strings so); a letter loses its accent (lodash's `deburr`, which Webamp
/// uses), case goes, and a character the font hasn't leaves its cell blank: the window's own
/// picture shows there. Where Webamp leaves a blank for a character the font has a plain stand-in
/// for (curly quotes, dashes, full-width forms), the stand-in shows.
pub mod font {
    use super::{Sprite, sprites};

    const ROWS: [&str; 2] = [
        "abcdefghijklmnopqrstuvwxyz\"@",
        "0123456789\u{2026}.:()-'!_+\\/[]^&%,=$#",
    ];

    /// A line as cells, `None` where the cell is blank.
    pub fn cells(text: &str) -> impl Iterator<Item = Option<Sprite>> + '_ {
        text.chars().flat_map(|c| {
            let cell = glyph(c);
            // A character beyond the first plane is two UTF-16 units: two cells, both blank.
            let units = if c.len_utf16() == 2 { [None, None] } else { [cell, None] };
            units.into_iter().take(c.len_utf16())
        })
    }

    /// One character's cell, if the font has it.
    pub fn glyph(character: char) -> Option<Sprite> {
        cell(character).map(|(row, column)| sprites::glyph(row, column))
    }

    fn cell(character: char) -> Option<(u32, u32)> {
        let character = fold(character);
        match character {
            ' ' => return Some((0, 30)),
            '?' => return Some((2, 3)),
            '*' => return Some((2, 4)),
            '<' | '{' => return Some((1, 22)),
            '>' | '}' => return Some((1, 23)),
            _ => {}
        }
        ROWS.iter().enumerate().find_map(|(row, cells)| {
            let column = cells.chars().position(|c| c == character)?;
            Some((row as u32, column as u32))
        })
    }

    /// Lower case, the accent dropped as `deburr` drops it (Latin-1 and Latin Extended-A letters to
    /// their first plain letter), and the stand-ins.
    fn fold(character: char) -> char {
        let plain = match character {
            '\u{c0}'..='\u{c6}' | '\u{e0}'..='\u{e6}' | '\u{100}'..='\u{105}' => 'a',
            '\u{c7}' | '\u{e7}' | '\u{106}'..='\u{10d}' => 'c',
            '\u{d0}' | '\u{f0}' | '\u{10e}'..='\u{111}' => 'd',
            '\u{c8}'..='\u{cb}' | '\u{e8}'..='\u{eb}' | '\u{112}'..='\u{11b}' => 'e',
            '\u{11c}'..='\u{123}' => 'g',
            '\u{124}'..='\u{127}' => 'h',
            '\u{cc}'..='\u{cf}' | '\u{ec}'..='\u{ef}' | '\u{128}'..='\u{133}' => 'i',
            '\u{134}' | '\u{135}' => 'j',
            '\u{136}'..='\u{138}' => 'k',
            '\u{139}'..='\u{142}' => 'l',
            '\u{d1}' | '\u{f1}' | '\u{143}'..='\u{148}' | '\u{14a}' | '\u{14b}' => 'n',
            '\u{149}' => '\'',
            '\u{d2}'..='\u{d6}' | '\u{d8}' | '\u{f2}'..='\u{f6}' | '\u{f8}' | '\u{14c}'..='\u{153}' => 'o',
            '\u{154}'..='\u{159}' => 'r',
            '\u{df}' | '\u{15a}'..='\u{161}' | '\u{17f}' => 's',
            '\u{de}' | '\u{fe}' | '\u{162}'..='\u{167}' => 't',
            '\u{d9}'..='\u{dc}' | '\u{f9}'..='\u{fc}' | '\u{168}'..='\u{173}' => 'u',
            '\u{174}' | '\u{175}' => 'w',
            '\u{dd}' | '\u{fd}' | '\u{ff}' | '\u{176}'..='\u{178}' => 'y',
            '\u{179}'..='\u{17e}' => 'z',
            // Full-width forms are ASCII shifted up by 0xFEE0; Japanese and Korean titles use them.
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(character as u32 - 0xfee0).unwrap_or(character),
            '\u{3000}' => ' ',
            '\u{2018}' | '\u{2019}' | '`' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            other => other,
        };
        plain.to_ascii_lowercase()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn letters_lose_case_and_accent_as_in_webamp() {
            assert_eq!(cell('A'), cell('a'));
            assert_eq!(cell('\u{e9}'), cell('e'));
            assert_eq!(cell('\u{c4}'), cell('a'));
            assert_eq!(cell('\u{142}'), cell('l'));
            assert_eq!(cell('\u{df}'), cell('s'));
            assert_eq!(cell('\u{ff21}'), cell('a'));
            assert_eq!(cell('\u{2019}'), cell('\''));
            assert_eq!(cell('{'), cell('['));
        }

        #[test]
        fn what_the_font_hasnt_is_a_blank_cell_a_unit_each() {
            assert_eq!(cell('\u{4e00}'), None);
            assert_eq!(cell('\u{416}'), None);
            assert_eq!(cell('\t'), None);
            let cells: Vec<_> = cells("a\u{1f3b5}b").collect();
            assert_eq!(cells.len(), 4);
            assert_eq!((cells[1], cells[2]), (None, None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_cuts_the_window_to_its_polygons_and_a_full_one_is_no_shape() {
        let regions = parse_regions(
            "[Normal]\nNumPoints=4\nPointList=0,0, 275,0, 275,14, 0,14\n[WindowShade]\nNumPoints=4\nPointList=0,0,275,0,275,14,0,14",
        );
        let at = |x: u32, y: u32| (y * WIDTH + x) as usize;
        let main = regions.main.map(|m| (m.contains(at(10, 5)), m.contains(at(10, 20))));
        assert_eq!(main, Some((true, false)));
        assert_eq!(regions.main_shade, None);
    }

    #[test]
    fn a_skin_is_read_from_its_pictures_and_its_three_text_files() {
        for name in ["main.bmp", "eqmain.png", "pledit.txt", "viscolor.txt", "region.txt"] {
            assert!(wanted(name), "{name}");
        }
        assert!(!wanted("readme.txt"));
    }

    #[test]
    fn the_playlist_style_reads_loose_colours_and_keeps_defaults() {
        let style = parse_playlist("[Text]\nNormal=#00FF00\nCurrent=FFFFFF00\nNormalBG=#zz\nFont=Tahoma\n");
        assert_eq!(
            (style.current, style.normal_background, style.font.as_str()),
            ([255, 255, 255], [0, 0, 0], "Tahoma")
        );
    }

    #[test]
    fn vis_colours_read_loose_lines_and_keep_winamps_for_missing_ones() {
        let colors = parse_vis_colors("1,2,3, // background\n 4 ,5,6\n");
        assert_eq!(
            (colors[0], colors[1], colors[2]),
            ([1, 2, 3], [4, 5, 6], DEFAULT_VIS_COLORS[2])
        );
    }
}
