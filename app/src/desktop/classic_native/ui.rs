//! Classic's three windows stacked, whatever window holds them: their layout, what the listener
//! does to them (the pointer, the keys, the menus) and what Winamp would do about it, and each
//! frame drawn at 1x into the canvas (canvas.rs and a module per window), only the windows that
//! changed. The platform's window (window.rs on Windows, gtk_window.rs on Linux) feeds it the
//! pointer in its own pixels and the keys, shows the menus it asks for, and puts on screen what a
//! frame says changed: scaled up nearest neighbour, the playlist's text drawn over it at the
//! screen's own size.

use super::canvas::{Canvas, TextRun};
use super::dock;
use super::draw::{self, Control, Focus, Target, View};
use super::engine::{Command, Snapshot, Status};
use super::eq::{self, presets};
use super::eqwin;
use super::playlist::{self, List};
use super::queue::Repeat;
use super::session::Session;
use super::skin::{HEIGHT, Skin, WIDTH, layout};
use super::vis::{Analyser, BARS, Bar, FFT_SAMPLES};
use std::time::{Duration, Instant};

/// While something moves (the analyser, a playing song): Winamp's own sixty a second.
const ANIMATE: Duration = Duration::from_millis(16);
/// Otherwise only the scrolling title and a pause's blinking time change.
const IDLE: Duration = Duration::from_millis(110);
/// The title scrolls a character this often (Winamp's pace), with this between its ends.
const MARQUEE_STEP: Duration = Duration::from_millis(220);
const MARQUEE_GAP: &str = "  ***  ";
/// Characters the marquee holds.
const MARQUEE_CHARS: usize = 31;
/// What the marquee says for the playlist menus' buttons lull's queue has no use for.
const ADD_IN_LULL: &str = "Classic plays lull's queue: add songs in lull";
const LISTS_IN_LULL: &str = "Classic plays lull's queue: lists are kept in lull";
const NOT_IN_CLASSIC: &str = "Not in Classic";
/// A notice (a skin that didn't load) holds the marquee this long.
const NOTICE: Duration = Duration::from_secs(8);
/// Rows a wheel notch scrolls the playlist, as Windows' lists do.
const WHEEL_ROWS: f64 = 3.0;

/// What the window plays and shows; built off the main thread, opened on it.
pub struct Setup {
    pub skin: Skin,
    /// What went wrong with the listener's skin, if anything: the marquee says it.
    pub said: Option<String>,
    pub session: Session,
    /// Classic's size (1x to 3x), and the monitor's own scale on top.
    pub size: f64,
    pub dpi: f64,
    /// Top left, physical pixels.
    pub at: (i32, i32),
    /// The listener's favourite skins (name, file), for the Options menu.
    pub favourites: Vec<(String, String)>,
}

/// What happens once the listener leaves Classic: the session as it is then, and the size.
pub type OnClose = Box<dyn FnOnce(Session, f64)>;

/// The three windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Main,
    Eq,
    Playlist,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::Main, Panel::Eq, Panel::Playlist];

    fn index(self) -> usize {
        self as usize
    }

    /// This window as the selected one.
    fn focus(self) -> Focus {
        match self {
            Panel::Main => Focus::Main,
            Panel::Eq => Focus::Equaliser,
            Panel::Playlist => Focus::Playlist,
        }
    }
}

/// Which windows show, and which are rolled up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Panels {
    main: bool,
    eq: bool,
    playlist: bool,
    main_shade: bool,
    eq_shade: bool,
    playlist_shade: bool,
    /// The playlist's size (its corner drags it bigger).
    playlist_size: playlist::Size,
}

impl Panels {
    /// Whether a window shows (Webamp's windows open or closed).
    fn shown(&mut self, panel: Panel) -> &mut bool {
        match panel {
            Panel::Main => &mut self.main,
            Panel::Eq => &mut self.eq,
            Panel::Playlist => &mut self.playlist,
        }
    }

    fn showing(&self, panel: Panel) -> bool {
        match panel {
            Panel::Main => self.main,
            Panel::Eq => self.eq,
            Panel::Playlist => self.playlist,
        }
    }

    fn shaded(&self, panel: Panel) -> bool {
        match panel {
            Panel::Main => self.main_shade,
            Panel::Eq => self.eq_shade,
            Panel::Playlist => self.playlist_shade,
        }
    }

    /// A window's size in skin pixels, rolled up or not.
    fn size(&self, panel: Panel) -> (u32, u32) {
        let rolled = |height: u32| {
            if self.shaded(panel) {
                layout::SHADE_HEIGHT
            } else {
                height
            }
        };
        match panel {
            Panel::Main | Panel::Eq => (WIDTH, rolled(HEIGHT)),
            Panel::Playlist => (self.playlist_size.width(), rolled(self.playlist_size.height())),
        }
    }
}

/// One of the windows as it is drawn: its canvas at 1x, where its skin pixels fall at the screen's
/// size, and what is on screen.
pub struct Pane {
    pub canvas: Canvas,
    /// For each skin column and row, the first physical one it covers (the last entry is the size).
    pub columns: Vec<i32>,
    pub rows: Vec<i32>,
    /// The frame on screen, at 1x: what the next one is compared with.
    shown: Vec<u32>,
    shown_texts: Vec<TextRun>,
    /// Its fingerprint when it was last drawn.
    print: Option<u64>,
    /// Whether the shape it was last cut to still holds: none since the size or the skin changed.
    /// Skins' sheets are opaque, so only those change it.
    shaped: bool,
}

impl Pane {
    fn new() -> Self {
        Self {
            canvas: Canvas::new(0, 0),
            columns: Vec::new(),
            rows: Vec::new(),
            shown: Vec::new(),
            shown_texts: Vec::new(),
            print: None,
            shaped: false,
        }
    }

    /// The pane's physical size.
    fn physical(&self) -> (i32, i32) {
        (
            self.columns.last().copied().unwrap_or(0),
            self.rows.last().copied().unwrap_or(0),
        )
    }

    /// A point of its window as the skin pixel it falls in.
    fn skin_point(&self, (x, y): (i32, i32)) -> (u32, u32) {
        (skin_at(x, &self.columns), skin_at(y, &self.rows))
    }

    /// Nothing of what's on screen is compared with: all of it is drawn again, and cut again.
    fn forget(&mut self) {
        self.shown.clear();
        self.shown_texts.clear();
        self.print = None;
        self.shaped = false;
    }
}

/// The windows being dragged: which, from where the pointer was (desktop pixels), and where all
/// the windows were then.
struct Drag {
    moving: Vec<usize>,
    from: (i32, i32),
    places: [(i32, i32); 3],
}

/// The popup menus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Menu {
    Options,
    /// The copies a song waiting in `choose` may play from.
    Choose,
    Presets,
    /// MISC's sort button: Webamp's menu of orders.
    Sort,
    /// MISC's options button.
    MiscOptions,
}

/// The stack and everything the listener does to it; the platform's window holds one.
pub struct Classic {
    pub skin: Skin,
    pub session: Session,
    pub on_close: Option<OnClose>,
    favourites: Vec<(String, String)>,
    /// The three windows as drawn, in `Panel` order.
    panes: [Pane; 3],
    /// Each window's top left on the desktop, physical pixels, shown or not.
    places: [(i32, i32); 3],
    /// The scale the windows were last laid out at, and the sizes they had (skin pixels).
    laid_out: Option<(f64, [(u32, u32); 3])>,
    /// Whether the platform lets the windows be placed (Wayland doesn't): if not, the platform
    /// moves each and they don't dock.
    pub places_windows: bool,
    /// The work areas of the desktop's monitors, for snapping and for keeping windows on them.
    areas: Vec<dock::Rect>,
    drag: Option<Drag>,
    /// The player as of this frame, and the queue's revision the playlist shows.
    snapshot: Snapshot,
    revision: Option<u64>,
    /// Classic's size (1x to 3x), and the monitor's own scale on top.
    pub size: f64,
    pub dpi: f64,
    panels: Panels,
    list: List,
    analyser: Analyser,
    /// The analyser's last bars: paused, they hold still, as Webamp's do.
    bars: [Bar; BARS],
    /// The selected window of the three (Webamp's focus).
    focus: Focus,
    /// The control held down and its window; for a button, whether the pointer is still on it.
    pressed: Option<(Target, Panel)>,
    over: bool,
    seeking: Option<f64>,
    /// The playlist's size when its corner was grabbed, and where the pointer was (desktop pixels).
    resizing: Option<(playlist::Size, (i32, i32))>,
    /// The balance slider, -1 to 1: this window's alone, as the page has none.
    balance: f64,
    remaining: bool,
    marquee: Marquee,
    notice: Option<(String, Instant)>,
    /// The row a selection by Shift-click runs from, and the row a drag of songs is over.
    anchor: usize,
    dropping: Option<usize>,
    /// When the song was paused, for the time's blink.
    paused_since: Option<Instant>,
    tick: Duration,
}

