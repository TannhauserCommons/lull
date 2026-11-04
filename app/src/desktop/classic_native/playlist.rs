//! The playlist window, under the equaliser: the queue as Winamp listed it (`1. Artist - Title`,
//! the time on the right) in the skin's colours and font (`pledit.txt`), the playing song in its
//! own colour, selected rows on theirs. Its frame is the skin's tiles cut to fit, its bottom bar
//! has the running time, the playing song's time and a little transport painted in. Rolled up, it
//! is a bar with the playing song. Frame and positions after Spotifast's
//! `src/ui/winamp/playlist.rs` (MIT, credited in app/licenses/THIRD-PARTY.md).

use super::canvas::Pen;
use super::draw::{Control, Focus, Target, View, mini_time, visualiser};
use super::skin::{Area, Sprite, WIDTH, layout, sprites};

/// The playlist's height, as Webamp opens it.
pub const HEIGHT: u32 = 116;
/// Its resize steps, across and down (Webamp's).
const STEP_X: u32 = 25;
const STEP_Y: u32 = 29;

/// The playlist's size, as Webamp keeps it: so many steps wider and taller than it opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub wide: u32,
    pub tall: u32,
}

impl Size {
    pub fn width(self) -> u32 {
        WIDTH + STEP_X * self.wide
    }

    pub fn height(self) -> u32 {
        HEIGHT + STEP_Y * self.tall
    }

    /// The size the resize handle, dragged `dx`, `dy` pixels from where it was at `self`, makes:
    /// Webamp rounds to whole steps, never smaller than the playlist opens.
    pub fn dragged(self, dx: f64, dy: f64) -> Self {
        let step = |from: u32, by: f64, step: u32| (f64::from(from) + (by / f64::from(step)).round()).max(0.0) as u32;
        Self {
            wide: step(self.wide, dx, STEP_X),
            tall: step(self.tall, dy, STEP_Y),
        }
    }

    /// An area laid out for the smallest playlist, moved with its right edge, or its bottom one,
    /// or both.
    fn moved(self, area: Area, right: bool, bottom: bool) -> Area {
        let dx = if right { self.width() - WIDTH } else { 0 };
        let dy = if bottom { self.height() - HEIGHT } else { 0 };
        Area::new(area.x + dx, area.y + dy, area.width, area.height)
    }

    /// The bottom bar's top.
    fn bottom(self) -> u32 {
        self.height() - layout::PLAYLIST_BOTTOM_HEIGHT
    }

    /// The middle of the window between the frame's tiles, on the list's background.
    fn middle(self) -> Area {
        Area::new(
            layout::PLAYLIST_LEFT_WIDTH,
            layout::PLAYLIST_TITLE_HEIGHT,
            self.width() - layout::PLAYLIST_LEFT_WIDTH - layout::PLAYLIST_RIGHT_WIDTH,
            self.bottom() - layout::PLAYLIST_TITLE_HEIGHT,
        )
    }

    /// The rows' area: the middle less Webamp's 3 px above and below.
    fn list(self) -> Area {
        let middle = self.middle();
        Area::new(
            middle.x,
            middle.y + LIST_PADDING,
            middle.width,
            middle.height - 2 * LIST_PADDING,
        )
    }

    /// How many rows the list holds whole.
    pub fn rows(self) -> usize {
        (self.list().height / layout::PLAYLIST_ROW_HEIGHT) as usize
    }

    /// The scroll bar's column, between the list and the right edge.
    fn scroll(self) -> Area {
        let middle = self.middle();
        Area::new(
            layout::PLAYLIST_SCROLL_X + self.width() - WIDTH,
            middle.y,
            8,
            middle.height,
        )
    }
}

/// The queue as the playlist shows it.
#[derive(Default)]
pub struct List {
    /// `1. Artist - Title`, and the song's length, for each song.
    pub labels: Vec<String>,
    pub lengths: Vec<Option<f64>>,
    pub selected: Vec<bool>,
    /// The first row in sight.
    pub scroll: usize,
    /// The menu along the bottom that is open, and its button under the pointer.
    pub menu: Option<usize>,
    pub hover: Option<usize>,
}

impl List {
    /// The first row in sight, kept so the list never scrolls past its end.
    pub fn scroll_to(&mut self, first: usize, size: Size) {
        self.scroll = first.min(self.labels.len().saturating_sub(size.rows()));
    }

    /// Scrolls just enough for the row to be in sight.
    pub fn show(&mut self, row: usize, size: Size) {
        if row < self.scroll {
            self.scroll_to(row, size);
        } else if row >= self.scroll + size.rows() {
            self.scroll_to(row + 1 - size.rows(), size);
        }
    }
}

