//! A Winamp 2 skin (`.wsz`): the main window's sprite sheets, its shape (`region.txt`), the
//! visualiser's colours (`viscolor.txt`) and the bitmap font (`text.bmp`).
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
}

impl Sheet {
    const ALL: [Sheet; 12] = [
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
        }
    }

    /// What stands in when the skin lacks this sheet, as Winamp did: balance is cut from the volume
    /// sheet, and either digit sheet serves for the other.
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
        OPTIONS_BUTTON = (TitleBar, 0, 0, 9, 9);
        OPTIONS_BUTTON_PRESSED = (TitleBar, 0, 9, 9, 9);
        MINIMIZE_BUTTON = (TitleBar, 9, 0, 9, 9);
        MINIMIZE_BUTTON_PRESSED = (TitleBar, 9, 9, 9, 9);
        SHADE_BUTTON = (TitleBar, 0, 18, 9, 9);
        SHADE_BUTTON_PRESSED = (TitleBar, 9, 18, 9, 9);
        CLOSE_BUTTON = (TitleBar, 18, 0, 9, 9);
        CLOSE_BUTTON_PRESSED = (TitleBar, 18, 9, 9, 9);
        CLUTTER_BAR = (TitleBar, 304, 0, 8, 43);

        SHUFFLE_OFF = (ShufRep, 28, 0, 47, 15);
        SHUFFLE_ON = (ShufRep, 28, 30, 47, 15);
        REPEAT_OFF = (ShufRep, 0, 0, 28, 15);
        REPEAT_ON = (ShufRep, 0, 30, 28, 15);
        EQ_OFF = (ShufRep, 0, 61, 23, 12);
        PLAYLIST_OFF = (ShufRep, 23, 61, 23, 12);

        POSITION_TRACK = (PosBar, 0, 0, 248, 10);
        POSITION_THUMB = (PosBar, 248, 0, 29, 10);
        POSITION_THUMB_PRESSED = (PosBar, 278, 0, 29, 10);

        VOLUME_THUMB = (Volume, 15, 422, 14, 11);
        VOLUME_THUMB_PRESSED = (Volume, 0, 422, 14, 11);
        BALANCE_THUMB = (Balance, 15, 422, 14, 11);

        STEREO_ON = (MonoSter, 0, 0, 29, 12);
        STEREO_OFF = (MonoSter, 0, 12, 29, 12);
        MONO_ON = (MonoSter, 29, 0, 27, 12);
        MONO_OFF = (MonoSter, 29, 12, 27, 12);

        STATUS_PLAYING = (PlayPaus, 0, 0, 9, 9);
        STATUS_PAUSED = (PlayPaus, 9, 0, 9, 9);
        STATUS_STOPPED = (PlayPaus, 18, 0, 9, 9);
        WORK_INDICATOR_OFF = (PlayPaus, 36, 0, 3, 9);
        WORK_INDICATOR_ON = (PlayPaus, 39, 0, 3, 9);

        // Skins without `nums_ex` have no minus sign: Winamp borrowed the middle bar of the 2 and,
        // to clear it, the same row of the 1.
        NUMBERS_MINUS = (Numbers, 20, 6, 5, 1);
        NUMBERS_NO_MINUS = (Numbers, 9, 6, 5, 1);
        NUMBERS_BLANK = (Numbers, 90, 0, 9, 13);
        NUMS_EX_BLANK = (NumsEx, 90, 0, 9, 13);
        NUMS_EX_MINUS = (NumsEx, 99, 0, 9, 13);
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
    }

    /// The time display's digit cells, left to right.
    pub const TIME_DIGITS: [Area; 4] = [MINUTE_TENS, MINUTE_ONES, SECOND_TENS, SECOND_ONES];
    /// How far a slider's thumb travels: the track's width less the thumb's.
    pub const VOLUME_TRAVEL: u32 = VOLUME.width - 14;
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
    /// The pixels inside any of the polygons, filled the even-odd way at the pixels' centres.
    fn from_polygons(polygons: &[Vec<(i32, i32)>]) -> Self {
        let mut inside = vec![false; (WIDTH * HEIGHT) as usize];
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

    /// Whether the pixel at this index of the window (row by row, at 1x) is in the shape.
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
    /// `None`: the whole rectangle.
    pub mask: Option<Mask>,
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
            mask: text("region.txt").and_then(|t| parse_region(&t)),
        })
    }

    /// Whether every sheet the main window draws is here, its own or a stand-in.
    pub fn complete(&self) -> bool {
        Sheet::ALL
            .iter()
            .all(|&s| self.sheets.contains_key(&s) || s.substitute().is_some_and(|t| self.sheets.contains_key(&t)))
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
    matches!(name, "viscolor.txt" | "region.txt")
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

/// Reads the `[Normal]` section of `region.txt`: `NumPoints` lists polygon sizes, `PointList` their
/// corners. Skin tools were loose with it, so a polygon short of points is dropped, and a shape
/// that covers everything is no shape.
fn parse_region(text: &str) -> Option<Mask> {
    let mut section = String::new();
    let (mut num_points, mut points) = (Vec::<usize>::new(), Vec::<i32>::new());
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
        if section != "normal" {
            continue;
        }
        let numbers = value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|field| field.trim().parse::<i32>().ok());
        match key.trim().to_ascii_lowercase().as_str() {
            "numpoints" => num_points.extend(numbers.map(|n| n.max(0) as usize)),
            "pointlist" => points.extend(numbers),
            _ => {}
        }
    }
    let pairs: Vec<(i32, i32)> = points.as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).collect();
    let mut polygons = Vec::new();
    let mut at = 0;
    for count in num_points {
        let polygon: Vec<(i32, i32)> = pairs.iter().skip(at).take(count).copied().collect();
        at += count;
        if polygon.len() == count && count >= 3 {
            polygons.push(polygon);
        }
    }
    if polygons.is_empty() {
        return None;
    }
    Some(Mask::from_polygons(&polygons)).filter(|mask| !mask.is_everything())
}

