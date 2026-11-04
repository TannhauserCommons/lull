//! The Classic window on Windows: a plain Win32 window, no webview. Like Winamp 2's windows it's a
//! borderless popup the skin cuts to shape, here one layered window holding the main window, the
//! equaliser and the playlist stacked (as Webamp's window holds them), its pixels outside the
//! skin's shapes transparent (clicks there reach the window behind). The skin is drawn at 1x
//! (canvas.rs and a module per window), only what changed since the last frame is scaled up,
//! nearest neighbour so pixels stay sharp at any size, and handed to the compositor
//! (UpdateLayeredWindowIndirect with a dirty rectangle). The playlist's names are drawn at the
//! screen's own size in the skin's font (GDI), as Webamp's are.
//!
//! It lives on the main thread, whose message loop (Tauri's) delivers its messages. Its state sits
//! in a thread-local; Windows calls back into the window procedure while it runs (capture changes,
//! the move loop, menus), so the state is borrowed only to decide what to do, and the calls that
//! may call back run after the borrow ends.
#![allow(unsafe_code)]

use super::canvas::{Canvas, TextRun};
use super::draw::{self, Control, Target, View};
use super::engine::{Command, Snapshot, Status};
use super::eq::presets;
use super::eqwin;
use super::playlist::{self, List};
use super::queue::Repeat;
use super::session::Session;
use super::skin::{HEIGHT, Skin, WIDTH, layout};
use super::vis::{Analyser, FFT_SAMPLES};
use std::cell::RefCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CLIP_DEFAULT_PRECIS, ClientToScreen, CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET,
    DIB_RGB_COLORS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
    FF_DONTCARE, FW_NORMAL, HBITMAP, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode,
    SetTextCharacterExtra, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_DELETE, VK_DOWN, VK_LEFT, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, HMENU,
    HTCAPTION, IDC_ARROW, KillTimer, LoadCursorW, LoadIconW, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    RegisterClassExW, SW_MINIMIZE, SW_SHOWNORMAL, SendMessageW, SetForegroundWindow, SetTimer, ShowWindow,
    TPM_NONOTIFY, TPM_RETURNCMD, TrackPopupMenuEx, ULW_ALPHA, UPDATELAYEREDWINDOWINFO, UpdateLayeredWindowIndirect,
    WA_INACTIVE, WM_ACTIVATE, WM_CLOSE, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};
use windows::core::{HSTRING, PCWSTR, w};

const CLASS: PCWSTR = w!("lull-classic");
const TIMER: usize = 1;
/// While something moves (the analyser, a playing song): Winamp's own sixty a second.
const ANIMATE: Duration = Duration::from_millis(16);
/// Otherwise only the scrolling title and a pause's blinking time change.
const IDLE: Duration = Duration::from_millis(110);
/// The title scrolls a character this often (Winamp's pace), with this between its ends.
const MARQUEE_STEP: Duration = Duration::from_millis(220);
const MARQUEE_GAP: &str = "  ***  ";
/// Characters the marquee holds.
const MARQUEE_CHARS: usize = 31;
/// A notice (a skin that didn't load) holds the marquee this long.
const NOTICE: Duration = Duration::from_secs(8);
/// Tauri's build puts the app's icon in the executable under this id (IDI_APPLICATION's).
const APP_ICON: u16 = 32512;
/// The playlist's text, as Webamp sets it: 9 px, half a pixel between letters.
const TEXT_SIZE: f64 = 9.0;
const TEXT_SPACING: f64 = 0.5;
/// Rows a wheel notch scrolls the playlist, as Windows' lists do.
const WHEEL_ROWS: f64 = 3.0;

thread_local! {
    static OPEN: RefCell<Option<Classic>> = const { RefCell::new(None) };
}

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

/// The three windows, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Panel {
    Main,
    Eq,
    Playlist,
}

/// Which windows show, and which are rolled up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Panels {
    eq: bool,
    playlist: bool,
    main_shade: bool,
    eq_shade: bool,
    playlist_shade: bool,
}

impl Panels {
    /// The windows that show: each with its top and height in the stack.
    fn stack(&self) -> Vec<(Panel, u32, u32)> {
        let rolled = |shade: bool, height: u32| if shade { layout::SHADE_HEIGHT } else { height };
        let mut stack = vec![(Panel::Main, 0, rolled(self.main_shade, HEIGHT))];
        let mut top = stack[0].2;
        if self.eq {
            let height = rolled(self.eq_shade, HEIGHT);
            stack.push((Panel::Eq, top, height));
            top += height;
        }
        if self.playlist {
            stack.push((Panel::Playlist, top, rolled(self.playlist_shade, playlist::HEIGHT)));
        }
        stack
    }

    fn height(&self) -> u32 {
        self.stack().iter().map(|(_, _, h)| h).sum()
    }

    /// The window a row of the stack is in, and the row in it.
    fn at(&self, y: u32) -> Option<(Panel, u32)> {
        self.stack()
            .into_iter()
            .find(|(_, top, height)| y >= *top && y < top + height)
            .map(|(panel, top, _)| (panel, y - top))
    }

    fn top(&self, panel: Panel) -> u32 {
        self.stack()
            .into_iter()
            .find(|(p, _, _)| *p == panel)
            .map_or(0, |(_, top, _)| top)
    }

    fn shaded(&self, panel: Panel) -> bool {
        match panel {
            Panel::Main => self.main_shade,
            Panel::Eq => self.eq_shade,
            Panel::Playlist => self.playlist_shade,
        }
    }
}

