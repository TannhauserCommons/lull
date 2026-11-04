//! The Classic windows on Linux: GTK windows, no webview, one a panel of ui.rs's stack, on X11
//! and on Wayland alike. Like Winamp 2's windows they have no frame and the skin cuts them to
//! shape: their input shape always (clicks outside the skin's shapes reach the window behind), and
//! with no compositor to see through them, their visible shape too. GTK asks for what a window
//! shows when it shows it; only that part is scaled up, nearest neighbour so pixels stay sharp at
//! any size, a band of rows at a time into one small surface at the screen's own resolution, with
//! the playlist's names drawn over it (Pango, through GTK, with the system's fallback fonts), and
//! painted in. GTK's own scale stands in for Windows' DPI: the stack works in physical pixels,
//! GTK's events and sizes are in its logical ones. On X11 the stack places the windows, and they
//! dock as Winamp's; Wayland lets no app place its windows, so there each is moved by the
//! compositor, alone.
//!
//! They live on the main thread, GTK's, whose main loop (Tauri's) delivers their signals. The state
//! sits in a thread-local, borrowed only to decide what to do; moves and menus run after the borrow
//! ends.
#![allow(unsafe_code)]

use super::canvas::{Canvas, TextRun};
use super::dock::Rect;
use super::playlist::{TEXT_SIZE, TEXT_SPACING, TIME_PADDING, baseline};
use super::skin::{Skin, layout};
use super::ui::{Classic, Effect, Frame, Item, Key, Menu, Mods, PaneFrame, Panel, Shape, scale_rows};
use gtk::gdk::keys::constants as keys;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib, pango};
use std::cell::RefCell;

/// Rows of a window scaled and painted at a time: the band's height.
const BAND: i32 = 64;

thread_local! {
    static OPEN: RefCell<Option<Windows>> = const { RefCell::new(None) };
}

/// The stack and the GTK windows showing it.
struct Windows {
    classic: Classic,
    /// The main window's, the equaliser's and the playlist's, which are transient for the main
    /// one and off the taskbar.
    panes: [Pane; 3],
    /// GTK's scale: physical pixels to one of its own.
    scale: i32,
    text: Text,
    /// The font and scale `text` was made for.
    text_style: (String, f64),
    timer: Option<glib::SourceId>,
    /// The menu showing, kept until the next.
    menu: Option<gtk::Menu>,
}

/// One GTK window, and what it shows now.
struct Pane {
    window: gtk::Window,
    shown: bool,
    at: (i32, i32),
}

/// Opens the windows. Main thread only.
pub fn open(mut setup: Setup, on_close: OnClose) -> Result<(), String> {
    if OPEN.with_borrow(Option::is_some) {
        return Err("Classic mode is open already".into());
    }
    let windows: [gtk::Window; 3] = Panel::ALL.map(|_| create());
    let main = windows[0].clone();
    for window in &windows[1..] {
        window.set_transient_for(Some(&main));
        window.set_skip_taskbar_hint(true);
    }
    // GTK's scale is the screen's; the stack's size is on top of it.
    let scale = main.scale_factor().max(1);
    setup.dpi = f64::from(scale);
    let mut classic = Classic::new(setup, on_close);
    match super::mpris::Overlay::new(classic.session.app().clone()) {
        Ok(overlay) => classic.session.attach_overlay(overlay),
        Err(e) => eprintln!("lull: no media keys for Classic: {e}"),
    }
    classic.places_windows = !gdk::Display::default().is_some_and(|d| d.type_().name().contains("Wayland"));
    classic.set_areas(work_areas(scale));
    let (font, size) = classic.text_style();
    let text_style = (font.to_owned(), size);
    let text = Text::new(&main, font, size);
    for (window, panel) in windows.iter().zip(Panel::ALL) {
        connect(window, panel);
    }
    if let Some(screen) = WidgetExt::screen(&main) {
        screen.connect_monitors_changed(move |_| {
            with(|w| {
                w.classic.set_areas(work_areas(scale));
                Effect::Handled
            });
        });
    }
    let panes = windows.map(|window| Pane {
        window,
        shown: false,
        at: (i32::MIN, i32::MIN),
    });
    let mut open = Windows {
        classic,
        panes,
        scale,
        text,
        text_style,
        timer: None,
        menu: None,
    };
    open.render();
    OPEN.with_borrow_mut(|o| *o = Some(open));
    main.present();
    Ok(())
}

