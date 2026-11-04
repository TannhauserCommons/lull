//! The main window drawn from the skin at 1x, the way Winamp 2 drew it: the background, then each
//! control's sprite at its place, the time in the skin's digits, the song in its bitmap font and
//! the analyser in its colours; or rolled up (shade), its title bar with the time, a small
//! analyser and the position. Also what the three windows share: their controls and the view they
//! draw from.

use super::canvas::Pen;
use super::session::Now;
use super::skin::{Area, Sprite, font, layout, sprites};
use super::vis::{BARS, Bar, ROWS};
use crate::desktop::classic::Equaliser;

/// Every control of the three windows, for hit testing and the pressed look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Options,
    Minimize,
    Shade,
    Close,
    Previous,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
    Shuffle,
    Repeat,
    Volume,
    Balance,
    Position,
    Time,
    /// The scrolling title: where a song waiting for the listener's choice asks for it.
    Marquee,
    EqWindow,
    PlaylistWindow,
    EqClose,
    EqShade,
    EqOn,
    EqAuto,
    EqPresets,
    EqPreamp,
    EqBand(usize),
    PlaylistClose,
    PlaylistShade,
    /// A song's row, by its place in the queue.
    PlaylistRow(usize),
    PlaylistScroll,
    /// ADD, REM, SEL, MISC, LIST along the bottom.
    PlaylistMenu(usize),
}

/// A control under the pointer: where it is in its window, and for a slider the size of its knob
/// along the way it slides and how far the knob travels (0 for a button). The travel is the one
/// the slider is drawn with, so the knob stays under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub control: Control,
    pub area: Area,
    pub knob: u32,
    pub travel: u32,
}

/// What the windows show beyond the skin.
/// Which of the three windows is the selected one, its title bar lit: as in Webamp, the last one
/// clicked, the main window to begin with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Focus {
    #[default]
    Main,
    Equaliser,
    Playlist,
}

pub struct View<'a> {
    pub now: &'a Now,
    pub marquee: &'a str,
    pub volume: f64,
    /// -1 (left) to 1 (right).
    pub balance: f64,
    pub focus: Focus,
    /// The button held down, drawn pressed while the pointer is on it.
    pub pressed: Option<Control>,
    /// The position the listener is dragging the seek bar to, as a fraction.
    pub seeking: Option<f64>,
    pub remaining: bool,
    /// Paused, the time blinks: this is its off half.
    pub blink_off: bool,
    pub bars: &'a [Bar; BARS],
    pub shuffle: bool,
    pub repeat: bool,
    pub eq_open: bool,
    pub playlist_open: bool,
    pub eq: &'a Equaliser,
    pub list: &'a super::playlist::List,
}

impl View<'_> {
    pub fn held(&self, control: Control) -> bool {
        self.pressed == Some(control)
    }
}

/// The main window's controls, unrolled and rolled up.
const MAIN: [(Control, Area); 19] = [
    (Control::Options, layout::OPTIONS_BUTTON),
    (Control::Minimize, layout::MINIMIZE_BUTTON),
    (Control::Shade, layout::SHADE_BUTTON),
    (Control::Close, layout::CLOSE_BUTTON),
    (Control::Previous, layout::PREVIOUS),
    (Control::Play, layout::PLAY),
    (Control::Pause, layout::PAUSE),
    (Control::Stop, layout::STOP),
    (Control::Next, layout::NEXT),
    (Control::Eject, layout::EJECT),
    (Control::Shuffle, layout::SHUFFLE),
    (Control::Repeat, layout::REPEAT),
    (Control::Volume, layout::VOLUME),
    (Control::Balance, layout::BALANCE),
    (Control::Position, layout::POSITION),
    (Control::Time, Area::new(layout::MINUS_EX.x, layout::MINUS_EX.y, 63, 13)),
    (Control::EqWindow, layout::EQ_BUTTON),
    (Control::PlaylistWindow, layout::PLAYLIST_BUTTON),
    (Control::Marquee, layout::MARQUEE),
];
const SHADED: [(Control, Area); 12] = [
    (Control::Options, layout::OPTIONS_BUTTON),
    (Control::Minimize, layout::MINIMIZE_BUTTON),
    (Control::Shade, layout::SHADE_BUTTON),
    (Control::Close, layout::CLOSE_BUTTON),
    (Control::Previous, layout::SHADE_PREVIOUS),
    (Control::Play, layout::SHADE_PLAY),
    (Control::Pause, layout::SHADE_PAUSE),
    (Control::Stop, layout::SHADE_STOP),
    (Control::Next, layout::SHADE_NEXT),
    (Control::Eject, layout::SHADE_EJECT),
    (Control::Position, layout::SHADE_POSITION),
    (Control::Time, layout::SHADE_TIME),
];