/// The popup menus.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Menu {
    Options,
    /// The copies a song waiting in `choose` may play from.
    Choose,
    Presets,
    Remove,
    Select,
}

struct Classic {
    hwnd: HWND,
    skin: Skin,
    session: Session,
    on_close: Option<OnClose>,
    favourites: Vec<(String, String)>,
    surface: Surface,
    canvas: Canvas,
    text: Text,
    /// The frame on screen, at 1x: what the next one is compared with.
    shown: Vec<u32>,
    shown_texts: Vec<TextRun>,
    /// Each window's fingerprint when it was last drawn, top to bottom.
    prints: [Option<u64>; 3],
    /// The player as of this frame, and the queue's revision the playlist shows.
    snapshot: Snapshot,
    revision: Option<u64>,
    size: f64,
    dpi: f64,
    /// For each skin column and row, the first physical one it covers (the last entry is the size).
    columns: Vec<i32>,
    rows: Vec<i32>,
    panels: Panels,
    list: List,
    analyser: Analyser,
    focused: bool,
    /// The control held down and its window's top in the stack; for a button, whether the pointer
    /// is still on it.
    pressed: Option<(Target, u32)>,
    over: bool,
    seeking: Option<f64>,
    remaining: bool,
    marquee: Marquee,
    notice: Option<(String, Instant)>,
    /// The row a selection by Shift-click runs from, and the row a drag of songs is over.
    anchor: usize,
    dropping: Option<usize>,
    started: Instant,
    tick: Duration,
}

/// What the window procedure does once the state is let go.
enum Effect {
    Default,
    Handled,
    /// The listener pressed on a window's own body: Windows moves it.
    Move,
    Capture,
    Release,
    Minimize,
    Close,
    /// A popup menu at this point of the screen.
    Menu(Menu, POINT),
}

/// Opens the window. Main thread only.
pub fn open(setup: Setup, on_close: OnClose) -> Result<(), String> {
    if OPEN.with_borrow(Option::is_some) {
        return Err("Classic mode is open already".into());
    }
    let panels = Panels {
        eq: true,
        playlist: true,
        main_shade: false,
        eq_shade: false,
        playlist_shade: false,
    };
    let scale = setup.size * setup.dpi;
    let (columns, rows) = (edges(WIDTH, scale), edges(panels.height(), scale));
    let size = (columns[WIDTH as usize], rows[rows.len() - 1]);
    let hwnd = create(setup.at, size)?;
    crate::desktop::platform::add_thumb_buttons_to(setup.session.app(), hwnd.0 as isize);
    let mut session = setup.session;
    match super::smtc::Overlay::new(hwnd, session.app().clone()) {
        Ok(overlay) => session.attach_overlay(overlay),
        Err(e) => eprintln!("lull: no media keys for Classic: {e}"),
    }
    let surface = match Surface::new(size) {
        Ok(surface) => surface,
        Err(e) => {
            // SAFETY: the window was just created on this thread and nothing else holds it.
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(e);
        }
    };
    let mut classic = Classic {
        hwnd,
        text: Text::new(&setup.skin.playlist.font, scale),
        skin: setup.skin,
        session,
        on_close: Some(on_close),
        favourites: setup.favourites,
        surface,
        canvas: Canvas::new(panels.height()),
        shown: Vec::new(),
        shown_texts: Vec::new(),
        prints: [None; 3],
        snapshot: Snapshot::default(),
        revision: None,
        size: setup.size,
        dpi: setup.dpi,
        columns,
        rows,
        panels,
        list: List::default(),
        analyser: Analyser::default(),
        focused: true,
        pressed: None,
        over: false,
        seeking: None,
        remaining: false,
        marquee: Marquee::default(),
        notice: setup.said.map(|said| (said, Instant::now())),
        anchor: 0,
        dropping: None,
        started: Instant::now(),
        tick: ANIMATE,
    };
    classic.snapshot = classic.session.snapshot();
    classic.relist();
    let index = classic.snapshot.index;
    classic.list.show(index);
    classic.render();
    OPEN.with_borrow_mut(|open| *open = Some(classic));
    // SAFETY: a window this thread owns.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
        SetTimer(Some(hwnd), TIMER, ANIMATE.as_millis() as u32, None);
    }
    Ok(())
}

