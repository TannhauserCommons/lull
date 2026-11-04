//! The Classic window on Windows: a plain Win32 window, no webview. Like Winamp 2's windows it's a
//! borderless popup the skin cuts to shape (a window region, so clicks outside the skin's shapes
//! reach the window behind), here one window holding the main window, the equaliser and the
//! playlist stacked (as Webamp's window holds them). The skin is drawn at 1x (canvas.rs and a
//! module per window); only what changed since the last frame is scaled up, nearest neighbour so
//! pixels stay sharp at any size, a band of rows at a time into one small bitmap, with the
//! playlist's names drawn over it at the screen's own size in the skin's font (GDI), as Webamp's
//! are, and copied to the window. Nothing the size of the window is kept: at 3x a whole-window
//! bitmap would be 3.4 MB, and a layered window's compositor keeps a copy of it too.
//!
//! It lives on the main thread, whose message loop (Tauri's) delivers its messages. Its state sits
//! in a thread-local; Windows calls back into the window procedure while it runs (capture changes,
//! the move loop, menus), so the state is borrowed only to decide what to do, and the calls that
//! may call back run after the borrow ends.
#![allow(unsafe_code)]

use super::canvas::{Canvas, TextRun};
use super::draw::{self, Control, Focus, Target, View};
use super::engine::{Command, Snapshot, Status};
use super::eq::{self, presets};
use super::eqwin;
use super::playlist::{self, List};
use super::queue::Repeat;
use super::session::Session;
use super::skin::{HEIGHT, Skin, WIDTH, layout};
use super::vis::{Analyser, BARS, Bar, FFT_SAMPLES};
use std::cell::RefCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1DCRenderTarget, ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_METRICS, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
    IDWriteInlineObject, IDWriteTextFormat, IDWriteTextLayout, IDWriteTextLayout1,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, ClientToScreen, CombineRgn, CreateCompatibleDC,
    CreateDIBSection, CreateRectRgn, DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, GdiFlush, GetDC, HBITMAP, HDC,
    HGDIOBJ, PAINTSTRUCT, RGN_OR, SRCCOPY, SelectObject, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_DELETE, VK_DOWN, VK_LEFT, VK_NUMPAD0, VK_RIGHT, VK_SHIFT,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CS_OWNDC, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    HMENU, HTCAPTION, IDC_ARROW, KillTimer, LoadCursorW, LoadIconW, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    RegisterClassExW, SW_MINIMIZE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD, TrackPopupMenuEx, WM_CLOSE,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_PAINT, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_APPWINDOW, WS_MINIMIZEBOX,
    WS_POPUP, WS_SYSMENU,
};
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, w};

const CLASS: PCWSTR = w!("lull-classic");
const TIMER: usize = 1;
/// Rows of the window scaled and copied at a time: the band's height.
const BAND: i32 = 64;
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
const TEXT_SIZE: f32 = 9.0;
const TEXT_SPACING: f32 = 0.5;
/// Right of the songs' times (Webamp's `padding-right: 3px`).
const TIME_PADDING: f32 = 3.0;
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

impl Panel {
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
}