/// What the platform's window does once the stack has dealt with the listener's input.
pub enum Effect {
    /// Not the stack's: the platform's own handling.
    Default,
    Handled,
    /// The listener pressed on a window's own body where the windows can't be placed: the
    /// platform moves it.
    Move,
    /// The pointer is held on a control: its moves and release come here even off the window.
    Capture,
    Release,
    Minimize,
    Close,
    /// A popup menu with its top left at this point of the window (its own pixels).
    Menu(Menu, (i32, i32)),
}

/// What the platform's windows do after a frame: each shows or hides, goes to its place, takes a
/// new size and a new shape, puts its changed part on screen, in this order; the timer takes a
/// new pace.
#[derive(Default)]
pub struct Frame {
    pub panes: [PaneFrame; 3],
    pub tick: Option<Duration>,
}

/// One window's part of a frame.
#[derive(Default)]
pub struct PaneFrame {
    pub shown: bool,
    /// Its top left on the desktop, physical pixels.
    pub at: (i32, i32),
    /// Physical pixels.
    pub size: Option<(i32, i32)>,
    pub shape: Option<Shape>,
    /// Skin pixels `[x0, x1) x [y0, y1)` of its canvas.
    pub dirty: Option<(u32, u32, u32, u32)>,
}

/// Rectangles of physical pixels `(x0, y0, x1, y1)` a window is cut to; none: the whole window.
pub type Shape = Option<Vec<(i32, i32, i32, i32)>>;

/// The keys the stack knows, as any platform names them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    /// A letter, upper case.
    Letter(u8),
    Left,
    Right,
    Up,
    Down,
    Delete,
    /// The numeric keypad's digit.
    Pad(u8),
}

/// The modifier keys held.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mods {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A menu's line: a command (its id, words, ticked or not), a separator, or a submenu.
pub enum Item {
    Command(u32, String, bool),
    Separator,
    Submenu(String, Vec<Item>),
}

impl Classic {
    /// The stack as Classic opens: Webamp's three windows, the playing song in sight.
    pub fn new(setup: Setup, on_close: OnClose) -> Self {
        let panels = Panels {
            main: true,
            eq: true,
            playlist: true,
            main_shade: false,
            eq_shade: false,
            playlist_shade: false,
            playlist_size: playlist::Size::default(),
        };
        // Stacked as Webamp opens them: the equaliser under the main window, the playlist under it.
        let scale = setup.size * setup.dpi;
        let (x, y) = setup.at;
        let height = edges(HEIGHT, scale).last().copied().unwrap_or(0);
        let mut classic = Classic {
            skin: setup.skin,
            session: setup.session,
            on_close: Some(on_close),
            favourites: setup.favourites,
            panes: [Pane::new(), Pane::new(), Pane::new()],
            places: [(x, y), (x, y + height), (x, y + 2 * height)],
            laid_out: None,
            places_windows: true,
            areas: Vec::new(),
            drag: None,
            snapshot: Snapshot::default(),
            revision: None,
            size: setup.size,
            dpi: setup.dpi,
            panels,
            list: List::default(),
            analyser: Analyser::default(),
            bars: [Bar::default(); BARS],
            focus: Focus::Main,
            pressed: None,
            over: false,
            seeking: None,
            resizing: None,
            balance: 0.0,
            remaining: false,
            marquee: Marquee::default(),
            notice: setup.said.map(|said| (said, Instant::now())),
            anchor: 0,
            dropping: None,
            paused_since: None,
            // None yet: the first frame sets the timer going.
            tick: Duration::ZERO,
        };
        classic.snapshot = classic.session.snapshot();
        classic.relist();
        let index = classic.snapshot.index;
        classic.list.show(index, classic.panels.playlist_size);
        classic
    }

    /// The widest window's physical width, for a band a window's rows are drawn in.
    pub fn widest(&self) -> i32 {
        self.panes.iter().map(|p| p.physical().0).max().unwrap_or(0).max(1)
    }

    /// A window as drawn: its canvas and where its pixels fall.
    pub fn pane(&self, panel: Panel) -> &Pane {
        &self.panes[panel.index()]
    }

    /// The control under a point of a window (skin pixels); `None` is the window itself, which
    /// drags.
    fn target_at(&self, panel: Panel, x: u32, y: u32) -> Option<Target> {
        let shaded = self.panels.shaded(panel);
        match panel {
            Panel::Main => draw::main_at(x, y, shaded),
            Panel::Eq => eqwin::at(x, y, shaded),
            Panel::Playlist => playlist::at(x, y, shaded, &self.list, self.panels.playlist_size),
        }
    }

    /// The left button down on a window at `at` (its own pixels; `screen`, the desktop's), twice
    /// for a double click.
    pub fn pointer_down(
        &mut self,
        panel: Panel,
        at: (i32, i32),
        screen: (i32, i32),
        double: bool,
        mods: Mods,
    ) -> Effect {
        let (x, y) = self.pane(panel).skin_point(at);
        let target = self.target_at(panel, x, y);
        // A press anywhere but on the open menu closes it, as a click outside Webamp's does.
        let control = target.map(|t| t.control);
        if !matches!(control, Some(Control::PlaylistMenuItem(..)))
            && control != self.list.menu.map(Control::PlaylistMenu)
        {
            self.list.menu = None;
            self.list.hover = None;
        }
        self.focus = panel.focus();
        let Some(target) = target else {
            if double {
                return self.double_click_title(panel, x, y);
            }
            if !self.places_windows {
                return Effect::Move;
            }
            // The main window brings what is docked to it; the others come away alone.
            let windows = self.rects();
            let moving = match panel {
                Panel::Main => dock::docked_set(&windows, panel.index()),
                _ => vec![panel.index()],
            };
            self.drag = Some(Drag {
                moving,
                from: screen,
                places: self.places,
            });
            return Effect::Capture;
        };
        match target.control {
            Control::Time => {
                self.remaining = !self.remaining;
                return Effect::Handled;
            }
            Control::PlaylistResize => {
                self.resizing = Some((self.panels.playlist_size, screen));
                self.pressed = Some((target, panel));
                return Effect::Capture;
            }
            Control::PlaylistRow(row) => {
                let effect = self.click_row(row, double, mods);
                // A single press on a song may drag the selected songs elsewhere in the list.
                if !double && row < self.list.labels.len() {
                    self.pressed = Some((target, panel));
                    self.dropping = Some(row);
                    return Effect::Capture;
                }
                return effect;
            }
            _ => {}
        }
        self.pressed = Some((target, panel));
        self.over = true;
        self.drag(x, y);
        Effect::Capture
    }

    /// A double click the platform reports on a window's title bar (it began a move first: where
    /// windows can't be placed).
    #[cfg(target_os = "linux")]
    pub fn title_double_click(&mut self, panel: Panel, at: (i32, i32)) -> Effect {
        let (x, y) = self.pane(panel).skin_point(at);
        self.double_click_title(panel, x, y)
    }

    /// The pointer moved over a window to `at` (its own pixels; `screen`, the desktop's): dragged
    /// windows go with it, a held control follows it, the open menu lights what it is over, the
    /// playlist's corner resizes.
    pub fn pointer_move(&mut self, panel: Panel, at: (i32, i32), screen: (i32, i32)) {
        if self.drag.is_some() {
            return self.move_windows(screen);
        }
        if self.resizing.is_some() {
            return self.resize_playlist(screen);
        }
        let (x, y) = self.pane(panel).skin_point(at);
        if self.pressed.is_some() {
            self.drag(x, y);
        }
        if self.list.menu.is_some() {
            self.hover_menu(panel, x, y);
        }
    }

    /// The left button up over a window at `at` (its own pixels; `screen`, the desktop's).
    pub fn pointer_up(&mut self, panel: Panel, at: (i32, i32), screen: (i32, i32)) -> Effect {
        if self.drag.is_some() {
            self.move_windows(screen);
            self.drag = None;
            return Effect::Release;
        }
        if self.resizing.is_some() {
            self.resize_playlist(screen);
            self.resizing = None;
            self.pressed = None;
            return Effect::Release;
        }
        if self.pressed.is_none() {
            return Effect::Default;
        }
        let (x, y) = self.pane(panel).skin_point(at);
        self.drag(x, y);
        self.release()
    }