/// Brings the window forward (the tray, a second launch). Main thread only.
pub fn show() {
    if let Some(hwnd) = OPEN.with_borrow(|open| open.as_ref().map(|c| c.hwnd)) {
        // SAFETY: a window this thread owns.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// The tray's and the taskbar's buttons, the system's media keys. Main thread only.
pub fn act(action: &str) {
    let acted = OPEN.with_borrow_mut(|open| {
        let classic = open.as_mut()?;
        classic.session.act(action);
        classic.render();
        Some(())
    });
    if acted.is_none() {
        eprintln!("lull: Classic is closed: no {action}");
    }
}

/// Another skin, fetched for the Options menu; or why it couldn't be. Main thread only.
pub fn set_skin(skin: Result<(Skin, Option<String>), String>) {
    OPEN.with_borrow_mut(|open| {
        let Some(classic) = open.as_mut() else {
            return;
        };
        match skin {
            Ok((skin, said)) => {
                classic.text = Text::new(&skin.playlist.font, classic.size * classic.dpi);
                classic.skin = skin;
                classic.notice = said.map(|said| (said, Instant::now()));
                classic.shown.clear();
            }
            Err(why) => classic.notice = Some((format!("Couldn't load the skin: {why}"), Instant::now())),
        }
        classic.render();
    });
}

/// Leaves Classic as its close button does. Main thread only.
pub fn close() {
    let Some(classic) = OPEN.with_borrow_mut(Option::take) else {
        return;
    };
    // SAFETY: a window this thread owns; its state is out of the thread-local already, so the
    // messages destroying it sends find nothing to borrow.
    unsafe {
        let _ = KillTimer(Some(classic.hwnd), TIMER);
        let _ = DestroyWindow(classic.hwnd);
    }
    let size = classic.size;
    let Classic { session, on_close, .. } = classic;
    if let Some(on_close) = on_close {
        on_close(session, size);
    }
}

fn create(at: (i32, i32), size: (i32, i32)) -> Result<HWND, String> {
    // SAFETY: plain Win32 calls with static strings; the class is registered once per process
    // (a second registration fails harmlessly and the first stands).
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            hIcon: LoadIconW(Some(module.into()), PCWSTR(APP_ICON as usize as *const u16)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_APPWINDOW,
            CLASS,
            w!("lull classic"),
            WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            at.0,
            at.1,
            size.0,
            size.1,
            None,
            None,
            Some(module.into()),
            None,
        )
        .map_err(|e| e.to_string())
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Borrowed already: Windows called back from inside a call this window made. That message gets
    // the default handling.
    let effect = OPEN.with(|open| match open.try_borrow_mut().as_deref_mut() {
        Ok(Some(classic)) if classic.hwnd == hwnd => classic.handle(message, wparam, lparam),
        _ => Effect::Default,
    });
    // SAFETY: calls on this thread's own window, with the borrow above ended.
    unsafe {
        match effect {
            Effect::Default => DefWindowProcW(hwnd, message, wparam, lparam),
            Effect::Handled => LRESULT(0),
            Effect::Move => {
                let _ = ReleaseCapture();
                SendMessageW(
                    hwnd,
                    WM_NCLBUTTONDOWN,
                    Some(WPARAM(HTCAPTION as usize)),
                    Some(LPARAM(0)),
                );
                LRESULT(0)
            }
            Effect::Capture => {
                SetCapture(hwnd);
                LRESULT(0)
            }
            Effect::Release => {
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            Effect::Minimize => {
                let _ = ReleaseCapture();
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
                LRESULT(0)
            }
            Effect::Close => {
                let _ = ReleaseCapture();
                close();
                LRESULT(0)
            }
            Effect::Menu(menu, at) => {
                let _ = ReleaseCapture();
                pop_up(hwnd, menu, at);
                LRESULT(0)
            }
        }
    }
}

/// Shows a menu and, once the listener has chosen, does what they chose.
fn pop_up(hwnd: HWND, menu: Menu, at: POINT) {
    let Some(items) = OPEN.with_borrow(|open| open.as_ref().map(|c| c.menu_items(menu))) else {
        return;
    };
    // SAFETY: a menu this function makes and destroys, on a window this thread owns; the menu's
    // loop runs with no borrow of the window's state held.
    let chosen = unsafe {
        let Ok(popup) = build_menu(&items) else {
            return;
        };
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenuEx(popup, (TPM_RETURNCMD | TPM_NONOTIFY).0, at.x, at.y, hwnd, None);
        let _ = DestroyMenu(popup);
        chosen.0 as u32
    };
    if chosen == 0 {
        return;
    }
    let effect = OPEN.with_borrow_mut(|open| open.as_mut().map(|c| c.chose(menu, chosen)));
    if let Some(Effect::Close) = effect {
        close();
    }
}

/// A menu's line: a command (its id, words, ticked or not), a separator, or a submenu.
enum Item {
    Command(u32, String, bool),
    Separator,
    Submenu(String, Vec<Item>),
}

/// SAFETY: the caller destroys the menu (its submenus go with it).
unsafe fn build_menu(items: &[Item]) -> windows::core::Result<HMENU> {
    // SAFETY: a fresh menu, filled with strings that outlive each call.
    unsafe {
        let menu = CreatePopupMenu()?;
        for item in items {
            match item {
                Item::Command(id, words, ticked) => {
                    let flags = if *ticked { MF_STRING | MF_CHECKED } else { MF_STRING };
                    AppendMenuW(menu, flags, *id as usize, &HSTRING::from(words.as_str()))?;
                }
                Item::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?,
                Item::Submenu(words, items) => {
                    let sub = build_menu(items)?;
                    AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(words.as_str()))?;
                }
            }
        }
        Ok(menu)
    }
}

impl Classic {
    fn handle(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Effect {
        let effect = match message {
            WM_TIMER => {
                self.render();
                return Effect::Handled;
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.client_to_skin(lparam);
                self.press(x, y, false)
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = self.client_to_skin(lparam);
                self.press(x, y, true)
            }
            // Windows reports a double click on a window's body as one on its caption: the first
            // click started a move.
            WM_NCLBUTTONDBLCLK => {
                let (x, y) = self.screen_to_skin(lparam);
                self.double_click_title(x, y)
            }
            WM_MOUSEMOVE if self.pressed.is_some() => {
                let (x, y) = self.client_to_skin(lparam);
                self.drag(x, y);
                Effect::Handled
            }
            WM_LBUTTONUP if self.pressed.is_some() => {
                let (x, y) = self.client_to_skin(lparam);
                self.drag(x, y);
                self.release()
            }
            WM_RBUTTONUP => Effect::Menu(Menu::Options, self.to_screen(lparam)),
            WM_MOUSEWHEEL => {
                let notches = f64::from((wparam.0 >> 16) as u16 as i16) / 120.0;
                self.wheel(notches, lparam)
            }
            WM_KEYDOWN => self.key(wparam.0 as u16),
            WM_ACTIVATE => {
                self.focused = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
                Effect::Default
            }
            WM_CLOSE => Effect::Close,
            _ => return Effect::Default,
        };
        self.render();
        effect
    }

    /// The window under a point of the stack, the row in it, and its control there.
    fn target_at(&self, x: u32, y: u32) -> Option<(Panel, u32, Option<Target>)> {
        let (panel, local) = self.panels.at(y)?;
        let shaded = self.panels.shaded(panel);
        let target = match panel {
            Panel::Main => draw::main_at(x, local, shaded),
            Panel::Eq => eqwin::at(x, local, shaded),
            Panel::Playlist => playlist::at(x, local, shaded, &self.list),
        };
        Some((panel, local, target))
    }

    fn press(&mut self, x: u32, y: u32, double: bool) -> Effect {
        let Some((panel, _, target)) = self.target_at(x, y) else {
            return Effect::Default;
        };
        let Some(target) = target else {
            if double {
                return self.double_click_title(x, y);
            }
            return Effect::Move;
        };
        match target.control {
            Control::Time => {
                self.remaining = !self.remaining;
                return Effect::Handled;
            }
            Control::PlaylistRow(row) => {
                let effect = self.click_row(row, double);
                // A single press on a song may drag the selected songs elsewhere in the list.
                if !double && row < self.list.labels.len() {
                    self.pressed = Some((target, self.panels.top(panel)));
                    self.dropping = Some(row);
                    return Effect::Capture;
                }
                return effect;
            }
            _ => {}
        }
        self.pressed = Some((target, self.panels.top(panel)));
        self.over = true;
        self.drag(x, y);
        Effect::Capture
    }

    /// A double click on a window's title bar rolls it up or down (Winamp's shade).
    fn double_click_title(&mut self, x: u32, y: u32) -> Effect {
        let Some((panel, local, target)) = self.target_at(x, y) else {
            return Effect::Handled;
        };
        let title = if panel == Panel::Playlist && !self.panels.playlist_shade {
            layout::PLAYLIST_TITLE_HEIGHT
        } else {
            layout::SHADE_HEIGHT
        };
        if target.is_none() && local < title {
            self.toggle_shade(panel);
        }
        Effect::Handled
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
    fn click_row(&mut self, row: usize, double: bool) -> Effect {
        if row >= self.list.labels.len() {
            self.list.selected.fill(false);
            return Effect::Handled;
        }
        if double {
            self.send(Command::Jump(row));
            return Effect::Handled;
        }
        // SAFETY: reads this thread's keyboard state.
        let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
        if held(VK_CONTROL.0) {
            self.list.selected[row] = !self.list.selected[row];
        } else {
            let (from, to) = if held(VK_SHIFT.0) {
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
        let Some((target, top)) = self.pressed else {
            return;
        };
        let area = target.area;
        let (x, y) = (f64::from(x), f64::from(y) - f64::from(top));
        let along = |at: f64, start: u32, length: u32| {
            let travel = f64::from(length - target.knob);
            ((at - f64::from(start) - f64::from(target.knob) / 2.0) / travel).clamp(0.0, 1.0)
        };
        let across = along(x, area.x, area.width);
        let down = along(y, area.y, area.height);
        match target.control {
            Control::Volume => self.volume(across),
            Control::Position => self.seeking = Some(across),
            Control::EqPreamp => self.equalise(|eq| eq.preamp = Some(eqwin::decibels(1.0 - down))),
            Control::EqBand(band) => self.equalise(|eq| eq.bands[band] = eqwin::decibels(1.0 - down)),
            Control::PlaylistRow(_) => self.dropping = Some(playlist::row_at(y.max(0.0) as u32, &self.list)),
            Control::PlaylistScroll => {
                let room = self.list.labels.len().saturating_sub(List::rows());
                self.list.scroll_to((down * room as f64).round() as usize);
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
        let Some((target, top)) = self.pressed.take() else {
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
            return self.click(target, top);
        }
        Effect::Release
    }

    /// A button, clicked: Winamp's own meaning for each.
    fn click(&mut self, target: Target, top: u32) -> Effect {
        let now = &self.snapshot;
        let area = target.area;
        let below = self.client_to_screen(
            self.columns[area.x as usize],
            self.rows[((top + area.y + area.height) as usize).min(self.rows.len() - 1)],
        );
        let command = match target.control {
            Control::Previous => Some(Command::Previous),
            // Play on a playing song starts it again.
            Control::Play if now.playing => Some(Command::Seek(0.0)),
            Control::Play => Some(Command::Play),
            Control::Pause if now.playing => Some(Command::Pause),
            Control::Pause if !now.stopped => Some(Command::Play),
            Control::Stop => Some(Command::Stop),
            Control::Next => Some(Command::Next),
            Control::Shuffle => Some(Command::Shuffle(!now.shuffle)),
            Control::Repeat => Some(Command::CycleRepeat),
            Control::Marquee if matches!(now.status, Status::Choose(_)) => return Effect::Menu(Menu::Choose, below),
            _ => None,
        };
        if let Some(command) = command {
            self.send(command);
            return Effect::Release;
        }
        match target.control {
            Control::Minimize => return Effect::Minimize,
            Control::Close => return Effect::Close,
            Control::Shade => self.toggle_shade(Panel::Main),
            Control::EqShade => self.toggle_shade(Panel::Eq),
            Control::PlaylistShade => self.toggle_shade(Panel::Playlist),
            Control::EqWindow | Control::EqClose => self.panels.eq = !self.panels.eq,
            Control::PlaylistWindow | Control::PlaylistClose => self.panels.playlist = !self.panels.playlist,
            Control::EqOn => self.equalise(|eq| eq.on = !eq.on),
            Control::EqAuto => self.equalise(|eq| {
                eq.preamp = match eq.preamp {
                    Some(_) => None,
                    None => Some(eq.level()),
                }
            }),
            Control::Options => return Effect::Menu(Menu::Options, below),
            Control::EqPresets => return Effect::Menu(Menu::Presets, below),
            Control::PlaylistMenu(1) => return Effect::Menu(Menu::Remove, below),
            Control::PlaylistMenu(2) => return Effect::Menu(Menu::Select, below),
            _ => {}
        }
        Effect::Release
    }

    fn menu_items(&self, menu: Menu) -> Vec<Item> {
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
            Menu::Remove => vec![
                Item::Command(1, "Remove selected\tDel".into(), false),
                Item::Command(2, "Crop selected".into(), false),
            ],
            Menu::Select => vec![
                Item::Command(1, "Select all".into(), false),
                Item::Command(2, "Select none".into(), false),
                Item::Command(3, "Invert selection".into(), false),
            ],
        }
    }

    fn chose(&mut self, menu: Menu, id: u32) -> Effect {
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
            (Menu::Remove, 1) => self.remove_selected(),
            (Menu::Remove, _) => {
                let keep = self.list.selected.clone();
                self.keep(&keep);
            }
            (Menu::Select, how) => {
                for selected in &mut self.list.selected {
                    *selected = match how {
                        1 => true,
                        2 => false,
                        _ => !*selected,
                    };
                }
            }
        }
        self.render();
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
        self.text = Text::new(&self.skin.playlist.font, self.size * self.dpi);
        self.rows.clear();
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
        self.list.scroll_to(scroll);
    }

    /// The wheel scrolls the playlist under the pointer, else turns the volume.
    fn wheel(&mut self, notches: f64, lparam: LPARAM) -> Effect {
        let (x, y) = self.screen_to_skin(lparam);
        if let Some((Panel::Playlist, _, _)) = self.target_at(x, y) {
            let rows = (-notches * WHEEL_ROWS).round() as isize;
            let first = self.list.scroll.saturating_add_signed(rows);
            self.list.scroll_to(first);
        } else {
            self.volume(self.snapshot.volume + 0.05 * notches);
        }
        Effect::Handled
    }

    /// Winamp's keys: Z X C V B for the transport, arrows for a few seconds and the volume, Ctrl+D
    /// for Double Size, Delete for the selected songs.
    fn key(&mut self, key: u16) -> Effect {
        let (playing, position, volume) = (self.snapshot.playing, self.snapshot.position, self.snapshot.volume);
        // SAFETY: reads this thread's keyboard state.
        let control = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        let letter = |c: u8| key == u16::from(c);
        let command = match key {
            _ if letter(b'Z') => Command::Previous,
            _ if letter(b'X') && playing => Command::Seek(0.0),
            _ if letter(b'X') => Command::Play,
            _ if letter(b'C') && playing => Command::Pause,
            _ if letter(b'C') => Command::Play,
            _ if letter(b'V') => Command::Stop,
            _ if letter(b'B') => Command::Next,
            _ if letter(b'D') && control => {
                self.double_size();
                return Effect::Handled;
            }
            k if k == VK_LEFT.0 => Command::Seek((position - 5.0).max(0.0)),
            k if k == VK_RIGHT.0 => Command::Seek(position + 5.0),
            k if k == VK_UP.0 => {
                self.volume(volume + 0.02);
                return Effect::Handled;
            }
            k if k == VK_DOWN.0 => {
                self.volume(volume - 0.02);
                return Effect::Handled;
            }
            k if k == VK_DELETE.0 => {
                self.remove_selected();
                return Effect::Handled;
            }
            _ => return Effect::Default,
        };
        self.send(command);
        Effect::Handled
    }

    /// Draws a frame at 1x and puts on screen what changed. The timer then runs at the pace the
    /// window needs: fast while something moves, slow while only the title or a blink does.
    fn render(&mut self) {
        self.snapshot = self.session.snapshot();
        self.session.tick(&self.snapshot);
        if self.revision != Some(self.snapshot.revision) {
            self.revision = Some(self.snapshot.revision);
            self.relist();
        }
        let now = self.session.now(&self.snapshot);
        let samples = if now.playing {
            self.session.engine.tap(FFT_SAMPLES)
        } else {
            Vec::new()
        };
        let clock = Instant::now();
        let bars = self.analyser.step(&samples, clock);
        self.notice = self.notice.take().filter(|(_, at)| at.elapsed() < NOTICE);
        let line = match (
            &now.said,
            &self.notice,
            self.seeking,
            self.session.titles.get(now.index),
        ) {
            (Some(error), _, _, _) => error.clone(),
            (None, Some((notice, _)), _, _) => notice.clone(),
            (None, None, Some(fraction), _) => seek_line(fraction, now.duration.unwrap_or(0.0)),
            (None, None, None, Some(title)) => title.clone(),
            (None, None, None, None) => String::new(),
        };
        let marquee = self.marquee.show(&line, clock);
        let blink_off = (clock - self.started).as_millis() / 500 % 2 == 1;
        let height = self.panels.height();
        self.canvas.resize(height);
        if self.rows.len() != height as usize + 1 {
            self.resize(height);
        }
        let view = View {
            now: &now,
            marquee: &marquee,
            volume: self.snapshot.volume,
            focused: self.focused,
            pressed: self.pressed.filter(|_| self.over).map(|(t, _)| t.control),
            seeking: self.seeking,
            remaining: self.remaining,
            blink_off,
            bars: &bars,
            shuffle: self.snapshot.shuffle,
            repeat: self.snapshot.repeat != Repeat::Off,
            eq_open: self.panels.eq,
            playlist_open: self.panels.playlist,
            eq: &self.snapshot.eq,
            list: &self.list,
        };
        let mut drawn = Vec::new();
        for (slot, (panel, top, rows)) in self.panels.stack().into_iter().enumerate() {
            let print = Some(fingerprint(panel, top, self.panels.shaded(panel), &view));
            if self.shown.is_empty() || self.prints[slot] != print {
                self.prints[slot] = print;
                drawn.push((top, top + rows));
            } else {
                continue;
            }
            self.canvas.clear(top, rows);
            let mut pen = self.canvas.pen(&self.skin, top);
            match (panel, self.panels.shaded(panel)) {
                (Panel::Main, false) => draw::main(&mut pen, &view),
                (Panel::Main, true) => draw::main_shaded(&mut pen, &view),
                (Panel::Eq, false) => eqwin::window(&mut pen, &view),
                (Panel::Eq, true) => eqwin::shaded(&mut pen, &view),
                (Panel::Playlist, false) => playlist::window(&mut pen, &view),
                (Panel::Playlist, true) => playlist::shaded(&mut pen, &view),
            }
        }
        let dirty = drawn
            .iter()
            .filter_map(|&(from, to)| changed(&self.canvas.pixels, &self.shown, from, to))
            .chain(texts_changed(&self.canvas.texts, &self.shown_texts))
            .reduce(union);
        if let Some(dirty) = dirty {
            self.present(dirty);
            if self.shown.len() == self.canvas.pixels.len() {
                let (from, to) = ((dirty.1 * WIDTH) as usize, (dirty.3 * WIDTH) as usize);
                self.shown[from..to].copy_from_slice(&self.canvas.pixels[from..to]);
            } else {
                self.shown.clone_from(&self.canvas.pixels);
            }
            self.shown_texts.clone_from(&self.canvas.texts);
        }
        let moving = now.playing || !self.analyser.settled() || self.seeking.is_some() || self.pressed.is_some();
        let tick = if moving { ANIMATE } else { IDLE };
        if tick != self.tick {
            self.tick = tick;
            // SAFETY: a window this thread owns; replaces the running timer.
            unsafe { SetTimer(Some(self.hwnd), TIMER, tick.as_millis() as u32, None) };
        }
    }

    /// The stack's height or the size changed: a new bitmap, all of it drawn again.
    fn resize(&mut self, height: u32) {
        let scale = self.size * self.dpi;
        self.columns = edges(WIDTH, scale);
        self.rows = edges(height, scale);
        let size = (self.columns[WIDTH as usize], self.rows[height as usize]);
        match Surface::new(size) {
            Ok(surface) => self.surface = surface,
            Err(e) => eprintln!("lull: Classic window: {e}"),
        }
        // What was on screen was another shape: nothing of it is compared with, all of it is drawn.
        self.shown.clear();
        self.shown_texts.clear();
        self.prints = [None; 3];
    }

    /// Scales the changed skin pixels into the window's bitmap, draws the playlist's text there at
    /// the screen's size, and hands that part to Windows.
    fn present(&mut self, (x0, y0, x1, y1): (u32, u32, u32, u32)) {
        let (left, right) = (self.columns[x0 as usize], self.columns[x1 as usize]);
        let (top, bottom) = (self.rows[y0 as usize], self.rows[y1 as usize]);
        let stride = self.surface.size.0 as usize;
        let pixels = self.surface.pixels();
        for py in top..bottom {
            let sy = skin_at(py, &self.rows) as usize;
            let source = &self.canvas.pixels[sy * WIDTH as usize..(sy + 1) * WIDTH as usize];
            let row = &mut pixels[py as usize * stride..(py as usize + 1) * stride];
            for sx in x0..x1 {
                let (from, to) = (
                    self.columns[sx as usize] as usize,
                    self.columns[sx as usize + 1] as usize,
                );
                row[from..to.min(stride)].fill(source[sx as usize]);
            }
        }
        let dirty = RECT {
            left,
            top,
            right,
            bottom,
        };
        for run in &self.canvas.texts {
            let area = run.area;
            let at = RECT {
                left: self.columns[area.x as usize],
                top: self.rows[area.y as usize],
                right: self.columns[(area.x + area.width) as usize],
                bottom: self.rows[(area.y + area.height) as usize],
            };
            if at.left < dirty.right && dirty.left < at.right && at.top < dirty.bottom && dirty.top < at.bottom {
                self.text.draw(&mut self.surface, run, at);
            }
        }
        let size = SIZE {
            cx: self.surface.size.0,
            cy: self.surface.size.1,
        };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            psize: &size,
            hdcSrc: self.surface.dc,
            pptSrc: &origin,
            crKey: COLORREF(0),
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: &dirty,
            ..Default::default()
        };
        // SAFETY: every pointer in `info` outlives the call; the surface's bitmap is selected into
        // its DC.
        if !unsafe { UpdateLayeredWindowIndirect(self.hwnd, &info) }.as_bool() {
            eprintln!(
                "lull: Classic window didn't update: {}",
                windows::core::Error::from_thread()
            );
        }
    }

    fn client_to_screen(&self, x: i32, y: i32) -> POINT {
        let mut point = POINT { x, y };
        // SAFETY: a window this thread owns, and a point to convert in place.
        let _ = unsafe { ClientToScreen(self.hwnd, &mut point) };
        point
    }

    fn to_screen(&self, lparam: LPARAM) -> POINT {
        let (x, y) = coordinates(lparam);
        self.client_to_screen(x, y)
    }

    /// A point of the window (a mouse message's) as a skin pixel of the stack.
    fn client_to_skin(&self, lparam: LPARAM) -> (u32, u32) {
        let (x, y) = coordinates(lparam);
        (skin_at(x, &self.columns), skin_at(y, &self.rows))
    }

    /// A point of the screen (the wheel's, a caption's double click) as a skin pixel of the stack.
    fn screen_to_skin(&self, lparam: LPARAM) -> (u32, u32) {
        let (x, y) = coordinates(lparam);
        let origin = self.client_to_screen(0, 0);
        (skin_at(x - origin.x, &self.columns), skin_at(y - origin.y, &self.rows))
    }
}

/// A mouse message's point, the two signed halves of its `lparam`.
fn coordinates(lparam: LPARAM) -> (i32, i32) {
    let x = i32::from((lparam.0 & 0xffff) as u16 as i16);
    let y = i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16);
    (x, y)
}

/// For each of `count` skin pixels and its end, the first physical pixel it covers at `scale`.
fn edges(count: u32, scale: f64) -> Vec<i32> {
    (0..=count).map(|i| (f64::from(i) * scale).ceil() as i32).collect()
}

/// The skin pixel a physical coordinate falls in: `edges` holds, for each skin pixel, the first
/// physical one it covers.
fn skin_at(physical: i32, edges: &[i32]) -> u32 {
    edges.partition_point(|&e| e <= physical).saturating_sub(1) as u32
}

/// The part of rows `from..to` of the frame that differs from the one on screen, as skin pixels
/// `[x0, x1) x [y0, y1)`; all of them when nothing of this shape is on screen yet.
fn changed(frame: &[u32], shown: &[u32], from: u32, to: u32) -> Option<(u32, u32, u32, u32)> {
    if shown.len() != frame.len() {
        return Some((0, from, WIDTH, to));
    }
    let (start, end) = ((from * WIDTH) as usize, (to * WIDTH) as usize);
    let (mut x0, mut y0, mut x1, mut y1) = (WIDTH, to, 0, from);
    for (i, (a, b)) in frame[start..end].iter().zip(&shown[start..end]).enumerate() {
        if a != b {
            let (x, y) = (i as u32 % WIDTH, from + i as u32 / WIDTH);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
}

/// What a window shows, in one number: the window is drawn again only when it changes. The
/// analyser moves the main window's every frame while it plays; the equaliser and the playlist
/// change when the listener touches them (the playlist's clock once a second).
fn fingerprint(panel: Panel, top: u32, shaded: bool, view: &View) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    (
        panel as u8,
        top,
        shaded,
        view.focused,
        view.pressed.map(|c| format!("{c:?}")),
    )
        .hash(&mut h);
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
                now.index,
                now.position as u64,
                now.stopped,
            )
                .hash(&mut h);
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
        let mut chars: Vec<char> = self.line.chars().collect();
        if chars.len() < MARQUEE_CHARS {
            return self.line.clone();
        }
        chars.extend(MARQUEE_GAP.chars());
        let moved = self.moved.get_or_insert(now);
        let steps = (now.saturating_duration_since(*moved).as_millis() / MARQUEE_STEP.as_millis()) as u32;
        if steps > 0 {
            self.offset = self.offset.wrapping_add(steps as usize);
            *moved += MARQUEE_STEP * steps;
        }
        (0..MARQUEE_CHARS)
            .map(|i| chars[(self.offset + i) % chars.len()])
            .collect()
    }
}

/// The playlist's text, in the skin's font (`pledit.txt`) at the screen's size. Each line is drawn
/// white on black into a scratch bitmap, greyscale-smoothed, and that coverage lays its colour over
/// the row in the window's bitmap: GDI leaves alpha alone, and the window's pixels need theirs.
struct Text {
    font: HFONT,
    spacing: i32,
    scratch: Option<Surface>,
}

impl Text {
    fn new(face: &str, scale: f64) -> Self {
        let height = -(TEXT_SIZE * scale).round() as i32;
        // SAFETY: a font made from plain values and a name that outlives the call; deleted in Drop.
        let font = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                FF_DONTCARE.0 as u32,
                &HSTRING::from(face),
            )
        };
        Self {
            font,
            spacing: (TEXT_SPACING * scale).round() as i32,
            scratch: None,
        }
    }

    fn draw(&mut self, surface: &mut Surface, run: &TextRun, at: RECT) {
        let (width, height) = (at.right - at.left, at.bottom - at.top);
        if width <= 0 || height <= 0 {
            return;
        }
        if self
            .scratch
            .as_ref()
            .is_none_or(|s| s.size.0 < width || s.size.1 < height)
        {
            match Surface::new((width.max(64), height.max(16))) {
                Ok(scratch) => self.scratch = Some(scratch),
                Err(e) => return eprintln!("lull: Classic text: {e}"),
            }
        }
        let Some(scratch) = self.scratch.as_mut() else {
            return;
        };
        let stride = scratch.size.0 as usize;
        let (w, h) = (width as usize, height as usize);
        for row in scratch.pixels().chunks_exact_mut(stride).take(h) {
            row[..w].fill(0);
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let mut words: Vec<u16> = run.text.encode_utf16().collect();
        let align = if run.right { DT_RIGHT } else { DT_LEFT };
        // SAFETY: the scratch bitmap is selected into its own DC; the font outlives the call.
        unsafe {
            let previous = SelectObject(scratch.dc, self.font.into());
            SetTextColor(scratch.dc, COLORREF(0x00ff_ffff));
            SetBkMode(scratch.dc, TRANSPARENT);
            SetTextCharacterExtra(scratch.dc, self.spacing);
            DrawTextW(
                scratch.dc,
                &mut words,
                &mut rect,
                align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
            SelectObject(scratch.dc, previous);
        }
        let coverage: Vec<u32> = scratch
            .pixels()
            .chunks_exact(stride)
            .take(h)
            .flat_map(|row| row[..w].iter().map(|p| (p >> 8) & 0xff))
            .collect();
        let [r, g, b] = run.color.map(u32::from);
        let target_stride = surface.size.0 as usize;
        let pixels = surface.pixels();
        for (i, &a) in coverage.iter().enumerate().filter(|(_, a)| **a > 0) {
            let (x, y) = (i % w, i / w);
            let index = (at.top as usize + y) * target_stride + at.left as usize + x;
            let Some(pixel) = pixels.get_mut(index) else {
                continue;
            };
            let mix = |under: u32, over: u32| (over * a + under * (255 - a)) / 255;
            let (ur, ug, ub) = ((*pixel >> 16) & 0xff, (*pixel >> 8) & 0xff, *pixel & 0xff);
            *pixel = 0xff00_0000 | mix(ur, r) << 16 | mix(ug, g) << 8 | mix(ub, b);
        }
    }
}

impl Drop for Text {
    fn drop(&mut self) {
        // SAFETY: the font this made, deleted once.
        let _ = unsafe { DeleteObject(self.font.into()) };
    }
}

/// A bitmap: physical size, 32-bit premultiplied BGRA, top row first, in a memory DC (the window's,
/// for UpdateLayeredWindowIndirect, and the text's scratch).
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut c_void,
    size: (i32, i32),
}

impl Surface {
    fn new(size: (i32, i32)) -> Result<Self, String> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.0,
                // Negative: rows top to bottom.
                biHeight: -size.1,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` describes a plain 32-bit DIB; `bits` receives its memory, owned by the
        // bitmap until it is deleted in Drop.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) if !bits.is_null() => bitmap,
                result => {
                    let _ = DeleteDC(dc);
                    return Err(format!("no bitmap for the Classic window: {result:?}"));
                }
            };
            let previous = SelectObject(dc, bitmap.into());
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits,
                size,
            })
        }
    }

    fn pixels(&mut self) -> &mut [u32] {
        let count = (self.size.0 * self.size.1) as usize;
        // SAFETY: the DIB holds width x height 32-bit pixels for as long as `self` lives, and the
        // `&mut self` borrow makes this the only view of them.
        unsafe { std::slice::from_raw_parts_mut(self.bits.cast::<u32>(), count) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the objects this surface created, released once.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_changed_pixels_go_to_the_screen() {
        let shown = vec![0u32; (WIDTH * HEIGHT) as usize];
        let mut frame = shown.clone();
        assert_eq!(changed(&frame, &shown, 0, HEIGHT), None);
        frame[(50 * WIDTH + 30) as usize] = 1;
        frame[(52 * WIDTH + 40) as usize] = 1;
        assert_eq!(changed(&frame, &shown, 0, HEIGHT), Some((30, 50, 41, 53)));
        assert_eq!(changed(&frame, &shown, 60, HEIGHT), None);
        assert_eq!(changed(&frame, &[], 10, 20), Some((0, 10, WIDTH, 20)));
    }

    #[test]
    fn a_long_title_scrolls_a_character_a_step_and_a_short_one_stays() {
        let mut marquee = Marquee::default();
        let start = Instant::now();
        assert_eq!(marquee.show("1. Short", start), "1. Short");
        let long = "1. Someone - A title far too long for Winamp's marquee";
        assert!(marquee.show(long, start).starts_with("1. Some"));
        assert!(marquee.show(long, start + MARQUEE_STEP * 3).starts_with("Someone"));
    }

    #[test]
    fn the_windows_stack_and_roll_up() {
        let mut panels = Panels {
            eq: true,
            playlist: true,
            main_shade: false,
            eq_shade: false,
            playlist_shade: false,
        };
        assert_eq!(panels.height(), 348);
        assert_eq!(panels.at(120), Some((Panel::Eq, 4)));
        panels.main_shade = true;
        panels.eq = false;
        assert_eq!(panels.height(), 14 + 116);
        assert_eq!(panels.at(20), Some((Panel::Playlist, 6)));
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