/// The skin's bitmap font: `text.bmp`, capitals only, Latin, digits and some punctuation. Lower case
/// maps onto the capitals and accents drop; anything else becomes a question mark.
pub mod font {
    use super::{Sprite, sprites};

    /// The blank cell at the end of the first row.
    const SPACE: (u32, u32) = (0, 30);
    const ROWS: [&str; 3] = [
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ\"@",
        "0123456789\u{2026}.:()-'!_+\\/[]^&%,=$#",
        "\u{c5}\u{d6}\u{c4}?*",
    ];

    pub fn glyph(character: char) -> Sprite {
        let (row, column) = cell(character);
        sprites::glyph(row, column)
    }

    fn cell(character: char) -> (u32, u32) {
        let character = fold(character);
        if character == ' ' {
            return SPACE;
        }
        for (row, cells) in ROWS.iter().enumerate() {
            if let Some(column) = cells.chars().position(|c| c == character) {
                return (row as u32, column as u32);
            }
        }
        (2, 3)
    }

    fn fold(character: char) -> char {
        // Full-width forms are ASCII shifted up by 0xFEE0; Japanese and Korean titles use them.
        let character = match character {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(character as u32 - 0xfee0).unwrap_or('?'),
            '\u{3000}' => ' ',
            other => other,
        };
        match character {
            'a'..='z' => character.to_ascii_uppercase(),
            ' ' | '\t' | '\u{a0}' => ' ',
            '\u{c5}' | '\u{e5}' => '\u{c5}',
            '\u{d6}' | '\u{f6}' => '\u{d6}',
            '\u{c4}' | '\u{e4}' => '\u{c4}',
            '\u{c0}'..='\u{c3}' | '\u{e0}'..='\u{e3}' | '\u{c6}' | '\u{e6}' => 'A',
            '\u{c7}' | '\u{e7}' => 'C',
            '\u{c8}'..='\u{cb}' | '\u{e8}'..='\u{eb}' => 'E',
            '\u{011e}' | '\u{011f}' => 'G',
            '\u{cc}'..='\u{cf}' | '\u{ec}'..='\u{ef}' | '\u{0130}' | '\u{0131}' => 'I',
            '\u{d0}' | '\u{f0}' => 'D',
            '\u{d1}' | '\u{f1}' => 'N',
            '\u{d2}'..='\u{d5}' | '\u{f2}'..='\u{f5}' | '\u{d8}' | '\u{f8}' => 'O',
            '\u{d9}'..='\u{dc}' | '\u{f9}'..='\u{fc}' => 'U',
            '\u{dd}' | '\u{fd}' | '\u{ff}' => 'Y',
            '\u{df}' | '\u{015e}' | '\u{015f}' => 'S',
            '\u{2018}' | '\u{2019}' | '`' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            other => other,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lower_case_accents_and_full_width_forms_share_the_skins_capitals() {
            assert_eq!(cell('a'), cell('A'));
            assert_eq!(cell('\u{e9}'), cell('E'));
            assert_eq!(cell('\u{ff21}'), cell('A'));
            assert_eq!(cell('\u{4e00}'), cell('?'));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_cuts_the_window_to_its_polygons_and_a_full_one_is_no_shape() {
        let mask = parse_region("[Normal]\nNumPoints=4\nPointList=0,0, 275,0, 275,14, 0,14\n");
        let mask = mask.map(|m| {
            (
                m.contains((5 * WIDTH + 10) as usize),
                m.contains((20 * WIDTH + 10) as usize),
            )
        });
        assert_eq!(mask, Some((true, false)));
        assert_eq!(
            parse_region("[Normal]\nNumPoints=4\nPointList=0,0,275,0,275,116,0,116"),
            None
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