    /// The dragged windows where the pointer has taken them, snapped to the others and to the work
    /// areas' edges.
    fn move_windows(&mut self, screen: (i32, i32)) {
        let Some(drag) = &self.drag else {
            return;
        };
        let by = (screen.0 - drag.from.0, screen.1 - drag.from.1);
        let mut windows = self.rects();
        for (i, window) in windows.iter_mut().enumerate() {
            if let Some(w) = window {
                (w.x, w.y) = drag.places[i];
            }
        }
        let (dx, dy) = dock::dragged(&windows, &drag.moving, by, &self.areas, dock::snap_distance(self.dpi));
        for &i in &drag.moving {
            self.places[i] = (drag.places[i].0 + dx, drag.places[i].1 + dy);
        }
    }

    /// The windows that show, where they are now (closed ones `None`), in `Panel` order.
    fn rects(&self) -> Vec<Option<dock::Rect>> {
        Panel::ALL
            .iter()
            .map(|&panel| {
                let (width, height) = self.pane(panel).physical();
                let (x, y) = self.places[panel.index()];
                self.panels
                    .showing(panel)
                    .then_some(dock::Rect::new(x, y, width, height))
            })
            .collect()
    }

    /// The desktop's monitors' work areas changed (or are first known): windows left off every
    /// one come back, the main window's docked set together.
    pub fn set_areas(&mut self, areas: Vec<dock::Rect>) {
        self.areas = areas;
        let windows = self.rects();
        let together = dock::docked_set(&windows, Panel::Main.index());
        for (i, (dx, dy)) in dock::on_screen(&windows, &together, &self.areas) {
            self.places[i].0 += dx;
            self.places[i].1 += dy;
        }
    }

    /// The right button: the Options menu there.
    pub fn right_click(&self, at: (i32, i32)) -> Effect {
        Effect::Menu(Menu::Options, at)
    }

    /// A double click on a window's title bar rolls it up or down (Winamp's shade).
    fn double_click_title(&mut self, panel: Panel, x: u32, y: u32) -> Effect {
        let target = self.target_at(panel, x, y);
        let title = if panel == Panel::Playlist && !self.panels.playlist_shade {
            layout::PLAYLIST_TITLE_HEIGHT
        } else {
            layout::SHADE_HEIGHT
        };
        if target.is_none() && y < title {
            self.toggle_shade(panel);
        }
        Effect::Handled
    }

    /// A window on or off, but never the last one: with nothing showing, nothing could be clicked.
    /// The selected window, closed, gives way to the first that shows.
    fn toggle_window(&mut self, panel: Panel) {
        let open = !*self.panels.shown(panel);
        *self.panels.shown(panel) = open;
        let showing: Vec<Panel> = Panel::ALL.into_iter().filter(|&p| self.panels.showing(p)).collect();
        let Some(first) = showing.first() else {
            *self.panels.shown(panel) = true;
            return;
        };
        if !showing.iter().any(|p| p.focus() == self.focus) {
            self.focus = first.focus();
        }
    }

    fn toggle_shade(&mut self, panel: Panel) {
        let p = &mut self.panels;
        match panel {
            Panel::Main => p.main_shade = !p.main_shade,
            Panel::Eq => p.eq_shade = !p.eq_shade,
            Panel::Playlist => p.playlist_shade = !p.playlist_shade,
        }
    }

    /// A click on the playlist: selects the row (Ctrl adds or takes it, Shift runs from the last);
    /// a double click plays it.
    fn click_row(&mut self, row: usize, double: bool, mods: Mods) -> Effect {
        if row >= self.list.labels.len() {
            self.list.selected.fill(false);
            return Effect::Handled;
        }
        if double {
            self.send(Command::Jump(row));
            return Effect::Handled;
        }
        if mods.control {
            self.list.selected[row] = !self.list.selected[row];
        } else {
            let (from, to) = if mods.shift {
                (self.anchor.min(row), self.anchor.max(row))
            } else {
                (row, row)
            };
            for (i, selected) in self.list.selected.iter_mut().enumerate() {
                *selected = (from..=to).contains(&i);
            }
        }
        self.anchor = row;
        Effect::Handled
    }

    /// The held control follows the pointer: a slider takes its value, a button looks pressed while
    /// the pointer is on it.
    fn drag(&mut self, x: u32, y: u32) {
        let Some((target, _)) = self.pressed else {
            return;
        };
        let area = target.area;
        let (x, y) = (f64::from(x), f64::from(y));
        let along = |at: f64, start: u32| {
            let travel = f64::from(target.travel.max(1));
            ((at - f64::from(start) - f64::from(target.knob) / 2.0) / travel).clamp(0.0, 1.0)
        };
        let across = along(x, area.x);
        let down = along(y, area.y);
        match target.control {
            Control::Volume => self.volume(across),
            Control::Balance => {
                self.balance = centred(across * 2.0 - 1.0);
                self.send(Command::Balance(self.balance));
            }
            Control::Position => self.seeking = Some(across),
            Control::EqPreamp => self.equalise(|eq| eq.preamp = Some(eqwin::decibels(1.0 - down))),
            Control::EqBand(band) => self.equalise(|eq| eq.bands[band] = eqwin::decibels(1.0 - down)),
            Control::PlaylistRow(_) => {
                self.dropping = Some(playlist::row_at(
                    y.max(0.0) as u32,
                    &self.list,
                    self.panels.playlist_size,
                ))
            }
            Control::PlaylistScroll => {
                let size = self.panels.playlist_size;
                let room = self.list.labels.len().saturating_sub(size.rows());
                self.list.scroll_to((down * room as f64).round() as usize, size);
            }
            _ => {
                self.over = x >= f64::from(area.x)
                    && y >= f64::from(area.y)
                    && x < f64::from(area.x + area.width)
                    && y < f64::from(area.y + area.height);
            }
        }
    }

    fn release(&mut self) -> Effect {
        let Some((target, panel)) = self.pressed.take() else {
            return Effect::Release;
        };
        if let (Control::PlaylistRow(grabbed), Some(dropped)) = (target.control, self.dropping.take()) {
            if dropped != grabbed {
                let rows: Vec<usize> = (0..self.list.selected.len())
                    .filter(|&i| self.list.selected[i])
                    .collect();
                let to = if dropped > grabbed { dropped + 1 } else { dropped };
                self.send(Command::Reorder(rows, to.min(self.list.labels.len())));
            }
            return Effect::Release;
        }
        if target.control == Control::Position {
            if let (Some(fraction), Some(length)) = (self.seeking.take(), self.snapshot.duration) {
                self.send(Command::Seek(fraction * length));
            }
        } else if self.over {
            return self.click(target, panel);
        }
        Effect::Release
    }

    /// A button, clicked: Winamp's own meaning for each.
    fn click(&mut self, target: Target, panel: Panel) -> Effect {
        let now = &self.snapshot;
        let area = target.area;
        let pane = self.pane(panel);
        let below = (
            pane.columns[area.x as usize],
            pane.rows[((area.y + area.height) as usize).min(pane.rows.len() - 1)],
        );
        if target.control == Control::Marquee && matches!(now.status, Status::Choose(_)) {
            return Effect::Menu(Menu::Choose, below);
        }
        if let Some(command) = self.transport(target.control) {
            self.send(command);
            return Effect::Release;
        }
        match target.control {
            Control::Minimize => return Effect::Minimize,
            Control::Close => return Effect::Close,
            Control::Shade => self.toggle_shade(Panel::Main),
            Control::EqShade => self.toggle_shade(Panel::Eq),
            Control::PlaylistShade => self.toggle_shade(Panel::Playlist),
            Control::EqWindow | Control::EqClose => self.toggle_window(Panel::Eq),
            Control::PlaylistWindow | Control::PlaylistClose => self.toggle_window(Panel::Playlist),
            Control::EqOn => self.equalise(|eq| eq.on = !eq.on),
            Control::EqAuto => self.equalise(|eq| {
                eq.preamp = match eq.preamp {
                    Some(_) => None,
                    None => Some(eq.level()),
                }
            }),
            Control::Options => return Effect::Menu(Menu::Options, below),
            Control::EqPresets => return Effect::Menu(Menu::Presets, below),
            Control::PlaylistMenu(menu) => {
                self.list.menu = if self.list.menu == Some(menu) { None } else { Some(menu) };
                self.list.hover = None;
            }
            Control::PlaylistMenuItem(menu, item) => {
                self.list.menu = None;
                self.list.hover = None;
                return self.menu_button(menu, item, below);
            }
            _ => {}
        }
        Effect::Release
    }