/// Above and below the rows (Webamp's `padding: 3px 0`).
const LIST_PADDING: u32 = 3;

/// The row a point of the playlist is on (past the last song: the end), for a drag.
pub fn row_at(y: u32, list: &List, size: Size) -> usize {
    let area = size.list();
    let row = list.scroll + (y.saturating_sub(area.y) / layout::PLAYLIST_ROW_HEIGHT) as usize;
    row.min(list.labels.len())
}

/// The little transport in the bottom bar: six ten-pixel cells, Previous to Eject.
fn mini_transport(size: Size) -> [(Control, Area); 6] {
    let (x, dy) = layout::PLAYLIST_MINI_TRANSPORT;
    let (x, y) = (x + size.width() - WIDTH, size.bottom() + dy);
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
pub fn at(x: u32, y: u32, shaded: bool, list: &List, size: Size) -> Option<Target> {
    let button = |control, area| Target {
        control,
        area,
        knob: 0,
        travel: 0,
    };
    let mut controls = vec![
        button(Control::PlaylistClose, size.moved(layout::PLAYLIST_CLOSE, true, false)),
        button(Control::PlaylistShade, size.moved(layout::PLAYLIST_SHADE, true, false)),
    ];
    if shaded {
        // Rolled up, it resizes only across (Webamp's handle beside the buttons).
        controls.push(button(Control::PlaylistResize, Area::new(size.width() - 29, 3, 9, 9)));
    } else {
        // An open menu stands over everything else.
        if let Some(menu) = list.menu {
            let mut open = (0..MENUS[menu].len())
                .map(|item| button(Control::PlaylistMenuItem(menu, item), menu_item(menu, item, size)));
            if let Some(target) = open.find(|t| t.area.contains(x, y)) {
                return Some(target);
            }
        }
        controls.push(button(
            Control::PlaylistResize,
            Area::new(size.width() - 20, size.height() - 20, 20, 20),
        ));
        controls.extend((0..MENUS.len()).map(|i| button(Control::PlaylistMenu(i), menu_button(i, size))));
        controls.extend(mini_transport(size).map(|(control, area)| button(control, area)));
        let scroll = size.scroll();
        controls.push(Target {
            control: Control::PlaylistScroll,
            area: scroll,
            knob: layout::PLAYLIST_SCROLL_HANDLE_HEIGHT,
            travel: scroll.height - layout::PLAYLIST_SCROLL_HANDLE_HEIGHT,
        });
        let area = size.list();
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
    let size = view.playlist;
    frame(pen, view.focus == Focus::Playlist, size);
    if let Some(area) = analyser_area(view) {
        visualiser(pen, view.bars, area);
    }
    if view.held(Control::PlaylistClose) {
        pen.at(
            sprites::PLAYLIST_CLOSE_PRESSED,
            size.moved(layout::PLAYLIST_CLOSE, true, false),
        );
    }
    if view.held(Control::PlaylistShade) {
        pen.at(
            sprites::PLAYLIST_SHADE_PRESSED,
            size.moved(layout::PLAYLIST_SHADE, true, false),
        );
    }
    rows(pen, view);
    scrollbar(pen, view);
    times(pen, view);
    if let Some(menu) = view.list.menu {
        open_menu(pen, menu, view.list.hover, size);
    }
}

/// The menus' buttons, top to bottom, at rest and under the pointer (Webamp's `.hover`).
const MENUS: [&[(Sprite, Sprite)]; 5] = [
    &[
        (sprites::PLAYLIST_ADD_URL, sprites::PLAYLIST_ADD_URL_HOVER),
        (sprites::PLAYLIST_ADD_DIR, sprites::PLAYLIST_ADD_DIR_HOVER),
        (sprites::PLAYLIST_ADD_FILE, sprites::PLAYLIST_ADD_FILE_HOVER),
    ],
    &[
        (sprites::PLAYLIST_REMOVE_MISC, sprites::PLAYLIST_REMOVE_MISC_HOVER),
        (sprites::PLAYLIST_REMOVE_ALL, sprites::PLAYLIST_REMOVE_ALL_HOVER),
        (sprites::PLAYLIST_REMOVE_CROP, sprites::PLAYLIST_REMOVE_CROP_HOVER),
        (
            sprites::PLAYLIST_REMOVE_SELECTED,
            sprites::PLAYLIST_REMOVE_SELECTED_HOVER,
        ),
    ],
    &[
        (sprites::PLAYLIST_SELECT_INVERT, sprites::PLAYLIST_SELECT_INVERT_HOVER),
        (sprites::PLAYLIST_SELECT_ZERO, sprites::PLAYLIST_SELECT_ZERO_HOVER),
        (sprites::PLAYLIST_SELECT_ALL, sprites::PLAYLIST_SELECT_ALL_HOVER),
    ],
    &[
        (sprites::PLAYLIST_MISC_SORT, sprites::PLAYLIST_MISC_SORT_HOVER),
        (sprites::PLAYLIST_MISC_INFO, sprites::PLAYLIST_MISC_INFO_HOVER),
        (sprites::PLAYLIST_MISC_OPTIONS, sprites::PLAYLIST_MISC_OPTIONS_HOVER),
    ],
    &[
        (sprites::PLAYLIST_LIST_NEW, sprites::PLAYLIST_LIST_NEW_HOVER),
        (sprites::PLAYLIST_LIST_SAVE, sprites::PLAYLIST_LIST_SAVE_HOVER),
        (sprites::PLAYLIST_LIST_LOAD, sprites::PLAYLIST_LIST_LOAD_HOVER),
    ],
];
const MENU_BARS: [Sprite; 5] = [
    sprites::PLAYLIST_ADD_BAR,
    sprites::PLAYLIST_REMOVE_BAR,
    sprites::PLAYLIST_SELECT_BAR,
    sprites::PLAYLIST_MISC_BAR,
    sprites::PLAYLIST_LIST_BAR,
];

/// Where the playlist shows the analyser: in its visualiser's box (three steps wide or more), and
/// only while the main window, which has its own, is closed and something plays, as in Webamp.
pub fn analyser_area(view: &View) -> Option<Area> {
    let size = view.playlist;
    let shows = size.wide > 2 && !view.main_open && !view.now.stopped;
    let x = size.width() - sprites::PLAYLIST_BOTTOM_RIGHT.width - sprites::PLAYLIST_VISUALIZER.width;
    shows.then(|| Area::new(x + 2, size.bottom() + 12, 72, layout::VISUALIZER.height))
}

/// A menu's own button along the bottom; LIST's keeps to the right edge.
fn menu_button(menu: usize, size: Size) -> Area {
    let area = Area::new(
        layout::PLAYLIST_MENUS[menu],
        HEIGHT - layout::PLAYLIST_BOTTOM_HEIGHT + 8,
        22,
        18,
    );
    size.moved(area, menu == MENUS.len() - 1, true)
}

/// Where a menu's button stands: the column rises from the menu's own button along the bottom.
pub fn menu_item(menu: usize, item: usize, size: Size) -> Area {
    let below = menu_button(menu, size);
    let above = (MENUS[menu].len() - 1 - item) as u32;
    Area::new(below.x, below.y - 18 * above, 22, 18)
}

/// An open menu as Webamp draws it: its column of buttons, the one under the pointer lit, and the
/// bar to its left, both standing on the menu's own button.
fn open_menu(pen: &mut Pen, menu: usize, hover: Option<usize>, size: Size) {
    let bar = MENU_BARS[menu];
    let bottom = menu_button(menu, size);
    pen.sprite(bar, bottom.x - bar.width, bottom.y + bottom.height - bar.height);
    for (item, &(rest, lit)) in MENUS[menu].iter().enumerate() {
        pen.at(
            if hover == Some(item) { lit } else { rest },
            menu_item(menu, item, size),
        );
    }
}

/// Rolled up: its bar, with the playing song's name and time.
pub fn shaded(pen: &mut Pen, view: &View) {
    let width = view.playlist.width();
    pen.sprite(sprites::PLAYLIST_SHADE_LEFT, 0, 0);
    let right = width - sprites::PLAYLIST_SHADE_RIGHT.width;
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
        pen.at(
            sprites::PLAYLIST_UNSHADE_PRESSED,
            view.playlist.moved(layout::PLAYLIST_SHADE, true, false),
        );
    }
    // Webamp's: the song as `n. Artist - Title` (its number unpadded), cut to `(205 + the extra
    // width) / 5` cells with an ellipsis, and its length right-aligned 30 pixels from the edge.
    let index = view.now.index;
    let Some(label) = view.list.labels.get(index) else {
        pen.text("[No file]", Area::new(5, 4, width - 5, 6));
        return;
    };
    let units: Vec<u16> = label.trim_start().encode_utf16().collect();
    let cells = ((205 + width - WIDTH) / 5) as usize;
    let title = if units.len() > cells {
        format!("{}\u{2026}", String::from_utf16_lossy(&units[..cells - 1]))
    } else {
        String::from_utf16_lossy(&units)
    };
    pen.text(&title, Area::new(5, 4, width - 5, 6));
    if let Some(length) = view.list.lengths.get(index).copied().flatten() {
        let time = clock(length);
        let cells = 5 * time.len() as u32;
        pen.text(&time, Area::new(width - 30 - cells, 4, cells, 6));
    }
}

/// The frame: the title bar's tiles either side of its centred title, the sides, the bottom.
fn frame(pen: &mut Pen, focused: bool, size: Size) {
    let width = size.width();
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
    // sharing what width is left. The spacers are there only at an even number of steps wide.
    let (tile, height) = (layout::PLAYLIST_TILE_WIDTH, layout::PLAYLIST_TITLE_HEIGHT);
    let spacers = if size.wide.is_multiple_of(2) {
        TITLE_SPACERS
    } else {
        (0, 0)
    };
    let fills = width - 2 * tile - spacers.0 - spacers.1 - TITLE_WIDTH;
    let (left_fill, right_fill) = (fills / 2, fills - fills / 2);
    pen.sprite(top_left, 0, 0);
    let mut x = tile;
    for (run, from_right) in [(spacers.0, false), (left_fill, true)] {
        tile_across(pen, top_tile, Area::new(x, 0, run, height), from_right);
        x += run;
    }
    pen.sprite(title, x, 0);
    x += TITLE_WIDTH;
    for (run, from_right) in [(spacers.1, false), (right_fill, true)] {
        tile_across(pen, top_tile, Area::new(x, 0, run, height), from_right);
        x += run;
    }
    pen.sprite(top_right, width - tile, 0);
    let middle = Area::new(0, layout::PLAYLIST_TITLE_HEIGHT, width, size.middle().height);
    let mut y = middle.y;
    while y < middle.y + middle.height {
        pen.sprite_clipped(PLAYLIST_LEFT_TILE, 0, y, middle);
        pen.sprite_clipped(PLAYLIST_RIGHT_TILE, width - layout::PLAYLIST_RIGHT_WIDTH, y, middle);
        y += layout::PLAYLIST_TILE_HEIGHT;
    }
    // The bottom: Webamp's fill tiled across, its corners over it, and from three steps wide the
    // visualiser's box beside the right one.
    let bottom = size.bottom();
    tile_across(
        pen,
        PLAYLIST_BOTTOM_TILE,
        Area::new(0, bottom, width, layout::PLAYLIST_BOTTOM_HEIGHT),
        false,
    );
    pen.sprite(PLAYLIST_BOTTOM_LEFT, 0, bottom);
    let right = width - PLAYLIST_BOTTOM_RIGHT.width;
    pen.sprite(PLAYLIST_BOTTOM_RIGHT, right, bottom);
    if size.wide > 2 {
        pen.sprite(PLAYLIST_VISUALIZER, right - PLAYLIST_VISUALIZER.width, bottom);
    }
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
    let size = view.playlist;
    let middle = size.middle();
    pen.fill(middle.x, middle.y, middle.width, middle.height, style.normal_background);
    let area = size.list();
    let list = view.list;
    for (slot, index) in (list.scroll..list.labels.len()).take(size.rows()).enumerate() {
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
    let area = view.playlist.scroll();
    let list = view.list;
    let travel = area.height - layout::PLAYLIST_SCROLL_HANDLE_HEIGHT;
    let room = list.labels.len().saturating_sub(view.playlist.rows());
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
    let (size, list) = (view.playlist, view.list);
    let (bottom, dx) = (size.bottom(), size.width() - WIDTH);
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
    pen.text(&running, Area::new(x + dx, bottom + dy, 5 * running.len() as u32, 6));
    let (x, dy) = layout::PLAYLIST_TRACK_TIME;
    mini_time(pen, view, x + dx, bottom + dy);
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
        let size = Size::default();
        assert_eq!(size.rows(), 4);
        let mut list = List {
            labels: vec![String::new(); 10],
            ..List::default()
        };
        list.scroll_to(50, size);
        assert_eq!(list.scroll, 6);
        list.show(2, size);
        assert_eq!(list.scroll, 2);
        list.show(9, size);
        assert_eq!(list.scroll, 6);
        // Two steps taller: 58 more pixels, four more rows.
        let taller = Size { wide: 0, tall: 2 };
        assert_eq!(taller.rows(), 8);
        list.scroll_to(50, taller);
        assert_eq!(list.scroll, 2);
    }

    #[test]
    fn the_handle_resizes_in_webamps_steps() {
        let size = Size { wide: 1, tall: 0 };
        assert_eq!(size.dragged(37.0, 15.0), Size { wide: 2, tall: 1 });
        assert_eq!(size.dragged(-80.0, -100.0), Size { wide: 0, tall: 0 });
        assert_eq!((size.width(), size.height()), (300, 116));
    }

    #[test]
    fn times_read_the_way_webamps_do() {
        assert_eq!((clock(65.4), clock(3725.0)), ("1:05".to_owned(), "62:05".to_owned()));
    }
}
