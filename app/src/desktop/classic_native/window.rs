//! The Classic window on Windows: a plain Win32 window, no webview, holding ui.rs's stack. Like
//! Winamp 2's windows it's a borderless popup the skin cuts to shape (a window region, so clicks
//! outside the skin's shapes reach the window behind). Only what changed since the last frame is
//! scaled up, nearest neighbour so pixels stay sharp at any size, a band of rows at a time into one
//! small bitmap, with the playlist's names drawn over it at the screen's own size (DirectWrite, as
//! the browser draws Webamp's), and copied to the window. Nothing the size of the window is kept:
//! at 3x a whole-window bitmap would be 3.4 MB, and a layered window's compositor keeps a copy of
//! it too.
//!
//! It lives on the main thread, whose message loop (Tauri's) delivers its messages. Its state sits
//! in a thread-local; Windows calls back into the window procedure while it runs (capture changes,
//! the move loop, menus), so the state is borrowed only to decide what to do, and the calls that
//! may call back run after the borrow ends.
#![allow(unsafe_code)]

use super::canvas::{Canvas, TextRun};
use super::dock::Rect;
use super::playlist::{TEXT_SIZE, TEXT_SPACING, TIME_PADDING, baseline};
use super::skin::{Skin, layout};
use super::ui::{Classic, Effect, Frame, Item, Key, Menu, Mods, PaneFrame, Panel, Shape, scale_rows};
pub use super::ui::{OnClose, Setup};
use std::cell::RefCell;
use std::ffi::c_void;
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
    CreateDIBSection, CreateRectRgn, DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, EnumDisplayMonitors, GdiFlush,
    GetDC, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, PAINTSTRUCT, RGN_OR, SRCCOPY, SelectObject,
    SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_DELETE, VK_DOWN, VK_LEFT, VK_NUMPAD0, VK_NUMPAD9, VK_RIGHT,
    VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CS_OWNDC, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    HMENU, HTCAPTION, IDC_ARROW, KillTimer, LoadCursorW, LoadIconW, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    RegisterClassExW, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD, TrackPopupMenuEx,
    WINDOW_EX_STYLE, WM_CLOSE, WM_DISPLAYCHANGE, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDOWN, WM_PAINT, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_SYSKEYDOWN, WM_TIMER,
    WNDCLASSEXW, WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, w};

const CLASS: PCWSTR = w!("lull-classic");
const TIMER: usize = 1;
/// Rows of the window scaled and copied at a time: the band's height.
const BAND: i32 = 64;
/// Tauri's build puts the app's icon in the executable under this id (IDI_APPLICATION's).
const APP_ICON: u16 = 32512;

thread_local! {
    static OPEN: RefCell<Option<Window>> = const { RefCell::new(None) };
}

/// The stack and the Win32 windows showing it, one a panel.
struct Window {
    classic: Classic,
    /// The main window's (on the taskbar), the equaliser's and the playlist's, which the main one
    /// owns: they minimise with it and stay off the taskbar.
    panes: [Pane; 3],
    /// A band of a window at the screen's size, the widest window's width and `BAND` rows: what
    /// changed is scaled into it and copied to its window a band at a time.
    band: Surface,
    text: Text,
    /// The font and scale `text` was made for.
    text_style: (String, f64),
}

/// One Win32 window, and what it shows now.
struct Pane {
    hwnd: HWND,
    /// Its own DC (its class's CS_OWNDC), for as long as it lives.
    screen: HDC,
    shown: bool,
    at: (i32, i32),
    size: (i32, i32),
}

/// Opens the windows. Main thread only.
pub fn open(setup: Setup, on_close: OnClose) -> Result<(), String> {
    if OPEN.with_borrow(Option::is_some) {
        return Err("Classic mode is open already".into());
    }
    let mut classic = Classic::new(setup, on_close);
    let (font, scale) = classic.text_style();
    let text_style = (font.to_owned(), scale);
    let text = Text::new(font, scale)?;
    let main = create(None)?;
    let mut hwnds = vec![main];
    for _ in 1..3 {
        match create(Some(main)) {
            Ok(hwnd) => hwnds.push(hwnd),
            Err(e) => {
                destroy(&hwnds);
                return Err(e);
            }
        }
    }
    crate::desktop::platform::add_thumb_buttons_to(classic.session.app(), main.0 as isize);
    match super::smtc::Overlay::new(main, classic.session.app().clone()) {
        Ok(overlay) => classic.session.attach_overlay(overlay),
        Err(e) => eprintln!("lull: no media keys for Classic: {e}"),
    }
    let band = match Surface::new((1, BAND)) {
        Ok(band) => band,
        Err(e) => {
            destroy(&hwnds);
            return Err(e);
        }
    };
    // SAFETY: windows this thread just made; their class owns their DCs, never released.
    let panes = [0, 1, 2].map(|i| Pane {
        hwnd: hwnds[i],
        screen: unsafe { GetDC(Some(hwnds[i])) },
        shown: false,
        at: (i32::MIN, i32::MIN),
        size: (0, 0),
    });
    classic.set_areas(work_areas());
    let mut window = Window {
        classic,
        panes,
        band,
        text,
        text_style,
    };
    window.render();
    OPEN.with_borrow_mut(|open| *open = Some(window));
    // SAFETY: a window this thread owns.
    let _ = unsafe { SetForegroundWindow(main) };
    Ok(())
}

