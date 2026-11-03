//! The Classic window on Windows: a plain Win32 window, no webview. Like Winamp 2 it's a borderless
//! popup the skin cuts to shape, here a layered window whose pixels outside `region.txt` are
//! transparent (clicks there reach the window behind). The skin is drawn at 1x (draw.rs), only
//! what changed since the last frame is scaled up, nearest neighbour so pixels stay sharp at any
//! size, and handed to the compositor (UpdateLayeredWindowIndirect with a dirty rectangle).
//!
//! It lives on the main thread, whose message loop (Tauri's) delivers its messages. Its state sits
//! in a thread-local; Windows calls back into the window procedure while it runs (capture changes,
//! the move loop), so the state is borrowed only to decide what to do, and the calls that may call
//! back run after the borrow ends.
#![allow(unsafe_code)]

use super::draw::{Canvas, Control, View};
use super::skin::{HEIGHT, Skin, WIDTH};
use super::sound::Player;
use super::vis::{Analyser, FFT_SAMPLES};
use std::cell::RefCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_DOWN, VK_LEFT, VK_RIGHT, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTCAPTION, IDC_ARROW, KillTimer, LoadCursorW, LoadIconW,
    RegisterClassExW, SW_MINIMIZE, SW_SHOWNORMAL, SendMessageW, SetForegroundWindow, SetTimer, ShowWindow, ULW_ALPHA,
    UPDATELAYEREDWINDOWINFO, UpdateLayeredWindowIndirect, WA_INACTIVE, WM_ACTIVATE, WM_CLOSE, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCLBUTTONDOWN, WM_TIMER, WNDCLASSEXW,
    WS_EX_APPWINDOW, WS_EX_LAYERED, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
};
use windows::core::{PCWSTR, w};

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
/// Tauri's build puts the app's icon in the executable under this id (IDI_APPLICATION's).
const APP_ICON: u16 = 32512;

thread_local! {
    static OPEN: RefCell<Option<Classic>> = const { RefCell::new(None) };
}

/// What the window plays and shows; built off the main thread, opened on it.
pub struct Setup {
    pub skin: Skin,
    pub player: Player,
    /// The marquee's line for each song.
    pub titles: Vec<String>,
    /// Physical pixels per skin pixel.
    pub scale: f64,
    /// Top left, physical pixels.
    pub at: (i32, i32),
    pub shuffle: bool,
    pub repeat: bool,
}

/// What happens once the listener leaves Classic: the player as it is then.
pub type OnClose = Box<dyn FnOnce(&Player)>;

struct Classic {
    hwnd: HWND,
    skin: Skin,
    player: Player,
    titles: Vec<String>,
    shuffle: bool,
    repeat: bool,
    on_close: Option<OnClose>,
    surface: Surface,
    canvas: Canvas,
    /// The frame on screen, at 1x: what the next one is compared with.
    shown: Vec<u32>,
    /// For each skin column and row, the first physical one it covers (the last entry is the size).
    columns: Vec<i32>,
    rows: Vec<i32>,
    analyser: Analyser,
    focused: bool,
    pressed: Option<Control>,
    /// The pressed button is under the pointer: it looks pressed, and acts on release.
    over: bool,
    seeking: Option<f64>,
    remaining: bool,
    marquee: Marquee,
    started: Instant,
    tick: Duration,
}

/// What the window procedure does once the state is let go.
enum Effect {
    Default,
    Handled,
    /// The listener pressed on the window's own body: Windows moves it.
    Move,
    Capture,
    Release,
    Minimize,
    Close,
}

pub fn is_open() -> bool {
    OPEN.with_borrow(Option::is_some)
}