/// The main window's control under a point of it; `None` is the window itself, which drags.
pub fn main_at(x: u32, y: u32, shaded: bool) -> Option<Target> {
    let controls: &[(Control, Area)] = if shaded { &SHADED } else { &MAIN };
    let (control, area) = controls.iter().copied().find(|(_, area)| area.contains(x, y))?;
    let (knob, travel) = match (control, shaded) {
        (Control::Volume, _) => (14, layout::VOLUME_TRAVEL),
        (Control::Balance, _) => (14, layout::BALANCE_TRAVEL),
        (Control::Position, false) => (29, layout::POSITION_TRAVEL),
        (Control::Position, true) => (3, area.width - 3),
        _ => (0, 0),
    };
    Some(Target {
        control,
        area,
        knob,
        travel,
    })
}

pub fn main(pen: &mut Pen, view: &View) {
    pen.sprite(sprites::MAIN_BACKGROUND, 0, 0);
    title_bar(pen, view);
    pen.sprite(sprites::CLUTTER_BAR, layout::CLUTTER_BAR.x, layout::CLUTTER_BAR.y);
    status(pen, view.now);
    time(pen, view);
    // Stopped, Webamp has no analyser: the main window's own picture shows there.
    if !view.now.stopped {
        visualiser(pen, view.bars);
    }
    pen.text(view.marquee, layout::MARQUEE);
    rates(pen, view.now);
    sliders(pen, view);
    transport(pen, view);
    let mask = pen.skin.regions.main.clone();
    pen.mask(mask.as_ref(), super::skin::HEIGHT);
}

/// Rolled up: the shade bar, its buttons, the time in the skin's font, a small analyser and the
/// position. The transport is painted into the bar: its places are only for clicking.
pub fn main_shaded(pen: &mut Pen, view: &View) {
    let focused = view.focus == Focus::Main;
    let bar = if focused {
        sprites::SHADE_BAR_ACTIVE
    } else {
        sprites::SHADE_BAR_INACTIVE
    };
    pen.at(bar, layout::TITLE_BAR);
    for (control, pressed) in [
        (Control::Options, sprites::OPTIONS_BUTTON_PRESSED),
        (Control::Minimize, sprites::MINIMIZE_BUTTON_PRESSED),
        (Control::Shade, sprites::UNSHADE_BUTTON_PRESSED),
        (Control::Close, sprites::CLOSE_BUTTON_PRESSED),
    ] {
        if view.held(control) {
            pen.at(pressed, main_area(control));
        }
    }
    // At rest the bar's own picture of its buttons shows, as in Webamp, but for the unshade
    // button of the selected window.
    if focused && !view.held(Control::Shade) {
        pen.at(sprites::UNSHADE_BUTTON, main_area(Control::Shade));
    }
    let area = layout::SHADE_TIME;
    mini_time(pen, view, area.x, area.y);
    small_analyser(pen, view);
    pen.at(sprites::SHADE_POSITION_TRACK, layout::SHADE_POSITION);
    if let Some(percent) = position_percent(view) {
        let travel = layout::SHADE_POSITION.width - 3;
        let offset = (percent / 100.0 * f64::from(travel)).round() as u32;
        // Webamp's thumb looks to the left up to a third, to the right from two.
        let thumb = match percent {
            p if p <= 33.0 => sprites::SHADE_POSITION_THUMB_LEFT,
            p if p >= 66.0 => sprites::SHADE_POSITION_THUMB_RIGHT,
            _ => sprites::SHADE_POSITION_THUMB,
        };
        pen.sprite(thumb, layout::SHADE_POSITION.x + offset, layout::SHADE_POSITION.y);
    }
    let mask = pen.skin.regions.main_shade.clone();
    pen.mask(mask.as_ref(), layout::SHADE_HEIGHT);
}