pub use super::ui::{OnClose, Setup};

/// A panel's window: undecorated, see-through where the skin isn't, taking the pointer and keys.
fn create() -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("lull classic");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_app_paintable(true);
    if let Some(visual) = WidgetExt::screen(&window).and_then(|screen| screen.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    window.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK
            | gdk::EventMask::KEY_PRESS_MASK,
    );
    window
}

/// The desktop's monitors' work areas (the screen less its panels), physical pixels.
fn work_areas(scale: i32) -> Vec<Rect> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    (0..display.n_monitors())
        .filter_map(|i| display.monitor(i))
        .map(|monitor| {
            let area = monitor.workarea();
            Rect::new(
                area.x() * scale,
                area.y() * scale,
                area.width() * scale,
                area.height() * scale,
            )
        })
        .collect()
}

/// Brings the windows forward (the tray, a second launch). Main thread only.
pub fn show() {
    if let Some(window) = OPEN.with_borrow(|open| open.as_ref().map(|w| w.panes[0].window.clone())) {
        window.present();
    }
}

/// The tray's and the taskbar's buttons, the system's media keys. Main thread only.
pub fn act(action: &str) {
    let acted = OPEN.with_borrow_mut(|open| {
        let windows = open.as_mut()?;
        windows.classic.act(action);
        windows.render();
        Some(())
    });
    if acted.is_none() {
        eprintln!("lull: Classic is closed: no {action}");
    }
}

/// Another skin, fetched for the Options menu; or why it couldn't be. Main thread only.
pub fn set_skin(skin: Result<(Skin, Option<String>), String>) {
    OPEN.with_borrow_mut(|open| {
        if let Some(windows) = open.as_mut() {
            windows.classic.set_skin(skin);
            windows.render();
        }
    });
}

/// Leaves Classic as its close button does. Main thread only.
pub fn close() {
    let Some(windows) = OPEN.with_borrow_mut(Option::take) else {
        return;
    };
    if let Some(timer) = windows.timer {
        timer.remove();
    }
    for pane in windows.panes.iter().rev() {
        // SAFETY: the windows' state is out of the thread-local already, so the signals destroying
        // them find nothing to borrow; nothing else holds them to use them after.
        unsafe { pane.window.destroy() };
    }
    let mut classic = windows.classic;
    let (size, layout) = (classic.size, classic.layout());
    if let Some(on_close) = classic.on_close.take() {
        on_close(classic.session, size, layout);
    }
}

/// A window's signals: each borrows the state to decide, then does what may call back.
fn connect(window: &gtk::Window, panel: Panel) {
    window.connect_draw(move |_, cr| {
        OPEN.with(|open| {
            if let Ok(open) = open.try_borrow()
                && let Some(w) = open.as_ref()
            {
                w.draw(panel, cr);
            }
        });
        glib::Propagation::Stop
    });
    window.connect_button_press_event(move |window, event| {
        let double = event.event_type() == gdk::EventType::DoubleButtonPress;
        let effect = match event.button() {
            1 => with(|w| {
                let (at, screen) = (w.physical(event.position()), w.physical(event.root()));
                w.classic.pointer_down(panel, at, screen, double, mods(event.state()))
            }),
            _ => None,
        };
        after(window, panel, effect, event);
        glib::Propagation::Stop
    });
    window.connect_button_release_event(move |window, event| {
        let effect = match event.button() {
            1 => with(|w| {
                let (at, screen) = (w.physical(event.position()), w.physical(event.root()));
                w.classic.pointer_up(panel, at, screen)
            }),
            3 => with(|w| w.classic.right_click(w.physical(event.position()))),
            _ => None,
        };
        after(window, panel, effect, event);
        glib::Propagation::Stop
    });
    window.connect_motion_notify_event(move |_, event| {
        with(|w| {
            let (at, screen) = (w.physical(event.position()), w.physical(event.root()));
            w.classic.pointer_move(panel, at, screen);
            Effect::Handled
        });
        glib::Propagation::Stop
    });
    window.connect_scroll_event(move |_, event| {
        let notches = match event.direction() {
            gdk::ScrollDirection::Up => 1.0,
            gdk::ScrollDirection::Down => -1.0,
            gdk::ScrollDirection::Smooth => -event.delta().1,
            _ => 0.0,
        };
        if notches != 0.0 {
            with(|w| w.classic.wheel(panel, notches));
        }
        glib::Propagation::Stop
    });
    window.connect_key_press_event(move |window, event| {
        let Some(key) = key(&event.keyval()) else {
            return glib::Propagation::Proceed;
        };
        match with(|w| w.classic.key(key, mods(event.state()))) {
            Some(Effect::Default) | None => glib::Propagation::Proceed,
            effect => {
                after(window, panel, effect, event);
                glib::Propagation::Stop
            }
        }
    });
    window.connect_delete_event(|_, _| {
        close();
        glib::Propagation::Stop
    });
}