    /// What the marquee says while a slider is held, as Webamp's does.
    fn drag_line(&self) -> Option<String> {
        let (target, _) = self.pressed?;
        let eq = &self.snapshot.eq;
        Some(match target.control {
            Control::Position => seek_line(self.seeking?, self.snapshot.duration.unwrap_or(0.0)),
            Control::Volume => format!("Volume: {}%", (self.snapshot.volume * 100.0).round()),
            Control::Balance => balance_line(self.balance),
            Control::EqPreamp => eq_line(None, eq.level()),
            Control::EqBand(band) => eq_line(Some(band), eq.bands[band]),
            _ => return None,
        })
    }

    /// The transport's buttons, shuffle and repeat, Winamp's meaning for each (the keys' too).
    fn transport(&self, control: Control) -> Option<Command> {
        let now = &self.snapshot;
        Some(match control {
            Control::Previous => Command::Previous,
            // Play on a playing song starts it again.
            Control::Play if now.playing => Command::Seek(0.0),
            Control::Play => Command::Play,
            Control::Pause if now.playing => Command::Pause,
            Control::Pause if !now.stopped => Command::Play,
            Control::Stop => Command::Stop,
            Control::Next => Command::Next,
            Control::Shuffle => Command::Shuffle(!now.shuffle),
            Control::Repeat => Command::CycleRepeat,
            _ => return None,
        })
    }

