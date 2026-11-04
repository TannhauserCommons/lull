//! The playlist window, under the equaliser: the queue as Winamp listed it (`1. Artist - Title`,
//! the time on the right) in the skin's colours and font (`pledit.txt`), the playing song in its
//! own colour, selected rows on theirs. Its frame is the skin's tiles cut to fit, its bottom bar
//! has the running time, the playing song's time and a little transport painted in. Rolled up, it
//! is a bar with the playing song. Frame and positions after Spotifast's
//! `src/ui/winamp/playlist.rs` (MIT, credited in app/licenses/THIRD-PARTY.md).

use super::canvas::Pen;
use super::draw::{Control, Focus, Target, View, mini_time};
use super::skin::{Area, Sprite, WIDTH, layout, sprites};

/// The playlist's height, as Webamp opens it.
pub const HEIGHT: u32 = 116;

/// The queue as the playlist shows it.
#[derive(Default)]
pub struct List {
    /// `1. Artist - Title`, and the song's length, for each song.
    pub labels: Vec<String>,
    pub lengths: Vec<Option<f64>>,
    pub selected: Vec<bool>,
    /// The first row in sight.
    pub scroll: usize,
}

impl List {
    /// How many rows the list holds whole.
    pub fn rows() -> usize {
        (list_area().height / layout::PLAYLIST_ROW_HEIGHT) as usize
    }

    /// The first row in sight, kept so the list never scrolls past its end.
    pub fn scroll_to(&mut self, first: usize) {
        self.scroll = first.min(self.labels.len().saturating_sub(Self::rows()));
    }

    /// Scrolls just enough for the row to be in sight.
    pub fn show(&mut self, row: usize) {
        if row < self.scroll {
            self.scroll_to(row);
        } else if row >= self.scroll + Self::rows() {
            self.scroll_to(row + 1 - Self::rows());
        }
    }
}

/// The middle of the window between the frame's tiles, on the list's background.
fn middle_area() -> Area {
    Area::new(
        layout::PLAYLIST_LEFT_WIDTH,
        layout::PLAYLIST_TITLE_HEIGHT,
        WIDTH - layout::PLAYLIST_LEFT_WIDTH - layout::PLAYLIST_RIGHT_WIDTH,
        HEIGHT - layout::PLAYLIST_TITLE_HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT,
    )
}

/// The rows' area: the middle less Webamp's 3 px above and below.
fn list_area() -> Area {
    let middle = middle_area();
    Area::new(
        middle.x,
        middle.y + LIST_PADDING,
        middle.width,
        middle.height - 2 * LIST_PADDING,
    )
}

/// Above and below the rows (Webamp's `padding: 3px 0`).
const LIST_PADDING: u32 = 3;

/// The row a point of the playlist is on (past the last song: the end), for a drag.
pub fn row_at(y: u32, list: &List) -> usize {
    let area = list_area();
    let row = list.scroll + (y.saturating_sub(area.y) / layout::PLAYLIST_ROW_HEIGHT) as usize;
    row.min(list.labels.len())
}

/// The scroll bar's column, between the list and the right edge.
fn scroll_area() -> Area {
    let list = middle_area();
    Area::new(layout::PLAYLIST_SCROLL_X, list.y, 8, list.height)
}

/// The little transport in the bottom bar: six ten-pixel cells, Previous to Eject.
fn mini_transport() -> [(Control, Area); 6] {
    let (x, dy) = layout::PLAYLIST_MINI_TRANSPORT;
    let y = HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT + dy;
    let cell = |i: u32| Area::new(x + 10 * i, y, 10, 10);
    [
        (Control::Previous, cell(0)),
        (Control::Play, cell(1)),
        (Control::Pause, cell(2)),
        (Control::Stop, cell(3)),
        (Control::Next, cell(4)),
        (Control::Eject, cell(5)),
    ]
}

/// The playlist's control under a point of it; `None` is the window itself, which drags.
pub fn at(x: u32, y: u32, shaded: bool, list: &List) -> Option<Target> {
    let button = |control, area| Target {
        control,
        area,
        knob: 0,
        travel: 0,
    };
    let mut controls = vec![
        button(Control::PlaylistClose, layout::PLAYLIST_CLOSE),
        button(Control::PlaylistShade, layout::PLAYLIST_SHADE),
    ];
    if !shaded {
        let bottom = HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT;
        controls.extend(
            layout::PLAYLIST_MENUS
                .iter()
                .enumerate()
                .map(|(i, &mx)| button(Control::PlaylistMenu(i), Area::new(mx, bottom + 8, 22, 18))),
        );
        controls.extend(mini_transport().map(|(control, area)| button(control, area)));
        controls.push(Target {
            control: Control::PlaylistScroll,
            area: scroll_area(),
            knob: layout::PLAYLIST_SCROLL_HANDLE_HEIGHT,
            travel: scroll_area().height - layout::PLAYLIST_SCROLL_HANDLE_HEIGHT,
        });
        let area = list_area();
        if area.contains(x, y) {
            let row = list.scroll + ((y - area.y) / layout::PLAYLIST_ROW_HEIGHT) as usize;
            let line = Area::new(
                area.x,
                y - (y - area.y) % layout::PLAYLIST_ROW_HEIGHT,
                area.width,
                layout::PLAYLIST_ROW_HEIGHT,
            );
            controls.push(button(Control::PlaylistRow(row.min(list.labels.len())), line));
        }
    }
    controls.into_iter().find(|t| t.area.contains(x, y))
}