/// Destroys windows this thread made.
fn destroy(hwnds: &[HWND]) {
    for &hwnd in hwnds.iter().rev() {
        // SAFETY: a window this thread made, held by nothing else.
        let _ = unsafe { DestroyWindow(hwnd) };
    }
}

/// The desktop's monitors' work areas (the screen less the taskbar), physical pixels.
fn work_areas() -> Vec<Rect> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, areas: LPARAM) -> BOOL {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `areas` is the Vec `work_areas` passed for this call; the monitor is Windows' own.
        unsafe {
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let r = info.rcWork;
                let areas = &mut *(areas.0 as *mut Vec<Rect>);
                areas.push(Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top));
            }
        }
        BOOL(1)
    }
    let mut areas: Vec<Rect> = Vec::new();
    // SAFETY: the callback only runs during this call, while `areas` lives.
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut areas as *mut Vec<Rect> as isize)) };
    areas
}

/// Brings the windows forward (the tray, a second launch). Main thread only.
pub fn show() {
    if let Some(main) = OPEN.with_borrow(|open| open.as_ref().map(|w| w.panes[0].hwnd)) {
        // SAFETY: a window this thread owns.
        unsafe {
            let _ = ShowWindow(main, SW_RESTORE);
            let _ = SetForegroundWindow(main);
        }
    }
}

/// The tray's and the taskbar's buttons, the system's media keys. Main thread only.
pub fn act(action: &str) {
    let acted = OPEN.with_borrow_mut(|open| {
        let window = open.as_mut()?;
        window.classic.act(action);
        window.render();
        Some(())
    });
    if acted.is_none() {
        eprintln!("lull: Classic is closed: no {action}");
    }
}

/// Another skin, fetched for the Options menu; or why it couldn't be. Main thread only.
pub fn set_skin(skin: Result<(Skin, Option<String>), String>) {
    OPEN.with_borrow_mut(|open| {
        if let Some(window) = open.as_mut() {
            window.classic.set_skin(skin);
            window.render();
        }
    });
}

/// Leaves Classic as its close button does. Main thread only.
pub fn close() {
    let Some(window) = OPEN.with_borrow_mut(Option::take) else {
        return;
    };
    // SAFETY: windows this thread owns; their state is out of the thread-local already, so the
    // messages destroying them send find nothing to borrow.
    let _ = unsafe { KillTimer(Some(window.panes[0].hwnd), TIMER) };
    destroy(&window.panes.each_ref().map(|p| p.hwnd));
    let mut classic = window.classic;
    let (size, layout) = (classic.size, classic.layout());
    if let Some(on_close) = classic.on_close.take() {
        on_close(classic.session, size, layout);
    }
}

/// One of the windows: the main one, or one it owns.
fn create(owner: Option<HWND>) -> Result<HWND, String> {
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
        let taskbar = if owner.is_none() {
            WS_EX_APPWINDOW
        } else {
            WINDOW_EX_STYLE(0)
        };
        CreateWindowExW(
            taskbar,
            CLASS,
            w!("lull classic"),
            WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            0,
            0,
            1,
            1,
            owner,
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
        Ok(Some(window)) => match window.panes.iter().position(|p| p.hwnd == hwnd) {
            Some(i) => window.handle(Panel::ALL[i], message, wparam, lparam),
            None => Effect::Default,
        },
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
                if let Some(main) = OPEN.with_borrow(|open| open.as_ref().map(|w| w.panes[0].hwnd)) {
                    let _ = ShowWindow(main, SW_MINIMIZE);
                }
                LRESULT(0)
            }
            Effect::Close => {
                let _ = ReleaseCapture();
                close();
                LRESULT(0)
            }
            Effect::Menu(menu, (x, y)) => {
                let _ = ReleaseCapture();
                let mut at = POINT { x, y };
                let _ = ClientToScreen(hwnd, &mut at);
                pop_up(hwnd, menu, at);
                LRESULT(0)
            }
        }
    }
}