    pub fn menu_items(&self, menu: Menu) -> Vec<Item> {
        match menu {
            Menu::Options => {
                let mut skins = vec![Item::Command(99, "Webamp's base skin".into(), false)];
                if !self.favourites.is_empty() {
                    skins.push(Item::Separator);
                }
                skins.extend(
                    self.favourites
                        .iter()
                        .enumerate()
                        .map(|(i, (name, _))| Item::Command(100 + i as u32, name.clone(), false)),
                );
                vec![
                    Item::Submenu("Skins".into(), skins),
                    Item::Command(2, "Double size\tCtrl+D".into(), self.size >= 2.0),
                    Item::Command(3, "Time remaining".into(), self.remaining),
                    Item::Separator,
                    Item::Command(1, "Back to lull".into(), false),
                ]
            }
            Menu::Presets => presets()
                .iter()
                .enumerate()
                .map(|(i, (name, bands))| Item::Command(1 + i as u32, (*name).into(), self.snapshot.eq.bands == *bands))
                .collect(),
            Menu::Choose => {
                let choices = match &self.snapshot.status {
                    Status::Choose(choices) => choices.clone(),
                    _ => Vec::new(),
                };
                let mut items: Vec<Item> = choices
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let text = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
                        Item::Command(1 + i as u32, format!("{}  ({})", text("format"), text("peer")), false)
                    })
                    .collect();
                items.extend([Item::Separator, Item::Command(99, "Skip this song".into(), false)]);
                items
            }
            Menu::Sort => vec![
                Item::Command(1, "Sort list by title".into(), false),
                Item::Separator,
                Item::Command(2, "Reverse list".into(), false),
                Item::Command(3, "Randomize list".into(), false),
            ],
            Menu::MiscOptions => vec![Item::Command(1, "Generate HTML playlist".into(), false)],
        }
    }

    /// What the listener chose from a menu.
    pub fn chose(&mut self, menu: Menu, id: u32) -> Effect {
        match (menu, id) {
            (Menu::Options, 1) => return Effect::Close,
            (Menu::Options, 2) => self.double_size(),
            (Menu::Options, 3) => self.remaining = !self.remaining,
            (Menu::Options, 99) => self.change_skin(super::BASE_SKIN),
            (Menu::Options, id) => {
                if let Some((_, url)) = self.favourites.get(id.saturating_sub(100) as usize) {
                    let url = url.clone();
                    self.change_skin(&url);
                }
            }
            (Menu::Presets, id) => {
                if let Some((_, bands)) = presets().get(id.saturating_sub(1) as usize) {
                    self.equalise(|eq| *eq = crate::desktop::classic::Equaliser::with_preset(*bands));
                }
            }
            (Menu::Choose, 99) => self.send(Command::Next),
            (Menu::Choose, id) => {
                if let Status::Choose(choices) = &self.snapshot.status
                    && let Some(choice) = choices.get(id.saturating_sub(1) as usize)
                {
                    self.send(Command::Choose(choice.clone()));
                }
            }
            (Menu::Sort, 1) => {
                let order = self.session.by_title(&self.snapshot);
                self.send(Command::Order(order));
            }
            (Menu::Sort, 2) => self.reverse(),
            (Menu::Sort, _) => self.send(Command::Randomize),
            (Menu::MiscOptions, _) => self.say(NOT_IN_CLASSIC),
        }
        Effect::Handled
    }

    /// Another skin from the Options menu, fetched off the main thread (set_skin puts it on).
    fn change_skin(&mut self, url: &str) {
        let Ok(url) = self.session.origin().join(url) else {
            return;
        };
        self.notice = Some(("Loading the skin...".into(), Instant::now()));
        super::change_skin(
            self.session.app(),
            self.session.origin().clone(),
            self.session.token().to_owned(),
            url.to_string(),
        );
    }

    /// Winamp's Double Size: 2x, or 1x from there; lull opens Classic at it next time too.
    fn double_size(&mut self) {
        self.size = if self.size >= 2.0 { 1.0 } else { 2.0 };
    }

    /// A button of an open playlist menu, Webamp's meaning for each. What lull's queue can't do
    /// (files from this computer, lists kept as files, a file's tags) says so in the marquee, as
    /// Webamp's own missing ones say so in a note.
    fn menu_button(&mut self, menu: usize, item: usize, below: (i32, i32)) -> Effect {
        let select = |list: &mut List, how: fn(bool) -> bool| {
            for selected in &mut list.selected {
                *selected = how(*selected);
            }
        };
        match (menu, item) {
            // ADD: a URL, a folder, a file.
            (0, _) => self.say(ADD_IN_LULL),
            // REM: misc, all, crop, selected.
            (1, 0) | (3, 1) => self.say(NOT_IN_CLASSIC),
            (1, 1) | (4, 0) => self.keep(&vec![false; self.list.labels.len()]),
            (1, 2) => {
                let keep = self.list.selected.clone();
                self.keep(&keep);
            }
            (1, _) => self.remove_selected(),
            // SEL: invert, none, all.
            (2, 0) => select(&mut self.list, |s| !s),
            (2, 1) => select(&mut self.list, |_| false),
            (2, _) => select(&mut self.list, |_| true),
            // MISC: sort, file info, options.
            (3, 0) => return Effect::Menu(Menu::Sort, below),
            (3, _) => return Effect::Menu(Menu::MiscOptions, below),
            // LIST: new (above), save, load.
            _ => self.say(LISTS_IN_LULL),
        }
        Effect::Release
    }

    /// The pointer over an open playlist menu lights the button under it.
    fn hover_menu(&mut self, panel: Panel, x: u32, y: u32) {
        let open = self.list.menu;
        self.list.hover = match (panel, self.target_at(panel, x, y)) {
            (
                Panel::Playlist,
                Some(Target {
                    control: Control::PlaylistMenuItem(menu, item),
                    ..
                }),
            ) if Some(menu) == open => Some(item),
            _ => None,
        };
    }

    /// The playlist's corner, dragged: Webamp's whole steps from where it was grabbed, counted in
    /// skin pixels from the pointer itself, so the window can grow past its edge. Rolled up, it
    /// only widens.
    fn resize_playlist(&mut self, (x, y): (i32, i32)) {
        let Some((from, (x0, y0))) = self.resizing else {
            return;
        };
        let scale = self.size * self.dpi;
        let mut size = from.dragged(f64::from(x - x0) / scale, f64::from(y - y0) / scale);
        if self.panels.playlist_shade {
            size.tall = from.tall;
        }
        self.panels.playlist_size = size;
        let scroll = self.list.scroll;
        self.list.scroll_to(scroll, size);
    }

    /// A line in the marquee for a few seconds.
    fn say(&mut self, line: &str) {
        self.notice = Some((line.to_owned(), Instant::now()));
    }

    /// Webamp's Reverse list (Ctrl+R too).
    fn reverse(&mut self) {
        let order: Vec<usize> = (0..self.list.labels.len()).rev().collect();
        self.send(Command::Order(order));
    }

    fn remove_selected(&mut self) {
        let keep: Vec<bool> = self.list.selected.iter().map(|s| !s).collect();
        self.keep(&keep);
    }

    fn keep(&mut self, keep: &[bool]) {
        self.send(Command::Keep(keep.to_vec()));
    }

    fn send(&self, command: Command) {
        self.session.engine.send(command);
    }

    /// The volume, at once on screen too.
    fn volume(&mut self, volume: f64) {
        self.snapshot.volume = volume.clamp(0.0, 1.0);
        self.send(Command::Volume(volume));
    }

    /// A change to the equaliser, at once on screen too.
    fn equalise(&mut self, change: impl FnOnce(&mut crate::desktop::classic::Equaliser)) {
        change(&mut self.snapshot.eq);
        self.snapshot.eq = self.snapshot.eq.clone().normalised();
        self.send(Command::Equaliser(self.snapshot.eq.clone()));
    }

    /// The playlist's lines again, after the queue changed.
    fn relist(&mut self) {
        let (labels, lengths) = self.session.lines(&self.snapshot);
        self.list.selected = vec![false; labels.len()];
        self.list.labels = labels;
        self.list.lengths = lengths;
        let scroll = self.list.scroll;
        self.list.scroll_to(scroll, self.panels.playlist_size);
    }

    /// The wheel, turned `notches` over a window: over the playlist it scrolls it, else it turns
    /// the volume.
    pub fn wheel(&mut self, panel: Panel, notches: f64) -> Effect {
        if panel == Panel::Playlist {
            let rows = (-notches * WHEEL_ROWS).round() as isize;
            let first = self.list.scroll.saturating_add_signed(rows);
            self.list.scroll_to(first, self.panels.playlist_size);
        } else {
            self.volume(self.snapshot.volume + 0.05 * notches);
        }
        Effect::Handled
    }

    /// Webamp's keys (its hotkeys, the numeric keypad's too): Z X C V B and the keypad's 4 5 6 for
    /// the transport, R and S for repeat and shuffle, arrows (keypad 7 9 8 2) for 5 seconds and the
    /// volume, keypad 1 and 3 for ten songs back or on; Ctrl+D Double Size, Ctrl+R the list
    /// reversed, Ctrl+T the time counting down; Alt+W, Alt+G and Alt+E a window on or off. Delete
    /// takes the selected songs out.
    pub fn key(&mut self, key: Key, mods: Mods) -> Effect {
        let (position, volume) = (self.snapshot.position, self.snapshot.volume);
        let is = |c: u8| key == Key::Letter(c);
        let pad = |n: u8| key == Key::Pad(n);
        if mods.alt {
            let panel = match key {
                _ if is(b'W') => Panel::Main,
                _ if is(b'G') => Panel::Eq,
                _ if is(b'E') => Panel::Playlist,
                _ => return Effect::Default,
            };
            self.toggle_window(panel);
            return Effect::Handled;
        }
        if mods.control {
            match key {
                _ if is(b'D') => self.double_size(),
                _ if is(b'R') => self.reverse(),
                _ if is(b'T') => self.remaining = !self.remaining,
                _ => return Effect::Default,
            }
            return Effect::Handled;
        }
        let command = match key {
            _ if is(b'Z') || pad(4) => self.transport(Control::Previous),
            _ if is(b'X') || pad(5) => self.transport(Control::Play),
            _ if is(b'C') => self.transport(Control::Pause),
            _ if is(b'V') => self.transport(Control::Stop),
            _ if is(b'B') || pad(6) => self.transport(Control::Next),
            _ if is(b'R') => self.transport(Control::Repeat),
            _ if is(b'S') => self.transport(Control::Shuffle),
            _ if pad(1) => self.skip(-10),
            _ if pad(3) => self.skip(10),
            Key::Left | Key::Pad(7) => Some(Command::Seek((position - 5.0).max(0.0))),
            Key::Right | Key::Pad(9) => Some(Command::Seek(position + 5.0)),
            Key::Up | Key::Pad(8) => {
                self.volume(volume + 0.01);
                None
            }
            Key::Down | Key::Pad(2) => {
                self.volume(volume - 0.01);
                None
            }
            Key::Delete => {
                self.remove_selected();
                None
            }
            _ => return Effect::Default,
        };
        if let Some(command) = command {
            self.send(command);
        }
        Effect::Handled
    }

    /// Ten songs on or back (the keypad's 3 and 1).
    fn skip(&self, songs: isize) -> Option<Command> {
        let (index, count) = (self.snapshot.index, self.snapshot.queue.len());
        skipped(index, count, self.snapshot.repeat != Repeat::Off, songs).map(Command::Jump)
    }

    /// Draws a frame at 1x and says what the window does about it: its size and shape if they
    /// changed, the part to put on screen, and the timer's pace (fast while something moves, slow
    /// while only the title or a blink does).
    pub fn render(&mut self) -> Frame {
        let mut frame = Frame::default();
        self.snapshot = self.session.snapshot();
        self.session.tick(&self.snapshot);
        if self.revision != Some(self.snapshot.revision) {
            self.revision = Some(self.snapshot.revision);
            self.relist();
        }
        let now = self.session.now(&self.snapshot);
        let clock = Instant::now();
        if now.stopped {
            self.analyser = Analyser::default();
            self.bars = [Bar::default(); BARS];
        } else if now.playing {
            let samples = self.session.engine.tap(FFT_SAMPLES);
            self.bars = self.analyser.step(&samples, clock);
        }
        let bars = self.bars;
        self.notice = self.notice.take().filter(|(_, at)| at.elapsed() < NOTICE);
        let line = match (
            self.drag_line(),
            &now.said,
            &self.notice,
            self.session.titles.get(now.index),
        ) {
            (Some(line), _, _, _) => line,
            (None, Some(error), _, _) => error.clone(),
            (None, None, Some((notice, _)), _) => notice.clone(),
            (None, None, None, Some(title)) => title.clone(),
            (None, None, None, None) => String::new(),
        };
        let marquee = self.marquee.show(&line, clock);
        // Paused, the time blinks as Webamp's does: on for the first second, then off in odd
        // seconds and on in even ones.
        if now.playing || now.stopped {
            self.paused_since = None;
        } else {
            self.paused_since.get_or_insert(clock);
        }
        let blink_off = self
            .paused_since
            .is_some_and(|since| (clock - since).as_secs() % 2 == 1);
        self.lay_out(&mut frame);
        let view = View {
            now: &now,
            marquee: &marquee,
            volume: self.snapshot.volume,
            balance: self.balance,
            focus: self.focus,
            pressed: self.pressed.filter(|_| self.over).map(|(t, _)| t.control),
            seeking: self.seeking,
            remaining: self.remaining,
            blink_off,
            bars: &bars,
            shuffle: self.snapshot.shuffle,
            repeat: self.snapshot.repeat != Repeat::Off,
            main_open: self.panels.main,
            eq_open: self.panels.eq,
            playlist_open: self.panels.playlist,
            eq: &self.snapshot.eq,
            list: &self.list,
            playlist: self.panels.playlist_size,
        };
        for panel in Panel::ALL {
            let i = panel.index();
            let out = &mut frame.panes[i];
            out.shown = self.panels.showing(panel);
            out.at = self.places[i];
            if !out.shown {
                continue;
            }
            let shaded = self.panels.shaded(panel);
            let print = Some(fingerprint(panel, shaded, &view));
            let pane = &mut self.panes[i];
            let drawn = pane.shown.is_empty() || pane.print != print;
            if drawn {
                pane.print = print;
                draw_panel(&mut pane.canvas, &self.skin, (panel, shaded), &view);
            }
            if !pane.shaped {
                pane.shaped = true;
                out.shape = Some(cut(pane));
            }
            let (width, height) = (pane.canvas.width, pane.canvas.height);
            let pixels = drawn
                .then(|| changed(&pane.canvas.pixels, &pane.shown, width, (0, height)))
                .flatten();
            out.dirty = pixels
                .into_iter()
                .chain(texts_changed(&pane.canvas.texts, &pane.shown_texts))
                .reduce(union);
            if let Some(dirty) = out.dirty {
                if pane.shown.len() == pane.canvas.pixels.len() {
                    let (from, to) = ((dirty.1 * width) as usize, (dirty.3 * width) as usize);
                    pane.shown[from..to].copy_from_slice(&pane.canvas.pixels[from..to]);
                } else {
                    pane.shown.clone_from(&pane.canvas.pixels);
                }
                pane.shown_texts.clone_from(&pane.canvas.texts);
            }
        }
        let moving = now.playing || self.seeking.is_some() || self.pressed.is_some() || self.drag.is_some();
        let tick = if moving { ANIMATE } else { IDLE };
        if tick != self.tick {
            self.tick = tick;
            frame.tick = Some(tick);
        }
        frame
    }

    /// Each window at its size now and at the scale now: one that changed size is laid out again
    /// (and all of it drawn), what is docked to its bottom and right moving with the change; a new
    /// scale keeps the windows where they were around the main window.
    fn lay_out(&mut self, frame: &mut Frame) {
        let scale = self.size * self.dpi;
        let sizes = Panel::ALL.map(|panel| self.panels.size(panel));
        let before = self.laid_out.replace((scale, sizes));
        let rects = self.rects();
        let mut grown = Vec::new();
        for (i, &(width, height)) in sizes.iter().enumerate() {
            if before.is_some_and(|(s, old)| s == scale && old[i] == (width, height)) {
                continue;
            }
            let pane = &mut self.panes[i];
            pane.canvas.resize(width, height);
            pane.columns = edges(width, scale);
            pane.rows = edges(height, scale);
            pane.forget();
            frame.panes[i].size = Some(pane.physical());
            grown.push(i);
        }
        match before {
            Some((old, _)) if old != scale => {
                let main = self.places[Panel::Main.index()];
                let at = |p: i32, from: i32| from + (f64::from(p - from) * scale / old).round() as i32;
                for place in &mut self.places {
                    *place = (at(place.0, main.0), at(place.1, main.1));
                }
            }
            Some(_) => {
                for i in grown {
                    let Some(was) = rects[i] else {
                        continue;
                    };
                    for (j, (dx, dy)) in dock::resized(&self.rects(), i, was) {
                        self.places[j].0 += dx;
                        self.places[j].1 += dy;
                    }
                }
            }
            None => {}
        }
    }

    /// The skin pixels a part of a window `(x0, y0, x1, y1)` (its own pixels, end exclusive)
    /// shows, for the platform to put on screen again; none before its first frame.
    pub fn covering(&self, panel: Panel, (x0, y0, x1, y1): (i32, i32, i32, i32)) -> Option<(u32, u32, u32, u32)> {
        let pane = self.pane(panel);
        if pane.shown.is_empty() || x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (sx0, sy0) = pane.skin_point((x0, y0));
        let (sx1, sy1) = pane.skin_point((x1 - 1, y1 - 1));
        let (sx1, sy1) = ((sx1 + 1).min(pane.canvas.width), (sy1 + 1).min(pane.canvas.height));
        (sx1 > sx0 && sy1 > sy0).then_some((sx0, sy0, sx1, sy1))
    }

    /// The font and scale the playlist's text is drawn in: the platform makes its text again when
    /// they change.
    pub fn text_style(&self) -> (&str, f64) {
        (&self.skin.playlist.font, self.size * self.dpi)
    }

    /// The tray's and the taskbar's buttons, the system's media keys.
    pub fn act(&mut self, action: &str) {
        self.session.act(action);
    }

    /// Another skin, fetched for the Options menu; or why it couldn't be.
    pub fn set_skin(&mut self, skin: Result<(Skin, Option<String>), String>) {
        match skin {
            Ok((skin, said)) => {
                self.skin = skin;
                self.notice = said.map(|said| (said, Instant::now()));
                for pane in &mut self.panes {
                    pane.forget();
                }
            }
            Err(why) => self.notice = Some((format!("Couldn't load the skin: {why}"), Instant::now())),
        }
    }
}