/// The state, borrowed to handle a signal, and the frame drawn after; `None` when it's closed or
/// GTK called back while it's borrowed.
fn with(handle: impl FnOnce(&mut Windows) -> Effect) -> Option<Effect> {
    OPEN.with(|open| {
        let mut open = open.try_borrow_mut().ok()?;
        let windows = open.as_mut()?;
        let effect = handle(windows);
        windows.render();
        Some(effect)
    })
}

/// What the stack said to do about an event on `panel`'s window, done with the borrow ended.
fn after(window: &gtk::Window, panel: Panel, effect: Option<Effect>, event: &gdk::Event) {
    match effect {
        Some(Effect::Move) => {
            if let (Some(button), Some((x, y))) = (event.button(), event.root_coords()) {
                window.begin_move_drag(button as i32, x as i32, y as i32, event.time());
            }
            // A second press of a double click comes as a press on its own first.
            if event.event_type() == gdk::EventType::DoubleButtonPress {
                with(|w| {
                    w.classic
                        .title_double_click(panel, w.physical(event.coords().unwrap_or_default()))
                });
            }
        }
        Some(Effect::Minimize) => {
            if let Some(main) = OPEN.with_borrow(|open| open.as_ref().map(|w| w.panes[0].window.clone())) {
                main.iconify();
            }
        }
        Some(Effect::Close) => close(),
        Some(Effect::Menu(menu, at)) => pop_up(window, menu, at, event),
        // GTK holds the pointer for the window while a button is down: nothing to capture.
        Some(Effect::Capture | Effect::Release | Effect::Handled | Effect::Default) | None => {}
    }
}

/// Shows a menu at `at` (physical pixels of `window`); a choice comes back through `chose`.
fn pop_up(window: &gtk::Window, menu: Menu, at: (i32, i32), event: &gdk::Event) {
    let Some((items, scale)) = OPEN.with_borrow(|open| open.as_ref().map(|w| (w.classic.menu_items(menu), w.scale)))
    else {
        return;
    };
    let popup = build_menu(&items, menu);
    popup.set_attach_widget(Some(window));
    if let Some(gdk_window) = window.window() {
        let at = gdk::Rectangle::new(at.0 / scale, at.1 / scale, 1, 1);
        popup.popup_at_rect(
            &gdk_window,
            &at,
            gdk::Gravity::NorthWest,
            gdk::Gravity::NorthWest,
            Some(event),
        );
    }
    OPEN.with_borrow_mut(|open| {
        if let Some(w) = open.as_mut() {
            w.menu = Some(popup);
        }
    });
}

/// A menu's lines; a tab's text (a key) set off by spaces, as GTK has no column for it here.
fn build_menu(items: &[Item], menu: Menu) -> gtk::Menu {
    let popup = gtk::Menu::new();
    for item in items {
        let entry: gtk::MenuItem = match item {
            Item::Command(id, words, ticked) => {
                let words = words.replace('\t', "    ");
                let entry: gtk::MenuItem = if *ticked {
                    let check = gtk::CheckMenuItem::with_label(&words);
                    // Before its activation is wired: setting it activates it.
                    check.set_active(true);
                    check.upcast()
                } else {
                    gtk::MenuItem::with_label(&words)
                };
                let id = *id;
                entry.connect_activate(move |_| chose(menu, id));
                entry
            }
            Item::Separator => gtk::SeparatorMenuItem::new().upcast(),
            Item::Submenu(words, items) => {
                let entry = gtk::MenuItem::with_label(words);
                entry.set_submenu(Some(&build_menu(items, menu)));
                entry
            }
        };
        popup.append(&entry);
    }
    popup.show_all();
    popup
}