/// Opens the window. Main thread only.
pub fn open(setup: Setup, on_close: OnClose) -> Result<(), String> {
    if is_open() {
        return Err("Classic mode is open already".into());
    }
    let size = (
        (f64::from(WIDTH) * setup.scale).ceil() as i32,
        (f64::from(HEIGHT) * setup.scale).ceil() as i32,
    );
    let hwnd = create(setup.at, size)?;
    let surface = match Surface::new(size) {
        Ok(surface) => surface,
        Err(e) => {
            // SAFETY: the window was just created on this thread and nothing else holds it.
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(e);
        }
    };
    let edges = |count: u32| -> Vec<i32> {
        (0..=count)
            .map(|i| (f64::from(i) * setup.scale).ceil() as i32)
            .collect()
    };
    let mut classic = Classic {
        hwnd,
        skin: setup.skin,
        player: setup.player,
        titles: setup.titles,
        shuffle: setup.shuffle,
        repeat: setup.repeat,
        on_close: Some(on_close),
        surface,
        canvas: Canvas::new(),
        shown: Vec::new(),
        columns: edges(WIDTH),
        rows: edges(HEIGHT),
        analyser: Analyser::default(),
        focused: true,
        pressed: None,
        over: false,
        seeking: None,
        remaining: false,
        marquee: Marquee::default(),
        started: Instant::now(),
        tick: ANIMATE,
    };
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

/// Leaves Classic as its close button does. Main thread only.
pub fn close() {
    let Some(mut classic) = OPEN.with_borrow_mut(Option::take) else {
        return;
    };
    // SAFETY: a window this thread owns; its state is out of the thread-local already, so the
    // messages destroying it sends find nothing to borrow.
    unsafe {
        let _ = KillTimer(Some(classic.hwnd), TIMER);
        let _ = DestroyWindow(classic.hwnd);
    }
    if let Some(on_close) = classic.on_close.take() {
        on_close(&classic.player);
    }
}

fn create(at: (i32, i32), size: (i32, i32)) -> Result<HWND, String> {
    // SAFETY: plain Win32 calls with static strings; the class is registered once per process
    // (a second registration fails harmlessly and the first stands).
    unsafe {
        let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
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
        }
    }
}