pub fn window(pen: &mut Pen, view: &View) {
    frame(pen, view.focus == Focus::Playlist);
    if view.held(Control::PlaylistClose) {
        pen.at(sprites::PLAYLIST_CLOSE_PRESSED, layout::PLAYLIST_CLOSE);
    }
    if view.held(Control::PlaylistShade) {
        pen.at(sprites::PLAYLIST_SHADE_PRESSED, layout::PLAYLIST_SHADE);
    }
    rows(pen, view);
    scrollbar(pen, view);
    times(pen, view);
}

/// Rolled up: its bar, with the playing song's name and time.
pub fn shaded(pen: &mut Pen, view: &View) {
    pen.sprite(sprites::PLAYLIST_SHADE_LEFT, 0, 0);
    let right = WIDTH - sprites::PLAYLIST_SHADE_RIGHT.width;
    let run = Area::new(25, 0, right - 25, layout::SHADE_HEIGHT);
    let mut x = 25;
    while x < right {
        pen.sprite_clipped(sprites::PLAYLIST_SHADE_TILE, x, 0, run);
        x += layout::PLAYLIST_TILE_WIDTH;
    }
    let corner = if view.focus == Focus::Playlist {
        sprites::PLAYLIST_SHADE_RIGHT_ACTIVE
    } else {
        sprites::PLAYLIST_SHADE_RIGHT
    };
    pen.sprite(corner, right, 0);
    if view.held(Control::PlaylistShade) {
        pen.at(sprites::PLAYLIST_UNSHADE_PRESSED, layout::PLAYLIST_SHADE);
    }
    // Webamp's: the song as `n. Artist - Title` (its number unpadded), cut to 41 cells with an
    // ellipsis, and its length right-aligned 30 pixels from the edge.
    let index = view.now.index;
    let Some(label) = view.list.labels.get(index) else {
        pen.text("[No file]", Area::new(5, 4, WIDTH - 5, 6));
        return;
    };
    let units: Vec<u16> = label.trim_start().encode_utf16().collect();
    let cells = SHADE_TITLE_CELLS as usize;
    let title = if units.len() > cells {
        format!("{}\u{2026}", String::from_utf16_lossy(&units[..cells - 1]))
    } else {
        String::from_utf16_lossy(&units)
    };
    pen.text(&title, Area::new(5, 4, WIDTH - 5, 6));
    if let Some(length) = view.list.lengths.get(index).copied().flatten() {
        let time = clock(length);
        let width = 5 * time.len() as u32;
        pen.text(&time, Area::new(WIDTH - 30 - width, 4, width, 6));
    }
}

/// How many cells the rolled-up playlist's title has (Webamp's `(205 + extra width) / 5`).
const SHADE_TITLE_CELLS: u32 = 205 / 5;

/// The frame: the title bar's tiles either side of its centred title, the sides, the bottom.
fn frame(pen: &mut Pen, focused: bool) {
    use sprites::*;
    let (top_left, title, top_tile, top_right) = if focused {
        (
            PLAYLIST_TOP_LEFT_ACTIVE,
            PLAYLIST_TITLE_ACTIVE,
            PLAYLIST_TOP_TILE_ACTIVE,
            PLAYLIST_TOP_RIGHT_ACTIVE,
        )
    } else {
        (PLAYLIST_TOP_LEFT, PLAYLIST_TITLE, PLAYLIST_TOP_TILE, PLAYLIST_TOP_RIGHT)
    };
    // Webamp's title bar: the left corner, a 12 px spacer, a fill, the title, a 13 px spacer, a
    // fill and the right corner; the spacers tiled from their left, the fills from their right,
    // sharing what width is left.
    let (tile, height) = (layout::PLAYLIST_TILE_WIDTH, layout::PLAYLIST_TITLE_HEIGHT);
    let fills = WIDTH - 2 * tile - TITLE_SPACERS.0 - TITLE_SPACERS.1 - TITLE_WIDTH;
    let (left_fill, right_fill) = (fills / 2, fills - fills / 2);
    pen.sprite(top_left, 0, 0);
    let mut x = tile;
    for (width, from_right) in [(TITLE_SPACERS.0, false), (left_fill, true)] {
        tile_across(pen, top_tile, Area::new(x, 0, width, height), from_right);
        x += width;
    }
    pen.sprite(title, x, 0);
    x += TITLE_WIDTH;
    for (width, from_right) in [(TITLE_SPACERS.1, false), (right_fill, true)] {
        tile_across(pen, top_tile, Area::new(x, 0, width, height), from_right);
        x += width;
    }
    pen.sprite(top_right, WIDTH - tile, 0);
    let middle = Area::new(0, layout::PLAYLIST_TITLE_HEIGHT, WIDTH, middle_area().height);
    let mut y = middle.y;
    while y < middle.y + middle.height {
        pen.sprite_clipped(PLAYLIST_LEFT_TILE, 0, y, middle);
        pen.sprite_clipped(PLAYLIST_RIGHT_TILE, WIDTH - layout::PLAYLIST_RIGHT_WIDTH, y, middle);
        y += layout::PLAYLIST_TILE_HEIGHT;
    }
    let bottom = HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT;
    pen.sprite(PLAYLIST_BOTTOM_LEFT, 0, bottom);
    pen.sprite(PLAYLIST_BOTTOM_RIGHT, 125, bottom);
}