/// The listener's choice from a menu.
fn chose(menu: Menu, id: u32) {
    if let Some(Effect::Close) = with(|w| w.classic.chose(menu, id)) {
        close();
    }
}

impl Windows {
    /// A frame of the stack, and what it says the windows do about it.
    fn render(&mut self) {
        let Frame { panes, tick } = self.classic.render();
        let (font, scale) = self.classic.text_style();
        if (font, scale) != (self.text_style.0.as_str(), self.text_style.1) {
            self.text = Text::new(&self.panes[0].window, font, scale);
            self.text_style = (font.to_owned(), scale);
        }
        for (i, frame) in panes.into_iter().enumerate() {
            self.apply(i, frame);
        }
        if let Some(tick) = tick {
            if let Some(timer) = self.timer.take() {
                timer.remove();
            }
            self.timer = Some(glib::timeout_add_local(tick, || {
                with(|_| Effect::Handled);
                glib::ControlFlow::Continue
            }));
        }
    }

    /// A window's part of a frame: shown or hidden, its place, size, shape and what changed.
    fn apply(&mut self, i: usize, frame: PaneFrame) {
        let scale = self.scale;
        let logical = |physical: i32| (physical + scale - 1) / scale;
        let pane = &mut self.panes[i];
        let window = pane.window.clone();
        if !frame.shown {
            if pane.shown {
                pane.shown = false;
                window.hide();
            }
            return;
        }
        if let Some((width, height)) = frame.size {
            window.set_size_request(logical(width), logical(height));
            window.resize(logical(width), logical(height));
        }
        if frame.at != pane.at {
            pane.at = frame.at;
            window.move_(frame.at.0 / scale, frame.at.1 / scale);
        }
        if let Some(shape) = frame.shape {
            reshape(&window, scale, shape);
        }
        if let Some((x0, y0, x1, y1)) = frame.dirty {
            let p = self.classic.pane(Panel::ALL[i]);
            let (left, top) = (p.columns[x0 as usize], p.rows[y0 as usize]);
            let (right, bottom) = (p.columns[x1 as usize], p.rows[y1 as usize]);
            window.queue_draw_area(
                left / scale,
                top / scale,
                logical(right) - left / scale,
                logical(bottom) - top / scale,
            );
        }
        let pane = &mut self.panes[i];
        if !pane.shown {
            pane.shown = true;
            window.show_all();
        }
    }

    /// What GTK asks to see of a window: the skin pixels under its clip, scaled into the band and
    /// painted in.
    fn draw(&self, panel: Panel, cr: &cairo::Context) {
        let Ok((x0, y0, x1, y1)) = cr.clip_extents() else {
            return;
        };
        let scale = f64::from(self.scale);
        let physical = |v: f64| (v * scale) as i32;
        let Some(rect) = self.classic.covering(
            panel,
            (physical(x0), physical(y0), physical(x1.ceil()), physical(y1.ceil())),
        ) else {
            return;
        };
        if let Err(e) = self.paint(panel, cr, rect) {
            eprintln!("lull: Classic window didn't update: {e}");
        }
    }