/// Shows a menu and, once the listener has chosen, does what they chose.
fn pop_up(hwnd: HWND, menu: Menu, at: POINT) {
    let Some(items) = OPEN.with_borrow(|open| open.as_ref().map(|w| w.classic.menu_items(menu))) else {
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
    let effect = OPEN.with_borrow_mut(|open| {
        open.as_mut().map(|w| {
            let effect = w.classic.chose(menu, chosen);
            w.render();
            effect
        })
    });
    if let Some(Effect::Close) = effect {
        close();
    }
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

impl Window {
    fn handle(&mut self, panel: Panel, message: u32, wparam: WPARAM, lparam: LPARAM) -> Effect {
        let hwnd = self.panes[panel as usize].hwnd;
        let at = coordinates(lparam);
        let screen = || {
            let mut point = POINT { x: at.0, y: at.1 };
            // SAFETY: a window this thread owns, and a point to convert in place.
            let _ = unsafe { ClientToScreen(hwnd, &mut point) };
            (point.x, point.y)
        };
        let effect = match message {
            WM_TIMER => Effect::Handled,
            WM_PAINT => {
                self.repaint(panel);
                return Effect::Handled;
            }
            WM_LBUTTONDOWN => self.classic.pointer_down(panel, at, screen(), false, mods()),
            WM_LBUTTONDBLCLK => self.classic.pointer_down(panel, at, screen(), true, mods()),
            WM_MOUSEMOVE => {
                self.classic.pointer_move(panel, at, screen());
                Effect::Handled
            }
            WM_LBUTTONUP => self.classic.pointer_up(panel, at, screen()),
            WM_RBUTTONUP => self.classic.right_click(at),
            WM_MOUSEWHEEL => {
                let notches = f64::from((wparam.0 >> 16) as u16 as i16) / 120.0;
                self.classic.wheel(panel, notches)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => match key(wparam.0 as u16) {
                Some(key) => self.classic.key(
                    key,
                    Mods {
                        alt: message == WM_SYSKEYDOWN,
                        ..mods()
                    },
                ),
                None => Effect::Default,
            },
            // A monitor added, gone or resized, the taskbar moved: no window stays off them all.
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                self.classic.set_areas(work_areas());
                Effect::Default
            }
            WM_CLOSE => Effect::Close,
            _ => return Effect::Default,
        };
        self.render();
        effect
    }

    /// A frame of the stack, and what it says the windows do about it.
    fn render(&mut self) {
        let Frame { panes, tick } = self.classic.render();
        let (font, scale) = self.classic.text_style();
        if (font, scale) != (self.text_style.0.as_str(), self.text_style.1) {
            match Text::new(font, scale) {
                Ok(text) => self.text = text,
                Err(e) => eprintln!("lull: Classic playlist text: {e}"),
            }
            self.text_style = (font.to_owned(), scale);
        }
        let widest = self.classic.widest();
        if widest != self.band.size.0 {
            match Surface::new((widest, BAND)) {
                Ok(band) => self.band = band,
                Err(e) => eprintln!("lull: Classic window: {e}"),
            }
        }
        for (i, frame) in panes.into_iter().enumerate() {
            self.apply(Panel::ALL[i], frame);
        }
        if let Some(tick) = tick {
            // SAFETY: a window this thread owns; replaces the running timer.
            unsafe { SetTimer(Some(self.panes[0].hwnd), TIMER, tick.as_millis() as u32, None) };
        }
    }

    /// A window's part of a frame: shown or hidden, its place, size, shape and what changed.
    fn apply(&mut self, panel: Panel, frame: PaneFrame) {
        let pane = &mut self.panes[panel as usize];
        let hwnd = pane.hwnd;
        if !frame.shown {
            if pane.shown {
                pane.shown = false;
                // SAFETY: a window this thread owns.
                let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
            return;
        }
        if let Some(size) = frame.size {
            pane.size = size;
        }
        if frame.size.is_some() || frame.at != pane.at {
            pane.at = frame.at;
            let (x, y, (w, h)) = (pane.at.0, pane.at.1, pane.size);
            // SAFETY: a window this thread owns; only its place and size change.
            if let Err(e) = unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) } {
                eprintln!("lull: Classic window didn't move: {e}");
            }
        }
        if let Some(shape) = frame.shape {
            reshape(hwnd, shape);
        }
        if let Some(dirty) = frame.dirty {
            let dc = self.panes[panel as usize].screen;
            self.paint(panel, dc, dirty);
        }
        let pane = &mut self.panes[panel as usize];
        if !pane.shown {
            pane.shown = true;
            // SAFETY: a window this thread owns.
            let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        }
    }

    /// Windows asks for a part of a window again (shown, restored, uncovered).
    fn repaint(&mut self, panel: Panel) {
        let hwnd = self.panes[panel as usize].hwnd;
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: a window this thread owns; EndPaint ends what BeginPaint began.
        unsafe {
            let dc = BeginPaint(hwnd, &mut paint);
            let r = paint.rcPaint;
            if let Some(rect) = self.classic.covering(panel, (r.left, r.top, r.right, r.bottom)) {
                self.paint(panel, dc, rect);
            }
            let _ = EndPaint(hwnd, &paint);
        }
    }

    fn paint(&mut self, panel: Panel, dc: HDC, rect: (u32, u32, u32, u32)) {
        let pane = self.classic.pane(panel);
        paint(
            &pane.canvas,
            (&pane.columns, &pane.rows),
            &mut self.band,
            &self.text,
            dc,
            rect,
        );
    }
}

/// Cuts a window to these rectangles; none: the whole window.
fn reshape(hwnd: HWND, pieces: Shape) {
    let region = pieces.map(|pieces| {
        // SAFETY: regions this makes; each piece is deleted once merged into `whole`, which
        // Windows owns once it is the window's.
        unsafe {
            let whole = CreateRectRgn(0, 0, 0, 0);
            for (x0, y0, x1, y1) in pieces {
                let piece = CreateRectRgn(x0, y0, x1, y1);
                CombineRgn(Some(whole), Some(whole), Some(piece), RGN_OR);
                let _ = DeleteObject(piece.into());
            }
            whole
        }
    });
    // SAFETY: a window this thread owns; Windows takes the region (none: the whole window).
    if unsafe { SetWindowRgn(hwnd, region, true) } == 0 {
        eprintln!("lull: Classic window's shape: {}", windows::core::Error::from_thread());
    }
}

/// The modifier keys held, from this thread's keyboard state.
fn mods() -> Mods {
    // SAFETY: reads this thread's keyboard state.
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    Mods {
        control: held(VK_CONTROL.0),
        shift: held(VK_SHIFT.0),
        alt: false,
    }
}

/// A virtual key as the stack names it.
fn key(vk: u16) -> Option<Key> {
    Some(match vk {
        k if (u16::from(b'A')..=u16::from(b'Z')).contains(&k) => Key::Letter(k as u8),
        k if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&k) => Key::Pad((k - VK_NUMPAD0.0) as u8),
        k if k == VK_LEFT.0 => Key::Left,
        k if k == VK_RIGHT.0 => Key::Right,
        k if k == VK_UP.0 => Key::Up,
        k if k == VK_DOWN.0 => Key::Down,
        k if k == VK_DELETE.0 => Key::Delete,
        _ => return None,
    })
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
        scale_rows(canvas, (columns, rows), (x0, x1), (from, to), band.pixels(), stride);
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
                let (ascent, descent) = (f32::from(metrics.ascent), f32::from(metrics.descent));
                baseline(line, ascent / em * size, descent / em * size)
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
    use super::super::ui::edges;
    use super::*;

    /// The side-by-side with Webamp: ui.rs's scenes, with this window's own text and copy.
    #[test]
    #[ignore = "renders a folder of skins: LULL_SKINS_IN, LULL_RENDERS_OUT"]
    fn render_skins() -> Result<(), String> {
        super::super::ui::scenes::render_skins(|skin, canvas| {
            let (width, height) = (canvas.width, canvas.height);
            let (columns, rows) = (edges(width, 1.0), edges(height, 1.0));
            let size = (width as i32, height as i32);
            let (mut band, mut target) = (Surface::new(size)?, Surface::new(size)?);
            let text = Text::new(&skin.playlist.font, 1.0)?;
            paint(
                canvas,
                (&columns, &rows),
                &mut band,
                &text,
                target.dc,
                (0, 0, width, height),
            );
            // SAFETY: the copy into `target` is done before its pixels are read.
            let _ = unsafe { GdiFlush() };
            Ok(target.pixels().to_vec())
        })
    }
}