/// The title bar's spacers either side of the title, and the title.
const TITLE_SPACERS: (u32, u32) = (12, 13);
const TITLE_WIDTH: u32 = 100;

/// `sprite` tiled across `run` from its left edge, or from its right one: CSS's background at
/// `left` or `right`.
fn tile_across(pen: &mut Pen, sprite: Sprite, run: Area, from_right: bool) {
    let step = sprite.width;
    if step == 0 || run.width == 0 {
        return;
    }
    let end = run.x + run.width;
    let mut x = if from_right {
        end.saturating_sub(run.width.div_ceil(step) * step)
    } else {
        run.x
    };
    while x < end {
        pen.sprite_clipped(sprite, x, run.y, run);
        x += step;
    }
}

/// The rows in sight on the skin's background: selected ones on their colour, the playing one in
/// its own.
fn rows(pen: &mut Pen, view: &View) {
    let style = pen.skin.playlist.clone();
    let middle = middle_area();
    pen.fill(middle.x, middle.y, middle.width, middle.height, style.normal_background);
    let area = list_area();
    let list = view.list;
    for (slot, index) in (list.scroll..list.labels.len()).take(List::rows()).enumerate() {
        let y = area.y + slot as u32 * layout::PLAYLIST_ROW_HEIGHT;
        if list.selected.get(index).copied().unwrap_or(false) {
            pen.fill(
                area.x,
                y,
                area.width,
                layout::PLAYLIST_ROW_HEIGHT,
                style.selected_background,
            );
        }
        let color = if index == view.now.index {
            style.current
        } else {
            style.normal
        };
        // The row's title and time; the window lays them out in two columns as Webamp's flex
        // does, the times' as wide as the widest in sight (window.rs `Text::column`).
        let row = Area::new(area.x, y, area.width, layout::PLAYLIST_ROW_HEIGHT);
        let time = list.lengths[index].map(clock).unwrap_or_default();
        pen.system_text(&time, row, color, true);
        pen.system_text(&list.labels[index], row, color, false);
    }
}

/// The scroll handle, where the rows in sight are in the list.
fn scrollbar(pen: &mut Pen, view: &View) {
    let area = scroll_area();
    let list = view.list;
    let travel = area.height - layout::PLAYLIST_SCROLL_HANDLE_HEIGHT;
    let room = list.labels.len().saturating_sub(List::rows());
    let y = if room == 0 {
        0
    } else {
        (list.scroll as f64 / room as f64 * f64::from(travel)).round() as u32
    };
    let handle = if view.held(Control::PlaylistScroll) {
        sprites::PLAYLIST_SCROLL_HANDLE_PRESSED
    } else {
        sprites::PLAYLIST_SCROLL_HANDLE
    };
    pen.sprite(handle, area.x, area.y + y);
}

/// In the bottom bar, in the skin's font: the selected songs' time and the whole list's, and the
/// small clock.
fn times(pen: &mut Pen, view: &View) {
    let bottom = HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT;
    let list = view.list;
    let total: f64 = list.lengths.iter().flatten().sum();
    let selected: f64 = list
        .lengths
        .iter()
        .zip(&list.selected)
        .filter(|(_, s)| **s)
        .filter_map(|(l, _)| *l)
        .sum();
    // Padded with blanks to 18 cells, as Webamp's is: a skin's blank needn't be see-through.
    let running = format!("{:<18}", format!("{}/{}", clock(selected), clock(total)));
    let (x, dy) = layout::PLAYLIST_RUNNING_TIME;
    pen.text(&running, Area::new(x, bottom + dy, 5 * running.len() as u32, 6));
    let (x, dy) = layout::PLAYLIST_TRACK_TIME;
    mini_time(pen, view, x, bottom + dy);
}

/// A song's length as Webamp writes it: minutes, however many, and seconds (`62:05`).
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_whole_rows_and_never_scrolls_past_its_end() {
        assert_eq!(List::rows(), 4);
        let mut list = List {
            labels: vec![String::new(); 10],
            ..List::default()
        };
        list.scroll_to(50);
        assert_eq!(list.scroll, 6);
        list.show(2);
        assert_eq!(list.scroll, 2);
        list.show(9);
        assert_eq!(list.scroll, 6);
    }

    #[test]
    fn times_read_the_way_webamps_do() {
        assert_eq!((clock(65.4), clock(3725.0)), ("1:05".to_owned(), "62:05".to_owned()));
    }
}