    /// Scales skin pixels `[x0, x1) x [y0, y1)` of a window's canvas up a band at a time, draws the
    /// playlist's text over each, and paints it into `cr` (GTK's, in its own pixels).
    fn paint(&self, panel: Panel, cr: &cairo::Context, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Result<(), String> {
        let failed = |e: cairo::Error| e.to_string();
        let pane = self.classic.pane(panel);
        let (columns, rows) = (&pane.columns, &pane.rows);
        let (left, right) = (columns[x0 as usize], columns[x1 as usize]);
        let (top, bottom) = (rows[y0 as usize], rows[y1 as usize]);
        let column = self.text.column(&pane.canvas.texts);
        let scale = f64::from(self.scale);
        cr.set_operator(cairo::Operator::Source);
        let mut from = top;
        while from < bottom {
            let to = (from + BAND).min(bottom);
            let mut band =
                cairo::ImageSurface::create(cairo::Format::ARgb32, right - left, to - from).map_err(failed)?;
            let stride = band.stride() as usize / 4;
            {
                let mut data = band.data().map_err(|e| e.to_string())?;
                let (_, pixels, _) = as_pixels(&mut data);
                scale_rows(&pane.canvas, (columns, rows), (x0, x1), (from, to), pixels, stride);
            }
            band.mark_dirty();
            self.text
                .draw_runs(&band, &pane.canvas, (columns, rows), (left, from), column)
                .map_err(failed)?;
            band.set_device_scale(scale, scale);
            cr.set_source_surface(&band, f64::from(left) / scale, f64::from(from) / scale)
                .map_err(failed)?;
            cr.rectangle(
                f64::from(left) / scale,
                f64::from(from) / scale,
                f64::from(right - left) / scale,
                f64::from(to - from) / scale,
            );
            cr.fill().map_err(failed)?;
            from = to;
        }
        Ok(())
    }

    /// A point of GTK's (its own pixels) as a physical one.
    fn physical(&self, (x, y): (f64, f64)) -> (i32, i32) {
        let scale = f64::from(self.scale);
        ((x * scale) as i32, (y * scale) as i32)
    }
}

/// Cuts a window to these rectangles (physical pixels); none: the whole window. What it takes
/// input in, always; what shows, too, with no compositor to see through it.
fn reshape(window: &gtk::Window, scale: i32, pieces: Shape) {
    let logical = |physical: i32| (physical + scale - 1) / scale;
    let region = pieces.map(|pieces| {
        let rects: Vec<cairo::RectangleInt> = pieces
            .into_iter()
            .map(|(x0, y0, x1, y1)| {
                let (x0, y0) = (x0 / scale, y0 / scale);
                cairo::RectangleInt::new(x0, y0, logical(x1) - x0, logical(y1) - y0)
            })
            .collect();
        cairo::Region::create_rectangles(&rects)
    });
    window.input_shape_combine_region(region.as_ref());
    let composited = WidgetExt::screen(window).is_some_and(|screen| screen.is_composited());
    if !composited {
        window.shape_combine_region(region.as_ref());
    }
}

/// A surface's bytes as the 32-bit pixels they are: Cairo's ARGB32, premultiplied and native-endian,
/// as the canvas's are. The ends are whatever doesn't line up (nothing, for Cairo's aligned rows).
fn as_pixels(bytes: &mut [u8]) -> (&mut [u8], &mut [u32], &mut [u8]) {
    // SAFETY: any four bytes are a valid u32; align_to_mut gives back only what lines up.
    unsafe { bytes.align_to_mut::<u32>() }
}

/// The modifier keys held, from an event's state.
fn mods(state: gdk::ModifierType) -> Mods {
    Mods {
        control: state.contains(gdk::ModifierType::CONTROL_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        alt: state.contains(gdk::ModifierType::MOD1_MASK),
    }
}

/// A key as the stack names it. With Num Lock off the keypad's keys are the arrows and Delete, as
/// they are to Webamp in a browser.
fn key(key: &gdk::keys::Key) -> Option<Key> {
    let pads = [
        keys::KP_0,
        keys::KP_1,
        keys::KP_2,
        keys::KP_3,
        keys::KP_4,
        keys::KP_5,
        keys::KP_6,
        keys::KP_7,
        keys::KP_8,
        keys::KP_9,
    ];
    if let Some(pad) = pads.iter().position(|k| k == key) {
        return Some(Key::Pad(pad as u8));
    }
    Some(match key {
        k if *k == keys::Left || *k == keys::KP_Left => Key::Left,
        k if *k == keys::Right || *k == keys::KP_Right => Key::Right,
        k if *k == keys::Up || *k == keys::KP_Up => Key::Up,
        k if *k == keys::Down || *k == keys::KP_Down => Key::Down,
        k if *k == keys::Delete || *k == keys::KP_Delete => Key::Delete,
        k => match k.to_unicode() {
            Some(c) if c.is_ascii_alphabetic() => Key::Letter(c.to_ascii_uppercase() as u8),
            _ => return None,
        },
    })
}

/// The playlist's text as Webamp's is drawn by a browser on Linux: Pango, in the skin's font
/// (`pledit.txt`) or Arial or the system's sans (Webamp's `font-family: <font>, Arial,
/// sans-serif`), 9 px with 0.5 px between letters, each 13 px row laid out as CSS lays out a line
/// of that height; titles too long end in an ellipsis, and characters the font hasn't come from the
/// system's fallback fonts. Drawn at the screen's size.
struct Text {
    context: pango::Context,
    style: gtk::StyleContext,
    font: pango::FontDescription,
    /// From a row's top.
    baseline: f64,
    /// Physical pixels a skin pixel is.
    scale: f64,
}

impl Text {
    fn new(window: &gtk::Window, face: &str, scale: f64) -> Self {
        let context = window.pango_context();
        let mut font = pango::FontDescription::new();
        font.set_family(&format!("{face},Arial,sans-serif"));
        font.set_absolute_size(f64::from(TEXT_SIZE) * scale * f64::from(pango::SCALE));
        let line = layout::PLAYLIST_ROW_HEIGHT as f32 * scale as f32;
        let metrics = context.metrics(Some(&font), None);
        let unit = pango::SCALE as f32;
        let baseline = baseline(line, metrics.ascent() as f32 / unit, metrics.descent() as f32 / unit);
        Self {
            context,
            style: window.style_context(),
            font,
            baseline: f64::from(baseline),
            scale,
        }
    }

    /// A line of the text, `width` physical pixels wide at most (ending in an ellipsis), in `color`.
    fn layout(&self, text: &str, width: Option<f64>, color: [u8; 3]) -> pango::Layout {
        let layout = pango::Layout::new(&self.context);
        layout.set_font_description(Some(&self.font));
        layout.set_text(text);
        let attributes = pango::AttrList::new();
        let spacing = f64::from(TEXT_SPACING) * self.scale * f64::from(pango::SCALE);
        attributes.insert(pango::AttrInt::new_letter_spacing(spacing as i32));
        let [r, g, b] = color.map(|c| u16::from(c) * 257);
        attributes.insert(pango::AttrColor::new_foreground(r, g, b));
        layout.set_attributes(Some(&attributes));
        if let Some(width) = width {
            layout.set_width((width * f64::from(pango::SCALE)) as i32);
            layout.set_ellipsize(pango::EllipsizeMode::End);
        }
        layout
    }

    /// The songs' times' column, as Webamp's flex lays it out: as wide as the widest in sight and
    /// its padding.
    fn column(&self, runs: &[TextRun]) -> f64 {
        let widest = runs
            .iter()
            .filter(|run| run.right)
            .map(|run| f64::from(self.layout(&run.text, None, run.color).pixel_size().0))
            .fold(0.0, f64::max);
        if widest > 0.0 {
            widest + f64::from(TIME_PADDING) * self.scale
        } else {
            0.0
        }
    }

    /// The canvas's runs that fall in `band` (whose top left is physical `origin`), drawn into it.
    fn draw_runs(
        &self,
        band: &cairo::ImageSurface,
        canvas: &Canvas,
        (columns, rows): (&[i32], &[i32]),
        (left, top): (i32, i32),
        column: f64,
    ) -> Result<(), cairo::Error> {
        let (width, height) = (band.width(), band.height());
        let cr = cairo::Context::new(band)?;
        cr.set_operator(cairo::Operator::Over);
        for run in &canvas.texts {
            let area = run.area;
            let (x0, x1) = (
                columns[area.x as usize] - left,
                columns[(area.x + area.width) as usize] - left,
            );
            let (y0, y1) = (rows[area.y as usize] - top, rows[(area.y + area.height) as usize] - top);
            if x0 >= width || x1 <= 0 || y0 >= height || y1 <= 0 {
                continue;
            }
            let (x0, x1, y0) = (f64::from(x0), f64::from(x1), f64::from(y0));
            let padding = f64::from(TIME_PADDING) * self.scale;
            let (layout, x) = if run.right {
                let layout = self.layout(&run.text, None, run.color);
                let x = x1 - padding - f64::from(layout.pixel_size().0);
                (layout, x)
            } else {
                (self.layout(&run.text, Some((x1 - column - x0).max(0.0)), run.color), x0)
            };
            cr.save()?;
            cr.rectangle(x0, y0, x1 - x0, f64::from(y1) - y0);
            cr.clip();
            let y = y0 + self.baseline - f64::from(layout.baseline()) / f64::from(pango::SCALE);
            gtk::render_layout(&self.style, &cr, x, y, &layout);
            cr.restore()?;
        }
        Ok(())
    }
}