fn main_area(control: Control) -> Area {
    MAIN.iter()
        .find(|(c, _)| *c == control)
        .map_or(Area::new(0, 0, 0, 0), |(_, area)| *area)
}

fn title_bar(pen: &mut Pen, view: &View) {
    let bar = if view.focus == Focus::Main {
        sprites::TITLE_BAR_ACTIVE
    } else {
        sprites::TITLE_BAR_INACTIVE
    };
    pen.at(bar, layout::TITLE_BAR);
    for (control, pressed) in [
        (Control::Options, sprites::OPTIONS_BUTTON_PRESSED),
        (Control::Minimize, sprites::MINIMIZE_BUTTON_PRESSED),
        (Control::Shade, sprites::SHADE_BUTTON_PRESSED),
        (Control::Close, sprites::CLOSE_BUTTON_PRESSED),
    ] {
        // Pressed only: at rest the title bar's own picture of its buttons shows, as in Webamp.
        if view.held(control) {
            pen.at(pressed, main_area(control));
        }
    }
}

fn status(pen: &mut Pen, now: &Now) {
    let status = if now.stopped {
        sprites::STATUS_STOPPED
    } else if now.playing {
        sprites::STATUS_PLAYING
    } else {
        sprites::STATUS_PAUSED
    };
    pen.at(status, layout::STATUS);
    // Webamp shows the work indicator while a song plays or loads, and nothing there otherwise.
    if now.loading {
        pen.at(sprites::WORK_INDICATOR_ON, layout::WORK_INDICATOR);
    } else if now.playing && !now.stopped {
        pen.at(sprites::WORK_INDICATOR_OFF, layout::WORK_INDICATOR);
    }
    let sounding = !now.stopped && !now.loading;
    let (stereo, mono) = match (sounding, now.stereo) {
        (false, _) => (sprites::STEREO_OFF, sprites::MONO_OFF),
        (true, true) => (sprites::STEREO_ON, sprites::MONO_OFF),
        (true, false) => (sprites::STEREO_OFF, sprites::MONO_ON),
    };
    pen.at(stereo, layout::STEREO);
    pen.at(mono, layout::MONO);
}

/// Elapsed, or remaining with a minus sign. Stopped, and in the off half of a pause's blink, the
/// time isn't there at all, as in Webamp: the main window's own background shows.
fn time(pen: &mut Pen, view: &View) {
    let extended = pen.skin.has_extended_digits();
    let now = view.now;
    if now.stopped || (!now.playing && view.blink_off) {
        return;
    }
    let length = now.duration.unwrap_or(0.0);
    let position = view.seeking.map_or(now.position, |f| f * length);
    let remaining = view.remaining && length > 0.0;
    let shown = if remaining { length - position } else { position };
    let seconds = shown.max(0.0) as u32;
    let minutes = (seconds / 60).min(99);
    let seconds = seconds % 60;
    let digits = [minutes / 10, minutes % 10, seconds / 10, seconds % 10];
    for (value, cell) in digits.into_iter().zip(layout::TIME_DIGITS) {
        pen.at(sprites::digit(value, extended), cell);
    }
    // The minus sign only counting down; otherwise nothing there, as in Webamp.
    match (remaining, extended) {
        (false, _) => {}
        (true, true) => pen.at(sprites::NUMS_EX_MINUS, layout::MINUS_EX),
        (true, false) => pen.at(sprites::NUMBERS_MINUS, layout::MINUS),
    }
}

/// The rolled-up window's small analyser: as the big one, not there at all while stopped (the
/// bar's own picture shows), as in Webamp.
fn small_analyser(pen: &mut Pen, view: &View) {
    if view.now.stopped {
        return;
    }
    let area = layout::SHADE_VISUALIZER;
    let palette = pen.skin.vis_colors;
    pen.fill(area.x, area.y, area.width, area.height, palette[0]);
    for (index, bar) in view.bars.iter().enumerate().take((area.width / 2) as usize) {
        let rows = u32::from(bar.height) * area.height / u32::from(ROWS);
        for row in 0..rows.min(area.height) {
            // The skin's gradient sampled across the five rows, bottom up.
            let shade = [17, 14, 11, 8, 4][row as usize];
            pen.fill(
                area.x + 2 * index as u32,
                area.y + area.height - 1 - row,
                1,
                1,
                palette[shade],
            );
        }
    }
}