/// The song `songs` on or back from `index` in a list of `count`, as Webamp's keypad goes: round
/// the list with repeat on, else no further than its ends, and none from the last song on or the
/// first back.
fn skipped(index: usize, count: usize, round: bool, songs: isize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    let to = index as isize + songs;
    if round {
        Some(to.rem_euclid(count as isize) as usize)
    } else if (index == last && songs > 0) || (index == 0 && songs < 0) {
        None
    } else {
        Some(to.clamp(0, last as isize) as usize)
    }
}

/// A window (rolled up or not) into its canvas.
fn draw_panel(canvas: &mut Canvas, skin: &Skin, (panel, shaded): (Panel, bool), view: &View) {
    canvas.clear(0, canvas.height);
    let mut pen = canvas.pen(skin, 0);
    match (panel, shaded) {
        (Panel::Main, false) => draw::main(&mut pen, view),
        (Panel::Main, true) => draw::main_shaded(&mut pen, view),
        (Panel::Eq, false) => eqwin::window(&mut pen, view),
        (Panel::Eq, true) => eqwin::shaded(&mut pen, view),
        (Panel::Playlist, false) => playlist::window(&mut pen, view),
        (Panel::Playlist, true) => playlist::shaded(&mut pen, view),
    }
}

/// For each of `count` skin pixels and its end, the first physical pixel it covers at `scale`.
pub fn edges(count: u32, scale: f64) -> Vec<i32> {
    (0..=count).map(|i| (f64::from(i) * scale).ceil() as i32).collect()
}

/// Skin pixels `[x0, x1)` of the canvas's rows that physical rows `from..to` show, scaled up
/// nearest neighbour into `pixels`: rows `stride` long, from physical row `from` and column
/// `columns[x0]`.
pub fn scale_rows(
    canvas: &Canvas,
    (columns, rows): (&[i32], &[i32]),
    (x0, x1): (u32, u32),
    (from, to): (i32, i32),
    pixels: &mut [u32],
    stride: usize,
) {
    let left = columns[x0 as usize];
    let width = canvas.width as usize;
    for py in from..to {
        let sy = skin_at(py, rows) as usize;
        let source = &canvas.pixels[sy * width..(sy + 1) * width];
        let at = (py - from) as usize * stride;
        let Some(row) = pixels.get_mut(at..at + stride) else {
            return;
        };
        for sx in x0..x1 {
            let (a, b) = (
                (columns[sx as usize] - left) as usize,
                (columns[sx as usize + 1] - left) as usize,
            );
            row[a.min(stride)..b.min(stride)].fill(source[sx as usize]);
        }
    }
}

/// The skin pixel a physical coordinate falls in: `edges` holds, for each skin pixel, the first
/// physical one it covers.
pub fn skin_at(physical: i32, edges: &[i32]) -> u32 {
    edges.partition_point(|&e| e <= physical).saturating_sub(1) as u32
}

/// A window cut to the skin's shapes as drawn (outside them its canvas is transparent), as
/// rectangles of physical pixels; a window drawn with no transparency is a plain rectangle.
fn cut(pane: &Pane) -> Shape {
    let pieces = shape(&pane.canvas.pixels, pane.canvas.width)?;
    let (columns, rows) = (&pane.columns, &pane.rows);
    let physical = |(x0, y0, x1, y1): (u32, u32, u32, u32)| {
        (
            columns[x0 as usize],
            rows[y0 as usize],
            columns[x1 as usize],
            rows[y1 as usize],
        )
    };
    Some(pieces.into_iter().map(physical).collect())
}

/// The canvas's drawn pixels as rectangles of skin pixels `(x0, y0, x1, y1)`, end exclusive: runs of
/// a row, each carried down while the rows below have the same run. None when nothing is
/// transparent (the window is a plain rectangle).
fn shape(pixels: &[u32], width: u32) -> Option<Vec<(u32, u32, u32, u32)>> {
    if pixels.iter().all(|p| p >> 24 != 0) {
        return None;
    }
    let width = width as usize;
    let mut done = Vec::new();
    // Runs still growing down: x0, x1 and the row they began on.
    let mut open: Vec<(u32, u32, u32)> = Vec::new();
    let mut runs = Vec::new();
    for (y, row) in pixels.chunks_exact(width).enumerate() {
        let y = y as u32;
        runs.clear();
        let mut x = 0;
        while x < width {
            if row[x] >> 24 == 0 {
                x += 1;
                continue;
            }
            let start = x;
            while x < width && row[x] >> 24 != 0 {
                x += 1;
            }
            runs.push((start as u32, x as u32));
        }
        open.retain(|&(x0, x1, top)| {
            let goes_on = runs.contains(&(x0, x1));
            if !goes_on {
                done.push((x0, top, x1, y));
            }
            goes_on
        });
        for &(x0, x1) in &runs {
            if !open.iter().any(|&(a, b, _)| (a, b) == (x0, x1)) {
                open.push((x0, x1, y));
            }
        }
    }
    let height = (pixels.len() / width) as u32;
    done.extend(open.into_iter().map(|(x0, x1, top)| (x0, top, x1, height)));
    Some(done)
}