impl Classic {
    fn handle(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Effect {
        let point = || {
            let x = i32::from((lparam.0 & 0xffff) as u16 as i16);
            let y = i32::from(((lparam.0 >> 16) & 0xffff) as u16 as i16);
            (skin_at(x, &self.columns), skin_at(y, &self.rows))
        };
        let effect = match message {
            WM_TIMER => {
                self.render();
                return Effect::Handled;
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point();
                self.press(x, y)
            }
            WM_MOUSEMOVE if self.pressed.is_some() => {
                let (x, y) = point();
                self.drag(x, y);
                Effect::Handled
            }
            WM_LBUTTONUP if self.pressed.is_some() => {
                let (x, y) = point();
                self.drag(x, y);
                self.release()
            }
            WM_MOUSEWHEEL => {
                let notches = f64::from((wparam.0 >> 16) as u16 as i16) / 120.0;
                self.player.set_volume(self.player.volume() + 0.05 * notches);
                Effect::Handled
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

    fn press(&mut self, x: u32, y: u32) -> Effect {
        let Some(control) = Control::at(x, y) else {
            return Effect::Move;
        };
        if control == Control::Time {
            self.remaining = !self.remaining;
            return Effect::Handled;
        }
        self.pressed = Some(control);
        self.over = true;
        self.drag(x, y);
        Effect::Capture
    }

    fn drag(&mut self, x: u32, y: u32) {
        let fraction = |area: super::skin::Area, thumb: u32| {
            let travel = f64::from(area.width - thumb);
            ((f64::from(x) - f64::from(area.x) - f64::from(thumb) / 2.0) / travel).clamp(0.0, 1.0)
        };
        match self.pressed {
            Some(Control::Volume) => self.player.set_volume(fraction(Control::Volume.area(), 14)),
            Some(Control::Position) => self.seeking = Some(fraction(Control::Position.area(), 29)),
            Some(control) => self.over = control.area().contains(x, y),
            None => {}
        }
    }

    fn release(&mut self) -> Effect {
        let (pressed, over) = (self.pressed.take(), self.over);
        let now = self.player.now();
        match pressed {
            Some(Control::Position) => {
                if let (Some(fraction), Some(length)) = (self.seeking.take(), now.duration) {
                    self.player.seek(fraction * length);
                }
            }
            Some(control) if over => return self.act(control),
            _ => {}
        }
        Effect::Release
    }

    /// A button, clicked: Winamp's own meaning for each.
    fn act(&mut self, control: Control) -> Effect {
        let now = self.player.now();
        match control {
            Control::Previous => self.player.jump(now.index.saturating_sub(1)),
            // Play on a playing song starts it again.
            Control::Play if now.playing => self.player.seek(0.0),
            Control::Play => self.player.play(),
            Control::Pause if now.playing => self.player.pause(),
            Control::Pause if !now.stopped => self.player.play(),
            Control::Stop => self.player.stop(),
            Control::Next if now.index + 1 < self.titles.len() => self.player.jump(now.index + 1),
            Control::Next if self.repeat => self.player.jump(0),
            Control::Minimize => return Effect::Minimize,
            Control::Close => return Effect::Close,
            _ => {}
        }
        Effect::Release
    }

    /// Winamp's keys: Z X C V B for the transport, arrows for a few seconds and the volume.
    fn key(&mut self, key: u16) -> Effect {
        let now = self.player.now();
        match key {
            k if k == u16::from(b'Z') => self.act(Control::Previous),
            k if k == u16::from(b'X') => self.act(Control::Play),
            k if k == u16::from(b'C') => self.act(Control::Pause),
            k if k == u16::from(b'V') => self.act(Control::Stop),
            k if k == u16::from(b'B') => self.act(Control::Next),
            k if k == VK_LEFT.0 => {
                self.player.seek((now.position - 5.0).max(0.0));
                Effect::Handled
            }
            k if k == VK_RIGHT.0 => {
                self.player.seek(now.position + 5.0);
                Effect::Handled
            }
            k if k == VK_UP.0 => {
                self.player.set_volume(self.player.volume() + 0.02);
                Effect::Handled
            }
            k if k == VK_DOWN.0 => {
                self.player.set_volume(self.player.volume() - 0.02);
                Effect::Handled
            }
            _ => Effect::Default,
        }
    }

    /// Draws a frame at 1x and puts on screen what changed. The timer then runs at the pace the
    /// window needs: fast while something moves, slow while only the title or a blink does.
    fn render(&mut self) {
        let now = self.player.now();
        let samples = if now.playing {
            self.player.tap(FFT_SAMPLES)
        } else {
            Vec::new()
        };
        let clock = Instant::now();
        let bars = self.analyser.step(&samples, clock);
        let line = match (&now.error, self.seeking, self.titles.get(now.index)) {
            (Some(error), _, _) => error.clone(),
            (None, Some(fraction), _) => seek_line(fraction, now.duration.unwrap_or(0.0)),
            (None, None, Some(title)) => title.clone(),
            (None, None, None) => String::new(),
        };
        let marquee = self.marquee.show(&line, clock);
        let blink_off = (clock - self.started).as_millis() / 500 % 2 == 1;
        let view = View {
            now: &now,
            marquee: &marquee,
            volume: self.player.volume(),
            focused: self.focused,
            pressed: self.pressed.filter(|_| self.over),
            seeking: self.seeking,
            remaining: self.remaining,
            blink_off,
            bars: &bars,
            shuffle: self.shuffle,
            repeat: self.repeat,
        };
        self.canvas.draw(&self.skin, &view);
        if let Some(dirty) = changed(&self.canvas.pixels, &self.shown) {
            self.present(dirty);
            self.shown.clone_from(&self.canvas.pixels);
        }
        let tick = if now.playing || !self.analyser.settled() || self.seeking.is_some() {
            ANIMATE
        } else {
            IDLE
        };
        if tick != self.tick {
            self.tick = tick;
            // SAFETY: a window this thread owns; replaces the running timer.
            unsafe { SetTimer(Some(self.hwnd), TIMER, tick.as_millis() as u32, None) };
        }
    }

    /// Scales the changed skin pixels into the window's bitmap and hands that part to Windows.
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
        let dirty = windows::Win32::Foundation::RECT {
            left,
            top,
            right,
            bottom,
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
}

/// The skin pixel a physical coordinate falls in: `edges` holds, for each skin pixel, the first
/// physical one it covers.
fn skin_at(physical: i32, edges: &[i32]) -> u32 {
    edges.partition_point(|&e| e <= physical).saturating_sub(1) as u32
}

/// The part of the frame that differs from the one on screen, as skin pixels `[x0, x1) x [y0, y1)`;
/// all of it the first time.
fn changed(frame: &[u32], shown: &[u32]) -> Option<(u32, u32, u32, u32)> {
    if shown.len() != frame.len() {
        return Some((0, 0, WIDTH, HEIGHT));
    }
    let (mut x0, mut y0, mut x1, mut y1) = (WIDTH, HEIGHT, 0, 0);
    for (i, (a, b)) in frame.iter().zip(shown).enumerate() {
        if a != b {
            let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (x1 > x0).then_some((x0, y0, x1, y1))
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

/// The window's bitmap: physical size, 32-bit premultiplied BGRA, top row first, in a memory DC
/// for UpdateLayeredWindowIndirect.
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
        assert_eq!(changed(&frame, &shown), None);
        frame[(50 * WIDTH + 30) as usize] = 1;
        frame[(52 * WIDTH + 40) as usize] = 1;
        assert_eq!(changed(&frame, &shown), Some((30, 50, 41, 53)));
        assert_eq!(changed(&frame, &[]), Some((0, 0, WIDTH, HEIGHT)));
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
}