/// Webamp's small clock, the rolled-up main window's and the playlist's: five cells of the skin's
/// font at its own spacing, blanks under them always, and over them the sign (counting down) and
/// the minutes and seconds of a song with a length, the lot gone in the off half of a pause's
/// blink. The colon is the bar's own picture.
pub fn mini_time(pen: &mut Pen, view: &View, x: u32, y: u32) {
    const CELLS: [u32; 5] = [1, 7, 12, 20, 25];
    let mut put = |c: char, cell: u32| {
        if let Some(glyph) = font::glyph(c) {
            pen.sprite(glyph, x + cell, y);
        }
    };
    for cell in CELLS {
        put(' ', cell);
    }
    let now = view.now;
    if now.stopped || (!now.playing && view.blink_off) {
        return;
    }
    let sign = if view.remaining { '-' } else { ' ' };
    let digits = match now.duration.filter(|length| *length > 0.0) {
        Some(length) => {
            let at = view.seeking.map_or(now.position, |f| f * length);
            let shown = if view.remaining { length - at } else { at };
            let seconds = shown.max(0.0) as u32;
            format!("{:02}{:02}", (seconds / 60).min(99), seconds % 60)
        }
        None => "    ".to_owned(),
    };
    for (cell, c) in CELLS.into_iter().zip(std::iter::once(sign).chain(digits.chars())) {
        put(c, cell);
    }
}

/// The analyser: the skin's background and dot grid (even columns, odd rows, as Webamp draws it),
/// nineteen bars in its sixteen row colours, and the peaks.
fn visualiser(pen: &mut Pen, bars: &[Bar; BARS]) {
    let area = layout::VISUALIZER;
    let palette = pen.skin.vis_colors;
    pen.fill(area.x, area.y, area.width, area.height, palette[0]);
    for y in (1..area.height).step_by(2) {
        for x in (0..area.width).step_by(2) {
            pen.fill(area.x + x, area.y + y, 1, 1, palette[1]);
        }
    }
    let row_at = |row: u8| u32::from(row) * area.height / u32::from(ROWS);
    let stride = 4;
    for (index, bar) in bars.iter().enumerate() {
        let x = area.x + stride * index as u32;
        for row in row_at(ROWS - bar.height)..area.height {
            pen.fill(x, area.y + row, stride - 1, 1, palette[2 + row as usize]);
        }
        if let Some(peak) = bar.peak {
            pen.fill(x, area.y + row_at(ROWS - peak), stride - 1, 1, palette[23]);
        }
    }
}

fn rates(pen: &mut Pen, now: &Now) {
    if now.stopped || now.loading {
        return;
    }
    if let Some(kbps) = now.kbps {
        pen.text(&format!("{:>3}", kbps.min(999)), layout::KBPS);
    }
    if let Some(khz) = now.khz {
        pen.text(&format!("{:>2}", khz.min(99)), layout::KHZ);
    }
}

/// A two-state button's look: off or on, resting or held.
fn lamp(on: bool, held: bool, looks: [Sprite; 4]) -> Sprite {
    looks[usize::from(on) * 2 + usize::from(held)]
}