/// The part of rows `from..to` of the frame that differs from the one on screen, as skin pixels
/// `[x0, x1) x [y0, y1)`; all of them when nothing of this shape is on screen yet.
fn changed(frame: &[u32], shown: &[u32], width: u32, (from, to): (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    if shown.len() != frame.len() {
        return Some((0, from, width, to));
    }
    let (start, end) = ((from * width) as usize, (to * width) as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (width, to, 0, from);
    for (i, (a, b)) in frame[start..end].iter().zip(&shown[start..end]).enumerate() {
        if a != b {
            let (x, y) = (i as u32 % width, from + i as u32 / width);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
}

/// What a window shows, in one number: the window is drawn again only when it changes. The
/// analyser moves the main window's every frame while it plays; the equaliser and the playlist
/// change when the listener touches them (the playlist's clock once a second).
fn fingerprint(panel: Panel, shaded: bool, view: &View) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    (panel as u8, shaded, view.focus, view.pressed.map(|c| format!("{c:?}"))).hash(&mut h);
    let now = view.now;
    match panel {
        Panel::Main => {
            (
                now.index,
                (now.position * 10.0) as i64,
                now.playing,
                now.stopped,
                now.loading,
            )
                .hash(&mut h);
            (now.kbps, now.khz, now.stereo, now.duration.map(f64::to_bits)).hash(&mut h);
            (view.marquee, view.volume.to_bits(), view.seeking.map(f64::to_bits)).hash(&mut h);
            (
                view.remaining,
                view.blink_off,
                view.shuffle,
                view.repeat,
                view.eq_open,
                view.playlist_open,
            )
                .hash(&mut h);
            for bar in view.bars {
                (bar.height, bar.peak).hash(&mut h);
            }
        }
        Panel::Eq => {
            (view.eq.on, view.eq.preamp.map(f64::to_bits), view.volume.to_bits()).hash(&mut h);
            view.eq.bands.map(f64::to_bits).hash(&mut h);
        }
        Panel::Playlist => {
            let list = view.list;
            (
                &list.labels,
                list.scroll,
                &list.selected,
                (list.menu, list.hover),
                now.index,
                now.position as u64,
                now.stopped,
            )
                .hash(&mut h);
            if playlist::analyser_area(view).is_some() {
                for bar in view.bars {
                    (bar.height, bar.peak).hash(&mut h);
                }
            }
        }
    }
    h.finish()
}

/// The places of text runs that are new or gone since the last frame.
fn texts_changed<'a>(texts: &'a [TextRun], shown: &'a [TextRun]) -> impl Iterator<Item = (u32, u32, u32, u32)> + 'a {
    let gone = shown.iter().filter(|run| !texts.contains(run));
    let new = texts.iter().filter(|run| !shown.contains(run));
    gone.chain(new).map(|run| {
        let a = run.area;
        (a.x, a.y, a.x + a.width, a.y + a.height)
    })
}

fn union(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

/// The balance as Webamp takes it from its slider: whole percents, and within a quarter of the
/// middle, the middle.
fn centred(balance: f64) -> f64 {
    let percent = (balance.clamp(-1.0, 1.0) * 100.0).round();
    if percent.abs() < 25.0 { 0.0 } else { percent / 100.0 }
}

/// What the marquee says while the listener drags the balance (Webamp's words).
fn balance_line(balance: f64) -> String {
    let percent = (balance * 100.0).round();
    if percent > 0.0 {
        format!("Balance: {percent}% Right")
    } else if percent < 0.0 {
        format!("Balance: {}% Left", -percent)
    } else {
        "Balance: Center".to_owned()
    }
}

/// What the marquee says while the listener drags an equaliser slider (Webamp's words): the band
/// (`60HZ`, `3KHZ`, or `Preamp`) and its gain to a tenth of a decibel, a whole one without its `.0`.
fn eq_line(band: Option<usize>, db: f64) -> String {
    let name = match band.and_then(|band| eq::BANDS.get(band)) {
        Some(hz) if *hz < 1000.0 => format!("{hz}HZ"),
        Some(hz) => format!("{}KHZ", hz / 1000.0),
        None => "Preamp".to_owned(),
    };
    let db = (db * 10.0).round() / 10.0;
    let sign = if db > 0.0 { "+" } else { "" };
    format!("EQ: {name} {sign}{} DB", if db == 0.0 { 0.0 } else { db })
}

/// What the marquee says while the listener drags the seek bar (Winamp's words).
fn seek_line(fraction: f64, length: f64) -> String {
    let at = (fraction * length) as u32;
    format!(
        "Seek to: {}:{:02}/{}:{:02} ({}%)",
        at / 60,
        at % 60,
        length as u32 / 60,
        length as u32 % 60,
        (fraction * 100.0).round()
    )
}

/// The scrolling title: a character every step, round through a gap; a line that fits stays put.
#[derive(Default)]
struct Marquee {
    line: String,
    offset: usize,
    moved: Option<Instant>,
}

impl Marquee {
    fn show(&mut self, line: &str, now: Instant) -> String {
        if line != self.line {
            self.line = line.to_owned();
            self.offset = 0;
            self.moved = Some(now);
        }
        // Counted in UTF-16 units, the skin font's cells.
        let mut units: Vec<u16> = self.line.encode_utf16().collect();
        // A line that fits is padded with blanks to the marquee's width, as Webamp's is: a skin's
        // blank needn't be see-through.
        if units.len() < MARQUEE_CHARS {
            let mut line = self.line.clone();
            line.extend(std::iter::repeat_n(' ', MARQUEE_CHARS - units.len()));
            return line;
        }
        units.extend(MARQUEE_GAP.encode_utf16());
        let moved = self.moved.get_or_insert(now);
        let steps = (now.saturating_duration_since(*moved).as_millis() / MARQUEE_STEP.as_millis()) as u32;
        if steps > 0 {
            self.offset = self.offset.wrapping_add(steps as usize);
            *moved += MARQUEE_STEP * steps;
        }
        let shown: Vec<u16> = (0..MARQUEE_CHARS)
            .map(|i| units[(self.offset + i) % units.len()])
            .collect();
        // A character cut in half by the window is a blank cell, as its halves are.
        String::from_utf16_lossy(&shown)
    }
}

#[cfg(test)]
pub mod scenes {
    use super::super::session::Now;
    use super::*;

    /// The side-by-side with Webamp (tests/ui/classic-compare.mjs): every `.wsz` in
    /// `LULL_SKINS_IN` drawn as the window draws it at 1x (`paint`: the platform's own text over the canvas, as
    /// its window puts it on screen, into the canvas's pixels), the three windows stacked, one song
    /// loaded and stopped, the equaliser on and flat, in the scene `LULL_SCENE` names (the
    /// harness's: `stopped`, `sliders`, `shaded`, `unicode` or `menus`), into `<name>.png` in
    /// `LULL_RENDERS_OUT`. A skin missing pieces takes them from base-2.91 (`BASE_MD5.wsz`, in the
    /// same folder), as the window does. Each platform's window runs it by hand: `cargo test render_skins -- --ignored`.
    pub fn render_skins(paint: impl Fn(&Skin, &Canvas) -> Result<Vec<u32>, String>) -> Result<(), String> {
        let (inn, out) = match (std::env::var("LULL_SKINS_IN"), std::env::var("LULL_RENDERS_OUT")) {
            (Ok(i), Ok(o)) => (std::path::PathBuf::from(i), std::path::PathBuf::from(o)),
            _ => return Err("set LULL_SKINS_IN and LULL_RENDERS_OUT".into()),
        };
        let read = |p: &std::path::Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
        let base = Skin::from_archive(&read(&inn.join(format!("{BASE_MD5}.wsz")))?, None)?;
        // Volume, balance, whether the windows are rolled up, and the song's artist and title.
        let scene = std::env::var("LULL_SCENE");
        let (volume, balance, shaded, (artist, title)) = match scene.as_deref() {
            Err(_) | Ok("stopped" | "menus" | "resized" | "resized-even") => (0.75, 0.0, false, SCENE_SONG),
            Ok("sliders") => (0.3, -0.6, false, SCENE_SONG),
            Ok("shaded") => (0.3, -0.6, true, SCENE_SONG),
            Ok("unicode") => (0.75, 0.0, false, SCENE_UNICODE),
            Ok(other) => {
                return Err(format!(
                    "no scene {other}: stopped, sliders, shaded, unicode, menus, resized or resized-even"
                ));
            }
        };
        // REM open, the pointer on its second button, the playlist the selected window.
        let menus = scene.as_deref() == Ok("menus");
        let (menu, hover) = if menus { (Some(1), Some(1)) } else { (None, None) };
        let focus = if menus { Focus::Playlist } else { Focus::Main };
        let panels = Panels {
            main: true,
            eq: true,
            playlist: true,
            main_shade: shaded,
            eq_shade: shaded,
            playlist_shade: shaded,
            playlist_size: match scene.as_deref() {
                Ok("resized") => playlist::Size { wide: 3, tall: 2 },
                Ok("resized-even") => playlist::Size { wide: 4, tall: 1 },
                _ => playlist::Size::default(),
            },
        };
        let now = Now {
            duration: Some(SCENE_SECONDS),
            stopped: true,
            ..Now::default()
        };
        let eq = crate::desktop::classic::Equaliser {
            on: true,
            bands: [0.0; 10],
            preamp: Some(0.0),
        };
        let list = playlist::List {
            labels: vec![format!("1. {artist} - {title}")],
            lengths: vec![Some(SCENE_SECONDS)],
            selected: vec![false],
            scroll: 0,
            menu,
            hover,
        };
        let bars = [Bar::default(); BARS];
        let marquee = Marquee::default().show(&format!("1. {artist} - {title} (4:40)"), Instant::now());
        let view = View {
            now: &now,
            marquee: &marquee,
            volume,
            balance,
            focus,
            pressed: None,
            seeking: None,
            remaining: false,
            blink_off: false,
            bars: &bars,
            shuffle: false,
            repeat: false,
            main_open: true,
            eq_open: true,
            playlist_open: true,
            eq: &eq,
            list: &list,
            playlist: panels.playlist_size,
        };
        let mut done = 0;
        for entry in std::fs::read_dir(&inn).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "wsz") {
                continue;
            }
            let bytes = read(&path)?;
            let skin = match Skin::from_archive(&bytes, None) {
                Ok(skin) if skin.complete() => skin,
                _ => Skin::from_archive(&bytes, Some(&base))?,
            };
            // Each window drawn and painted on its own, then stacked as the harness's Webamp stacks
            // them: one image as wide as the widest.
            let sizes = Panel::ALL.map(|panel| panels.size(panel));
            let width = sizes.iter().map(|s| s.0).max().unwrap_or(WIDTH);
            let height = sizes.iter().map(|s| s.1).sum();
            let mut rgba = vec![0u8; (width * height * 4) as usize];
            let mut top = 0;
            for (panel, (w, h)) in Panel::ALL.into_iter().zip(sizes) {
                let mut canvas = Canvas::new(w, h);
                draw_panel(&mut canvas, &skin, (panel, panels.shaded(panel)), &view);
                let painted = paint(&skin, &canvas)?;
                for (i, (p, drawn)) in painted.iter().zip(&canvas.pixels).enumerate() {
                    let (x, y) = (i as u32 % w, top + i as u32 / w);
                    let alpha = if drawn >> 24 == 0 { 0 } else { 255 };
                    let at = ((y * width + x) * 4) as usize;
                    rgba[at..at + 4].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, alpha]);
                }
                top += h;
            }
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            image::save_buffer(
                out.join(format!("{name}.png")),
                &rgba,
                width,
                height,
                image::ColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
            done += 1;
        }
        eprintln!("rendered {done} skins into {}", out.display());
        Ok(())
    }

    /// The scene both sides draw (tests/ui/classic-compare.mjs has the same).
    const SCENE_SONG: (&str, &str) = ("Velvet Comet", "Static Season");
    const SCENE_UNICODE: (&str, &str) = (
        "\u{41a}\u{43e}\u{43c}\u{435}\u{442}\u{430}",
        "\u{9759}\u{304b} \u{2019}\u{e9} \u{1f3b5}",
    );
    const SCENE_SECONDS: f64 = 280.0;
    const BASE_MD5: &str = "5e4f10275dcb1fb211d4a8b4f1bda236";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_speak_as_webamps_do() {
        assert_eq!(centred(0.2), 0.0);
        assert_eq!(centred(-0.314), -0.31);
        assert_eq!(balance_line(0.0), "Balance: Center");
        assert_eq!(balance_line(-0.31), "Balance: 31% Left");
        assert_eq!(balance_line(1.0), "Balance: 100% Right");
        assert_eq!(eq_line(Some(0), 3.0), "EQ: 60HZ +3 DB");
        assert_eq!(eq_line(Some(5), -2.46), "EQ: 3KHZ -2.5 DB");
        assert_eq!(eq_line(Some(8), -0.01), "EQ: 14KHZ 0 DB");
        assert_eq!(eq_line(None, 12.0), "EQ: Preamp +12 DB");
    }

    #[test]
    fn the_keypad_skips_ten_songs_as_webamps_does() {
        assert_eq!(skipped(2, 30, false, 10), Some(12));
        assert_eq!(skipped(25, 30, false, 10), Some(29));
        assert_eq!(skipped(29, 30, false, 10), None);
        assert_eq!(skipped(3, 30, false, -10), Some(0));
        assert_eq!(skipped(0, 30, false, -10), None);
        assert_eq!(skipped(25, 30, true, 10), Some(5));
        assert_eq!(skipped(3, 30, true, -10), Some(23));
        assert_eq!(skipped(0, 0, true, 10), None);
    }

    #[test]
    fn each_window_has_its_size_rolled_up_or_not() {
        let mut panels = Panels {
            main: false,
            eq: true,
            playlist: true,
            main_shade: false,
            eq_shade: true,
            playlist_shade: false,
            playlist_size: playlist::Size { wide: 1, tall: 1 },
        };
        assert_eq!(panels.size(Panel::Main), (WIDTH, HEIGHT));
        assert_eq!(panels.size(Panel::Eq), (WIDTH, layout::SHADE_HEIGHT));
        assert_eq!(panels.size(Panel::Playlist), (300, 145));
        panels.playlist_shade = true;
        assert_eq!(panels.size(Panel::Playlist), (300, layout::SHADE_HEIGHT));
        assert!(!panels.showing(Panel::Main) && panels.showing(Panel::Eq));
    }

    #[test]
    fn the_window_is_cut_to_what_the_skin_draws() {
        let width = WIDTH as usize;
        let mut pixels = vec![0xff00_0000u32; width * 6];
        assert_eq!(shape(&pixels, WIDTH), None, "drawn everywhere: no shape");
        // A notch in rows 2 and 3, columns 10 to 19, and the last row only half drawn.
        for y in 2..4 {
            pixels[y * width + 10..y * width + 20].fill(0);
        }
        pixels[5 * width + 100..6 * width].fill(0);
        let mut pieces = shape(&pixels, WIDTH).unwrap_or_default();
        pieces.sort_unstable();
        assert_eq!(
            pieces,
            vec![
                (0, 0, WIDTH, 2),
                (0, 2, 10, 4),
                (0, 4, WIDTH, 5),
                (0, 5, 100, 6),
                (20, 2, WIDTH, 4),
            ]
        );
    }

    #[test]
    fn only_the_changed_pixels_go_to_the_screen() {
        let shown = vec![0u32; (WIDTH * HEIGHT) as usize];
        let mut frame = shown.clone();
        assert_eq!(changed(&frame, &shown, WIDTH, (0, HEIGHT)), None);
        frame[(50 * WIDTH + 30) as usize] = 1;
        frame[(52 * WIDTH + 40) as usize] = 1;
        assert_eq!(changed(&frame, &shown, WIDTH, (0, HEIGHT)), Some((30, 50, 41, 53)));
        assert_eq!(changed(&frame, &shown, WIDTH, (60, HEIGHT)), None);
        assert_eq!(changed(&frame, &[], WIDTH, (10, 20)), Some((0, 10, WIDTH, 20)));
    }

    #[test]
    fn a_long_title_scrolls_a_character_a_step_and_a_short_one_stays() {
        let mut marquee = Marquee::default();
        let start = Instant::now();
        // Padded to the marquee's 31 cells with blanks, as Webamp's is.
        assert_eq!(marquee.show("1. Short", start), format!("{:<31}", "1. Short"));
        let long = "1. Someone - A title far too long for Winamp's marquee";
        assert!(marquee.show(long, start).starts_with("1. Some"));
        assert!(marquee.show(long, start + MARQUEE_STEP * 3).starts_with("Someone"));
    }

    #[test]
    fn a_physical_pixel_falls_in_the_skin_pixel_that_covers_it() {
        let columns = edges(WIDTH, 1.5);
        assert_eq!(
            (skin_at(0, &columns), skin_at(2, &columns), skin_at(3, &columns)),
            (0, 1, 2)
        );
    }
}