impl Panels {
    /// The windows that show: each with its top and height in the stack.
    fn stack(&self) -> Vec<(Panel, u32, u32)> {
        let rolled = |shade: bool, height: u32| if shade { layout::SHADE_HEIGHT } else { height };
        let (mut stack, mut top) = (Vec::new(), 0);
        if self.main {
            top = rolled(self.main_shade, HEIGHT);
            stack.push((Panel::Main, 0, top));
        }
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

    /// Whether a window shows (Webamp's windows open or closed).
    fn shown(&mut self, panel: Panel) -> &mut bool {
        match panel {
            Panel::Main => &mut self.main,
            Panel::Eq => &mut self.eq,
            Panel::Playlist => &mut self.playlist,
        }
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
    /// A band of the window at the screen's size, its whole width and `BAND` rows: what changed is
    /// scaled into it and copied to the window a band at a time.
    band: Surface,
    /// The window's own DC (its class's CS_OWNDC), for as long as the window lives.
    screen: HDC,
    /// The windows' layout the shape Windows cuts the window to was made for: none since the size
    /// or the skin changed. Skins' sheets are opaque, so only the layout and the skin change it.
    shape: Option<Panels>,
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
    /// The analyser's last bars: paused, they hold still, as Webamp's do.
    bars: [Bar; BARS],
    /// The selected window of the three (Webamp's focus).
    focus: Focus,
    /// The control held down and its window's top in the stack; for a button, whether the pointer
    /// is still on it.
    pressed: Option<(Target, u32)>,
    over: bool,
    seeking: Option<f64>,
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
        main: true,
        eq: true,
        playlist: true,
        main_shade: false,
        eq_shade: false,
        playlist_shade: false,
    };
    let scale = setup.size * setup.dpi;
    let (columns, rows) = (edges(WIDTH, scale), edges(panels.height(), scale));
    let size = (columns[WIDTH as usize], rows[rows.len() - 1]);
    let text = Text::new(&setup.skin.playlist.font, scale)?;
    let hwnd = create(setup.at, size)?;
    crate::desktop::platform::add_thumb_buttons_to(setup.session.app(), hwnd.0 as isize);
    let mut session = setup.session;
    match super::smtc::Overlay::new(hwnd, session.app().clone()) {
        Ok(overlay) => session.attach_overlay(overlay),
        Err(e) => eprintln!("lull: no media keys for Classic: {e}"),
    }
    // SAFETY: a window this thread just made; its class owns the DC, so it is never released.
    let screen = unsafe { GetDC(Some(hwnd)) };
    let band = match Surface::new((size.0, BAND)) {
        Ok(band) => band,
        Err(e) => {
            // SAFETY: the window was just created on this thread and nothing else holds it.
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(e);
        }
    };
    let mut classic = Classic {
        hwnd,
        text,
        skin: setup.skin,
        session,
        on_close: Some(on_close),
        favourites: setup.favourites,
        band,
        screen,
        shape: None,
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
        bars: [Bar::default(); BARS],
        focus: Focus::Main,
        pressed: None,
        over: false,
        seeking: None,
        balance: 0.0,
        remaining: false,
        marquee: Marquee::default(),
        notice: setup.said.map(|said| (said, Instant::now())),
        anchor: 0,
        dropping: None,
        paused_since: None,
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
                classic.retext(&skin.playlist.font);
                classic.skin = skin;
                classic.notice = said.map(|said| (said, Instant::now()));
                classic.shown.clear();
                classic.shape = None;
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
            // Its own DC, kept: a window cut to a region otherwise works out what of it shows on
            // every GetDC, about as long as the frame's copy itself.
            style: CS_DBLCLKS | CS_OWNDC,
            lpfnWndProc: Some(procedure),
            hInstance: module.into(),
            hIcon: LoadIconW(Some(module.into()), PCWSTR(APP_ICON as usize as *const u16)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
        CreateWindowExW(
            WS_EX_APPWINDOW,
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
            WM_PAINT => {
                self.repaint();
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
            WM_KEYDOWN => self.key(wparam.0 as u16, false),
            WM_SYSKEYDOWN => self.key(wparam.0 as u16, true),
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
        self.focus = panel.focus();
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

    /// A window on or off, but never the last one: with nothing showing, nothing could be clicked.
    /// The selected window, closed, gives way to the first that shows.
    fn toggle_window(&mut self, panel: Panel) {
        let open = !*self.panels.shown(panel);
        *self.panels.shown(panel) = open;
        let stack = self.panels.stack();
        let Some((first, _, _)) = stack.first() else {
            *self.panels.shown(panel) = true;
            return;
        };
        if !stack.iter().any(|(p, _, _)| p.focus() == self.focus) {
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
            Control::PlaylistMenu(1) => return Effect::Menu(Menu::Remove, below),
            Control::PlaylistMenu(2) => return Effect::Menu(Menu::Select, below),
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
        let font = self.skin.playlist.font.clone();
        self.retext(&font);
        self.rows.clear();
    }

    /// The playlist's text in `font` at the window's size; failing that, as it was.
    fn retext(&mut self, font: &str) {
        match Text::new(font, self.size * self.dpi) {
            Ok(text) => self.text = text,
            Err(e) => eprintln!("lull: Classic playlist text: {e}"),
        }
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

    /// Webamp's keys (its hotkeys, the numeric keypad's too): Z X C V B and the keypad's 4 5 6 for
    /// the transport, R and S for repeat and shuffle, arrows (keypad 7 9 8 2) for 5 seconds and the
    /// volume, keypad 1 and 3 for ten songs back or on; Ctrl+D Double Size, Ctrl+R the list
    /// reversed, Ctrl+T the time counting down; Alt+W, Alt+G and Alt+E a window on or off. Delete
    /// takes the selected songs out.
    fn key(&mut self, key: u16, alt: bool) -> Effect {
        let (position, volume) = (self.snapshot.position, self.snapshot.volume);
        // SAFETY: reads this thread's keyboard state.
        let control = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        let is = |c: u8| key == u16::from(c);
        let pad = |n: u16| key == VK_NUMPAD0.0 + n;
        if alt {
            let panel = match key {
                _ if is(b'W') => Panel::Main,
                _ if is(b'G') => Panel::Eq,
                _ if is(b'E') => Panel::Playlist,
                _ => return Effect::Default,
            };
            self.toggle_window(panel);
            return Effect::Handled;
        }
        if control {
            match key {
                _ if is(b'D') => self.double_size(),
                _ if is(b'R') => {
                    let rows: Vec<usize> = (0..self.list.labels.len()).rev().collect();
                    self.send(Command::Reorder(rows, 0));
                }
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
            k if k == VK_LEFT.0 || pad(7) => Some(Command::Seek((position - 5.0).max(0.0))),
            k if k == VK_RIGHT.0 || pad(9) => Some(Command::Seek(position + 5.0)),
            k if k == VK_UP.0 || pad(8) => {
                self.volume(volume + 0.01);
                None
            }
            k if k == VK_DOWN.0 || pad(2) => {
                self.volume(volume - 0.01);
                None
            }
            k if k == VK_DELETE.0 => {
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
        let height = self.panels.height();
        self.canvas.resize(height);
        if self.rows.len() != height as usize + 1 {
            self.resize(height);
        }
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
            draw_panel(
                &mut self.canvas,
                &self.skin,
                (panel, self.panels.shaded(panel)),
                top,
                rows,
                &view,
            );
        }
        if self.shape != Some(self.panels) {
            self.reshape();
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
        let moving = now.playing || self.seeking.is_some() || self.pressed.is_some();
        let tick = if moving { ANIMATE } else { IDLE };
        if tick != self.tick {
            self.tick = tick;
            // SAFETY: a window this thread owns; replaces the running timer.
            unsafe { SetTimer(Some(self.hwnd), TIMER, tick.as_millis() as u32, None) };
        }
    }

    /// The stack's height or the size changed: the window takes its new size, and all of it is
    /// drawn again.
    fn resize(&mut self, height: u32) {
        let scale = self.size * self.dpi;
        self.columns = edges(WIDTH, scale);
        self.rows = edges(height, scale);
        let size = (self.columns[WIDTH as usize], self.rows[height as usize]);
        if size.0 != self.band.size.0 {
            match Surface::new((size.0, BAND)) {
                Ok(band) => self.band = band,
                Err(e) => eprintln!("lull: Classic window: {e}"),
            }
        }
        // SAFETY: a window this thread owns; only its size changes.
        let resized = unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                size.0,
                size.1,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if let Err(e) = resized {
            eprintln!("lull: Classic window didn't resize: {e}");
        }
        // What was on screen was another shape: nothing of it is compared with, all of it is drawn.
        self.shown.clear();
        self.shown_texts.clear();
        self.prints = [None; 3];
        self.shape = None;
    }

    /// Cuts the window to the skin's shapes as drawn (outside them the canvas is transparent); a
    /// window drawn with no transparency is a plain rectangle.
    fn reshape(&mut self) {
        self.shape = Some(self.panels);
        let region = shape(&self.canvas.pixels).map(|pieces| {
            // SAFETY: regions this makes; each piece is deleted once merged into `whole`, which
            // Windows owns once it is the window's.
            unsafe {
                let whole = CreateRectRgn(0, 0, 0, 0);
                for (x0, y0, x1, y1) in pieces {
                    let piece = CreateRectRgn(
                        self.columns[x0 as usize],
                        self.rows[y0 as usize],
                        self.columns[x1 as usize],
                        self.rows[y1 as usize],
                    );
                    CombineRgn(Some(whole), Some(whole), Some(piece), RGN_OR);
                    let _ = DeleteObject(piece.into());
                }
                whole
            }
        });
        // SAFETY: a window this thread owns; Windows takes the region (none: the whole window).
        if unsafe { SetWindowRgn(self.hwnd, region, false) } == 0 {
            eprintln!("lull: Classic window's shape: {}", windows::core::Error::from_thread());
        }
    }

    /// Shows skin pixels `[x0, x1) x [y0, y1)` of the canvas in the window now.
    fn present(&mut self, dirty: (u32, u32, u32, u32)) {
        self.paint(self.screen, dirty);
    }

    /// Windows asks for a part of the window again (shown, restored, uncovered).
    fn repaint(&mut self) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: a window this thread owns; EndPaint ends what BeginPaint began.
        unsafe {
            let dc = BeginPaint(self.hwnd, &mut paint);
            let r = paint.rcPaint;
            if r.right > r.left && r.bottom > r.top && !self.shown.is_empty() {
                let x0 = skin_at(r.left, &self.columns);
                let x1 = (skin_at(r.right - 1, &self.columns) + 1).min(WIDTH);
                let y0 = skin_at(r.top, &self.rows);
                let y1 = (skin_at(r.bottom - 1, &self.rows) + 1).min(self.canvas.height);
                if x1 > x0 && y1 > y0 {
                    self.paint(dc, (x0, y0, x1, y1));
                }
            }
            let _ = EndPaint(self.hwnd, &paint);
        }
    }

    fn paint(&mut self, dc: HDC, rect: (u32, u32, u32, u32)) {
        paint(
            &self.canvas,
            (&self.columns, &self.rows),
            &mut self.band,
            &self.text,
            dc,
            rect,
        );
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

/// One window of the stack (shaded or not) into the canvas, `rows` high from `top`.
fn draw_panel(canvas: &mut Canvas, skin: &Skin, (panel, shaded): (Panel, bool), top: u32, rows: u32, view: &View) {
    canvas.clear(top, rows);
    let mut pen = canvas.pen(skin, top);
    match (panel, shaded) {
        (Panel::Main, false) => draw::main(&mut pen, view),
        (Panel::Main, true) => draw::main_shaded(&mut pen, view),
        (Panel::Eq, false) => eqwin::window(&mut pen, view),
        (Panel::Eq, true) => eqwin::shaded(&mut pen, view),
        (Panel::Playlist, false) => playlist::window(&mut pen, view),
        (Panel::Playlist, true) => playlist::shaded(&mut pen, view),
    }
}

/// Scales skin pixels `[x0, x1) x [y0, y1)` of the canvas up into `band` (`columns` and `rows`: where
/// each skin pixel starts at the screen's size), a band of rows at a time, draws the playlist's text
/// over them at the screen's size, and copies each band to `dc`.
fn paint(
    canvas: &Canvas,
    (columns, rows): (&[i32], &[i32]),
    band: &mut Surface,
    text: &Text,
    dc: HDC,
    (x0, y0, x1, y1): (u32, u32, u32, u32),
) {
    let (left, right) = (columns[x0 as usize], columns[x1 as usize]);
    let (top, bottom) = (rows[y0 as usize], rows[y1 as usize]);
    let stride = band.size.0 as usize;
    let column = text.column(&canvas.texts);
    let mut from = top;
    while from < bottom {
        let to = (from + band.size.1).min(bottom);
        // SAFETY: GDI finishes with the band (the last copy out of it) before its pixels change.
        let _ = unsafe { GdiFlush() };
        let pixels = band.pixels();
        for py in from..to {
            let sy = skin_at(py, rows) as usize;
            let source = &canvas.pixels[sy * WIDTH as usize..(sy + 1) * WIDTH as usize];
            let at = (py - from) as usize * stride;
            let row = &mut pixels[at..at + stride];
            for sx in x0..x1 {
                let (a, b) = (
                    (columns[sx as usize] - left) as usize,
                    (columns[sx as usize + 1] - left) as usize,
                );
                row[a..b.min(stride)].fill(source[sx as usize]);
            }
        }
        let drawing = !canvas.texts.is_empty() && text.begin(band.dc, band.size);
        for run in canvas.texts.iter().filter(|_| drawing) {
            let area = run.area;
            let at = RECT {
                left: columns[area.x as usize] - left,
                top: rows[area.y as usize] - from,
                right: columns[(area.x + area.width) as usize] - left,
                bottom: rows[(area.y + area.height) as usize] - from,
            };
            if at.left < right - left && at.right > 0 && at.top < to - from && at.bottom > 0 {
                text.draw(run, at, column);
            }
        }
        if drawing {
            text.end();
        }
        // SAFETY: the band's bitmap is selected into its DC; `dc` is the window's.
        if let Err(e) = unsafe { BitBlt(dc, left, from, right - left, to - from, Some(band.dc), 0, 0, SRCCOPY) } {
            eprintln!("lull: Classic window didn't update: {e}");
        }
        from = to;
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

/// The canvas's drawn pixels as rectangles of skin pixels `(x0, y0, x1, y1)`, end exclusive: runs of
/// a row, each carried down while the rows below have the same run. None when nothing is
/// transparent (the window is a plain rectangle).
fn shape(pixels: &[u32]) -> Option<Vec<(u32, u32, u32, u32)>> {
    if pixels.iter().all(|p| p >> 24 != 0) {
        return None;
    }
    let width = WIDTH as usize;
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
        view.focus,
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

/// The playlist's text as the browser draws Webamp's: DirectWrite (through Direct2D, into the
/// band's DC), greyscale-smoothed, in the skin's font (`pledit.txt`) or Arial when this computer
/// hasn't it (Webamp's `font-family: <font>, Arial, sans-serif`), 9 px with 0.5 px between letters,
/// each 13 px row laid out as CSS lays out a line of that height; titles too long end in an
/// ellipsis, and characters the font hasn't come from the system's fallback fonts.
struct Text {
    write: IDWriteFactory,
    format: IDWriteTextFormat,
    ellipsis: IDWriteInlineObject,
    target: ID2D1DCRenderTarget,
    scale: f32,
}

impl Text {
    fn new(face: &str, scale: f64) -> Result<Self, String> {
        let scale = scale as f32;
        let size = TEXT_SIZE * scale;
        let line = layout::PLAYLIST_ROW_HEIGHT as f32 * scale;
        // SAFETY: plain factory calls; every out-pointer is a local that lives through its call.
        unsafe {
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| e.to_string())?;
            let fonts = {
                let mut fonts = None;
                write
                    .GetSystemFontCollection(&mut fonts, false)
                    .map_err(|e| e.to_string())?;
                fonts.ok_or("no system fonts")?
            };
            let (mut index, mut found) = (0, BOOL::default());
            fonts
                .FindFamilyName(&HSTRING::from(face), &mut index, &mut found)
                .map_err(|e| e.to_string())?;
            let family = if found.as_bool() { face } else { "Arial" };
            fonts
                .FindFamilyName(&HSTRING::from(family), &mut index, &mut found)
                .map_err(|e| e.to_string())?;
            // The line's baseline as CSS places it: half the spare height (rounded down, as Blink
            // does) above the font's ascent, the ascent and descent each rounded to a pixel as
            // Chrome rounds them.
            let baseline = if found.as_bool() {
                let font = fonts
                    .GetFontFamily(index)
                    .and_then(|f| {
                        f.GetFirstMatchingFont(
                            DWRITE_FONT_WEIGHT_NORMAL,
                            DWRITE_FONT_STRETCH_NORMAL,
                            DWRITE_FONT_STYLE_NORMAL,
                        )
                    })
                    .map_err(|e| e.to_string())?;
                let mut metrics = DWRITE_FONT_METRICS::default();
                font.GetMetrics(&mut metrics);
                let em = f32::from(metrics.designUnitsPerEm.max(1));
                let ascent = (f32::from(metrics.ascent) / em * size).round();
                let descent = (f32::from(metrics.descent) / em * size).round();
                ((line - ascent - descent) / 2.0).floor() + ascent
            } else {
                line * 0.75
            };
            let format = write
                .CreateTextFormat(
                    &HSTRING::from(family),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
                .map_err(|e| e.to_string())?;
            format
                .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
                .map_err(|e| e.to_string())?;
            format
                .SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, baseline)
                .map_err(|e| e.to_string())?;
            let ellipsis = write.CreateEllipsisTrimmingSign(&format).map_err(|e| e.to_string())?;
            let draw: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(|e| e.to_string())?;
            let target = draw
                .CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                })
                .map_err(|e| e.to_string())?;
            Ok(Self {
                write,
                format,
                ellipsis,
                target,
                scale,
            })
        }
    }

    /// `text` laid out in `width` (a single line), letters spaced.
    fn layout(&self, text: &str, width: f32, height: f32) -> windows::core::Result<IDWriteTextLayout> {
        let words: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: the words outlive the call; the layout is this function's own.
        unsafe {
            let layout = self
                .write
                .CreateTextLayout(&words, &self.format, width.max(0.0), height)?;
            let range = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: words.len() as u32,
            };
            layout
                .cast::<IDWriteTextLayout1>()?
                .SetCharacterSpacing(0.0, TEXT_SPACING * self.scale, 0.0, range)?;
            Ok(layout)
        }
    }

    /// The durations' column, as wide as the widest time in sight and its 3 px padding (Webamp's
    /// flex column), in physical pixels.
    fn column(&self, runs: &[TextRun]) -> f32 {
        let widest = runs
            .iter()
            .filter(|run| run.right)
            .filter_map(|run| {
                let layout = self.layout(&run.text, f32::MAX, f32::MAX).ok()?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                // SAFETY: a layout this made, measured into a local.
                unsafe { layout.GetMetrics(&mut metrics).ok()? };
                Some(metrics.widthIncludingTrailingWhitespace)
            })
            .fold(0.0, f32::max);
        if widest > 0.0 {
            widest + TIME_PADDING * self.scale
        } else {
            0.0
        }
    }

    /// Begins drawing into `dc` (the band's, `size` big); `false` when it can't.
    fn begin(&self, dc: HDC, size: (i32, i32)) -> bool {
        let all = RECT {
            left: 0,
            top: 0,
            right: size.0,
            bottom: size.1,
        };
        // SAFETY: `dc` has the band's bitmap selected for as long as this drawing lasts.
        match unsafe { self.target.BindDC(dc, &all) } {
            Ok(()) => {
                // SAFETY: the target is bound; EndDraw (`end`) follows.
                unsafe {
                    self.target.BeginDraw();
                    self.target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                }
                true
            }
            Err(e) => {
                eprintln!("lull: Classic playlist text: {e}");
                false
            }
        }
    }

    /// One row's title (left, ending in an ellipsis before the durations' `column`) or time
    /// (right, inside the column's padding), in `at`.
    fn draw(&self, run: &TextRun, at: RECT, column: f32) {
        let (left, right, align) = if run.right {
            (
                at.right as f32 - column,
                at.right as f32 - TIME_PADDING * self.scale,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            )
        } else {
            (at.left as f32, at.right as f32 - column, DWRITE_TEXT_ALIGNMENT_LEADING)
        };
        let height = (at.bottom - at.top) as f32;
        let Ok(layout) = self.layout(&run.text, right - left, height) else {
            return;
        };
        let [r, g, b] = run.color.map(|c| f32::from(c) / 255.0);
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        // SAFETY: a layout and a brush this made, drawn between `begin` and `end`.
        let drawn = unsafe {
            layout
                .SetTextAlignment(align)
                .and_then(|()| {
                    if run.right {
                        Ok(())
                    } else {
                        layout.SetTrimming(&trimming, &self.ellipsis)
                    }
                })
                .and_then(|()| {
                    self.target
                        .CreateSolidColorBrush(&D2D1_COLOR_F { r, g, b, a: 1.0 }, None)
                })
                .map(|brush| {
                    self.target.DrawTextLayout(
                        windows_numerics::Vector2 {
                            X: left,
                            Y: at.top as f32,
                        },
                        &layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                    );
                })
        };
        if let Err(e) = drawn {
            eprintln!("lull: Classic playlist text: {e}");
        }
    }

    fn end(&self) {
        // SAFETY: ends the drawing `begin` began.
        if let Err(e) = unsafe { self.target.EndDraw(None, None) } {
            eprintln!("lull: Classic playlist text: {e}");
        }
    }
}

/// A bitmap: physical size, 32-bit BGRA, top row first, in a memory DC (the window's band).
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
    use super::super::session::Now;
    use super::*;

    /// The side-by-side with Webamp (tests/ui/classic-compare.mjs): every `.wsz` in
    /// `LULL_SKINS_IN` drawn as the window draws it at 1x, the three windows stacked, one song
    /// loaded and stopped, the equaliser on and flat, in the scene `LULL_SCENE` names (the
    /// harness's: `stopped`, `sliders` or `shaded`), into `<name>.png` in `LULL_RENDERS_OUT`. A skin missing pieces takes them from base-2.91 (`BASE_MD5.wsz`, in the
    /// same folder), as the window does. Run by hand: `cargo test render_skins -- --ignored`.
    #[test]
    #[ignore = "renders a folder of skins: LULL_SKINS_IN, LULL_RENDERS_OUT"]
    fn render_skins() -> Result<(), String> {
        let (inn, out) = match (std::env::var("LULL_SKINS_IN"), std::env::var("LULL_RENDERS_OUT")) {
            (Ok(i), Ok(o)) => (std::path::PathBuf::from(i), std::path::PathBuf::from(o)),
            _ => return Err("set LULL_SKINS_IN and LULL_RENDERS_OUT".into()),
        };
        let read = |p: &std::path::Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
        let base = Skin::from_archive(&read(&inn.join(format!("{BASE_MD5}.wsz")))?, None)?;
        // Volume, balance, and whether the main window and the equaliser are rolled up.
        let (volume, balance, shaded) = match std::env::var("LULL_SCENE").as_deref() {
            Err(_) | Ok("stopped") => (0.75, 0.0, false),
            Ok("sliders") => (0.3, -0.6, false),
            Ok("shaded") => (0.3, -0.6, true),
            Ok(other) => return Err(format!("no scene {other}: stopped, sliders or shaded")),
        };
        let panels = Panels {
            main: true,
            eq: true,
            playlist: true,
            main_shade: shaded,
            eq_shade: shaded,
            playlist_shade: false,
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
            labels: vec![format!("1. {SCENE_ARTIST} - {SCENE_TITLE}")],
            lengths: vec![Some(SCENE_SECONDS)],
            selected: vec![false],
            scroll: 0,
        };
        let bars = [Bar::default(); BARS];
        let marquee = format!("1. {SCENE_ARTIST} - {SCENE_TITLE} (4:40)");
        let view = View {
            now: &now,
            marquee: &marquee,
            volume,
            balance,
            focus: Focus::Main,
            pressed: None,
            seeking: None,
            remaining: false,
            blink_off: false,
            bars: &bars,
            shuffle: false,
            repeat: false,
            eq_open: true,
            playlist_open: true,
            eq: &eq,
            list: &list,
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
            let height = panels.height();
            let mut canvas = Canvas::new(height);
            for (panel, top, rows) in panels.stack() {
                draw_panel(&mut canvas, &skin, (panel, panels.shaded(panel)), top, rows, &view);
            }
            let (columns, rows) = (edges(WIDTH, 1.0), edges(height, 1.0));
            let size = (WIDTH as i32, height as i32);
            let (mut band, mut target) = (Surface::new(size)?, Surface::new(size)?);
            let text = Text::new(&skin.playlist.font, 1.0)?;
            paint(
                &canvas,
                (&columns, &rows),
                &mut band,
                &text,
                target.dc,
                (0, 0, WIDTH, height),
            );
            // SAFETY: the copy into `target` is done before its pixels are read.
            let _ = unsafe { GdiFlush() };
            let rgba: Vec<u8> = target
                .pixels()
                .iter()
                .zip(&canvas.pixels)
                .flat_map(|(p, drawn)| {
                    let alpha = if drawn >> 24 == 0 { 0 } else { 255 };
                    [(p >> 16) as u8, (p >> 8) as u8, *p as u8, alpha]
                })
                .collect();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            image::save_buffer(
                out.join(format!("{name}.png")),
                &rgba,
                WIDTH,
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
    const SCENE_ARTIST: &str = "Velvet Comet";
    const SCENE_TITLE: &str = "Static Season";
    const SCENE_SECONDS: f64 = 280.0;
    const BASE_MD5: &str = "5e4f10275dcb1fb211d4a8b4f1bda236";

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
    fn a_window_closed_leaves_the_others_stacked_and_the_last_stays() {
        let mut panels = Panels {
            main: false,
            eq: true,
            playlist: true,
            main_shade: false,
            eq_shade: true,
            playlist_shade: false,
        };
        let stack: Vec<_> = panels.stack().iter().map(|(p, top, _)| (*p, *top)).collect();
        assert_eq!(stack, [(Panel::Eq, 0), (Panel::Playlist, layout::SHADE_HEIGHT)]);
        panels.eq = false;
        panels.playlist = false;
        assert!(panels.stack().is_empty());
    }

    #[test]
    fn the_window_is_cut_to_what_the_skin_draws() {
        let width = WIDTH as usize;
        let mut pixels = vec![0xff00_0000u32; width * 6];
        assert_eq!(shape(&pixels), None, "drawn everywhere: no shape");
        // A notch in rows 2 and 3, columns 10 to 19, and the last row only half drawn.
        for y in 2..4 {
            pixels[y * width + 10..y * width + 20].fill(0);
        }
        pixels[5 * width + 100..6 * width].fill(0);
        let mut pieces = shape(&pixels).unwrap_or_default();
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
        // Padded to the marquee's 31 cells with blanks, as Webamp's is.
        assert_eq!(marquee.show("1. Short", start), format!("{:<31}", "1. Short"));
        let long = "1. Someone - A title far too long for Winamp's marquee";
        assert!(marquee.show(long, start).starts_with("1. Some"));
        assert!(marquee.show(long, start + MARQUEE_STEP * 3).starts_with("Someone"));
    }

    #[test]
    fn the_windows_stack_and_roll_up() {
        let mut panels = Panels {
            main: true,
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