fn sliders(pen: &mut Pen, view: &View) {
    let level = view.volume.clamp(0.0, 1.0);
    // Webamp's frame for a volume (`round(28 v) - 1`, the first at silence).
    let frame = ((level * 28.0).round() as u32).saturating_sub(1);
    pen.at(sprites::volume_frame(frame), layout::VOLUME);
    let thumb = if view.held(Control::Volume) {
        sprites::VOLUME_THUMB_PRESSED
    } else {
        sprites::VOLUME_THUMB
    };
    let x = layout::VOLUME.x + (level * f64::from(layout::VOLUME_TRAVEL)).round() as u32;
    pen.sprite(thumb, x, layout::VOLUME.y + 1);
    // Webamp's balance: 27 frames either way, the same for both sides; a skin without balance.bmp
    // shows its volume slider's pictures.
    let balance = view.balance.clamp(-1.0, 1.0);
    let frame = (27.0 * balance.abs()).floor() as u32;
    let held = view.held(Control::Balance);
    let (track, thumb) = match (pen.skin.has_balance(), held) {
        (true, false) => (sprites::balance_frame(frame), sprites::BALANCE_THUMB),
        (true, true) => (sprites::balance_frame(frame), sprites::BALANCE_THUMB_PRESSED),
        (false, false) => (sprites::volume_frame(frame), sprites::VOLUME_THUMB),
        (false, true) => (sprites::volume_frame(frame), sprites::VOLUME_THUMB_PRESSED),
    };
    let area = layout::BALANCE;
    pen.sprite_clipped(track, area.x, area.y, area);
    let x = area.x + ((balance + 1.0) / 2.0 * f64::from(layout::BALANCE_TRAVEL)).round() as u32;
    pen.sprite(thumb, x, area.y + 1);
    let eq = [
        sprites::EQ_OFF,
        sprites::EQ_OFF_PRESSED,
        sprites::EQ_ON,
        sprites::EQ_ON_PRESSED,
    ];
    pen.at(lamp(view.eq_open, view.held(Control::EqWindow), eq), layout::EQ_BUTTON);
    let playlist = [
        sprites::PLAYLIST_OFF,
        sprites::PLAYLIST_OFF_PRESSED,
        sprites::PLAYLIST_ON,
        sprites::PLAYLIST_ON_PRESSED,
    ];
    pen.at(
        lamp(view.playlist_open, view.held(Control::PlaylistWindow), playlist),
        layout::PLAYLIST_BUTTON,
    );
    pen.at(sprites::POSITION_TRACK, layout::POSITION);
    if let Some(percent) = position_percent(view) {
        let thumb = if view.seeking.is_some() {
            sprites::POSITION_THUMB_PRESSED
        } else {
            sprites::POSITION_THUMB
        };
        let x = layout::POSITION.x + (percent / 100.0 * f64::from(layout::POSITION_TRAVEL)).round() as u32;
        pen.sprite(thumb, x, layout::POSITION.y);
    }
}

/// Where the seek bar's thumb is, as Webamp's range input has it: whole percents of the song,
/// from its whole seconds (or where the listener drags it); none while stopped or of no length.
fn position_percent(view: &View) -> Option<f64> {
    let length = view.now.duration.filter(|l| *l > 0.0 && !view.now.stopped)?;
    let fraction = view.seeking.unwrap_or(view.now.position.floor() / length);
    Some((fraction.clamp(0.0, 1.0) * 100.0).round())
}

fn transport(pen: &mut Pen, view: &View) {
    for (control, normal, pressed) in [
        (Control::Previous, sprites::PREVIOUS, sprites::PREVIOUS_PRESSED),
        (Control::Play, sprites::PLAY, sprites::PLAY_PRESSED),
        (Control::Pause, sprites::PAUSE, sprites::PAUSE_PRESSED),
        (Control::Stop, sprites::STOP, sprites::STOP_PRESSED),
        (Control::Next, sprites::NEXT, sprites::NEXT_PRESSED),
        (Control::Eject, sprites::EJECT, sprites::EJECT_PRESSED),
    ] {
        pen.at(if view.held(control) { pressed } else { normal }, main_area(control));
    }
    let shuffle = [
        sprites::SHUFFLE_OFF,
        sprites::SHUFFLE_OFF_PRESSED,
        sprites::SHUFFLE_ON,
        sprites::SHUFFLE_ON_PRESSED,
    ];
    pen.at(
        lamp(view.shuffle, view.held(Control::Shuffle), shuffle),
        layout::SHUFFLE,
    );
    let repeat = [
        sprites::REPEAT_OFF,
        sprites::REPEAT_OFF_PRESSED,
        sprites::REPEAT_ON,
        sprites::REPEAT_ON_PRESSED,
    ];
    pen.at(lamp(view.repeat, view.held(Control::Repeat), repeat), layout::REPEAT);
}
