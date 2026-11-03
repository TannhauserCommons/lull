//! The main window drawn from the skin at 1x, the way Winamp 2 drew it: the background, then each
//! control's sprite at its place, the time in the skin's digits, the song in its bitmap font and
//! the analyser in its colours; or rolled up (shade), its title bar with the time, a small
//! analyser and the position. Also what the three windows share: their controls and the view they
//! draw from.

use super::canvas::Pen;
use super::skin::{Area, Sprite, layout, sprites};
use super::sound::Now;
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
    Position,
    Time,
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
/// along the way it slides (0 for a button).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub control: Control,
    pub area: Area,
    pub knob: u32,
}

/// What the windows show beyond the skin.
pub struct View<'a> {
    pub now: &'a Now,
    pub marquee: &'a str,
    pub volume: f64,
    pub focused: bool,
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
const MAIN: [(Control, Area); 17] = [
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
    (Control::Position, layout::POSITION),
    (Control::Time, Area::new(layout::MINUS_EX.x, layout::MINUS_EX.y, 63, 13)),
    (Control::EqWindow, layout::EQ_BUTTON),
    (Control::PlaylistWindow, layout::PLAYLIST_BUTTON),
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
    let knob = match (control, shaded) {
        (Control::Volume, _) => 14,
        (Control::Position, false) => 29,
        (Control::Position, true) => 3,
        _ => 0,
    };
    Some(Target { control, area, knob })
}

pub fn main(pen: &mut Pen, view: &View) {
    pen.sprite(sprites::MAIN_BACKGROUND, 0, 0);
    title_bar(pen, view);
    pen.sprite(sprites::CLUTTER_BAR, layout::CLUTTER_BAR.x, layout::CLUTTER_BAR.y);
    status(pen, view.now);
    time(pen, view);
    visualiser(pen, view.bars);
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
    let bar = if view.focused {
        sprites::SHADE_BAR_ACTIVE
    } else {
        sprites::SHADE_BAR_INACTIVE
    };
    pen.at(bar, layout::TITLE_BAR);
    for (control, normal, pressed) in [
        (
            Control::Options,
            sprites::OPTIONS_BUTTON,
            sprites::OPTIONS_BUTTON_PRESSED,
        ),
        (
            Control::Minimize,
            sprites::MINIMIZE_BUTTON,
            sprites::MINIMIZE_BUTTON_PRESSED,
        ),
        (Control::Shade, sprites::UNSHADE_BUTTON, sprites::UNSHADE_BUTTON_PRESSED),
        (Control::Close, sprites::CLOSE_BUTTON, sprites::CLOSE_BUTTON_PRESSED),
    ] {
        pen.at(if view.held(control) { pressed } else { normal }, main_area(control));
    }
    let now = view.now;
    if !now.stopped && (now.playing || !view.blink_off) {
        let length = now.duration.unwrap_or(0.0);
        let at = view.seeking.map_or(now.position, |f| f * length);
        let remaining = view.remaining && length > 0.0;
        let seconds = if remaining { length - at } else { at }.max(0.0) as u32;
        let sign = if remaining { '-' } else { ' ' };
        let text = format!("{sign}{:02}:{:02}", (seconds / 60).min(99), seconds % 60);
        pen.text(&text, layout::SHADE_TIME);
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
    pen.at(sprites::SHADE_POSITION_TRACK, layout::SHADE_POSITION);
    let length = now.duration.unwrap_or(0.0);
    if !now.stopped && length > 0.0 {
        let fraction = view.seeking.unwrap_or(now.position / length).clamp(0.0, 1.0);
        let travel = layout::SHADE_POSITION.width - 3;
        let offset = (fraction * f64::from(travel)).round() as u32;
        let thumb = match offset {
            0 => sprites::SHADE_POSITION_THUMB_LEFT,
            x if x >= travel => sprites::SHADE_POSITION_THUMB_RIGHT,
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
    let bar = if view.focused {
        sprites::TITLE_BAR_ACTIVE
    } else {
        sprites::TITLE_BAR_INACTIVE
    };
    pen.at(bar, layout::TITLE_BAR);
    for (control, normal, pressed) in [
        (
            Control::Options,
            sprites::OPTIONS_BUTTON,
            sprites::OPTIONS_BUTTON_PRESSED,
        ),
        (
            Control::Minimize,
            sprites::MINIMIZE_BUTTON,
            sprites::MINIMIZE_BUTTON_PRESSED,
        ),
        (Control::Shade, sprites::SHADE_BUTTON, sprites::SHADE_BUTTON_PRESSED),
        (Control::Close, sprites::CLOSE_BUTTON, sprites::CLOSE_BUTTON_PRESSED),
    ] {
        pen.at(if view.held(control) { pressed } else { normal }, main_area(control));
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
    let working = if now.loading {
        sprites::WORK_INDICATOR_ON
    } else {
        sprites::WORK_INDICATOR_OFF
    };
    pen.at(working, layout::WORK_INDICATOR);
    let sounding = !now.stopped && !now.loading;
    let (stereo, mono) = match (sounding, now.stereo) {
        (false, _) => (sprites::STEREO_OFF, sprites::MONO_OFF),
        (true, true) => (sprites::STEREO_ON, sprites::MONO_OFF),
        (true, false) => (sprites::STEREO_OFF, sprites::MONO_ON),
    };
    pen.at(stereo, layout::STEREO);
    pen.at(mono, layout::MONO);
}

/// Elapsed, or remaining with a minus sign; blank when stopped and in the off half of a pause's
/// blink. The blank digit is drawn, not left out: under the digits is the skin's own idea of an
/// empty display, which is not always empty.
fn time(pen: &mut Pen, view: &View) {
    let extended = pen.skin.has_extended_digits();
    let blank = if extended {
        sprites::NUMS_EX_BLANK
    } else {
        sprites::NUMBERS_BLANK
    };
    let now = view.now;
    if now.stopped || (!now.playing && view.blink_off) {
        for cell in layout::TIME_DIGITS {
            pen.at(blank, cell);
        }
        minus(pen, extended, false);
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
    minus(pen, extended, remaining);
}

fn minus(pen: &mut Pen, extended: bool, on: bool) {
    match (extended, on) {
        (true, true) => pen.at(sprites::NUMS_EX_MINUS, layout::MINUS_EX),
        (true, false) => pen.at(sprites::NUMS_EX_BLANK, layout::MINUS_EX),
        (false, true) => pen.at(sprites::NUMBERS_MINUS, layout::MINUS),
        (false, false) => pen.at(sprites::NUMBERS_NO_MINUS, layout::MINUS),
    }
}

/// The analyser: the skin's background and dot grid, nineteen bars in its sixteen row colours, and
/// the peaks.
fn visualiser(pen: &mut Pen, bars: &[Bar; BARS]) {
    let area = layout::VISUALIZER;
    let palette = pen.skin.vis_colors;
    pen.fill(area.x, area.y, area.width, area.height, palette[0]);
    for y in (0..area.height).step_by(2) {
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
    let frame = (level * f64::from(sprites::SLIDER_FRAMES - 1)).round() as u32;
    pen.at(sprites::volume_frame(frame), layout::VOLUME);
    let thumb = if view.held(Control::Volume) {
        sprites::VOLUME_THUMB_PRESSED
    } else {
        sprites::VOLUME_THUMB
    };
    let x = layout::VOLUME.x + (level * f64::from(layout::VOLUME_TRAVEL)).round() as u32;
    pen.sprite(thumb, x, layout::VOLUME.y + 1);
    // Balance stays centred: Classic's player plays both channels as they are.
    pen.at(sprites::balance_frame(0), layout::BALANCE);
    pen.sprite(sprites::BALANCE_THUMB, layout::BALANCE.x + 12, layout::BALANCE.y + 1);
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
    let length = view.now.duration.unwrap_or(0.0);
    if !view.now.stopped && length > 0.0 {
        let fraction = view.seeking.unwrap_or(view.now.position / length).clamp(0.0, 1.0);
        let thumb = if view.seeking.is_some() {
            sprites::POSITION_THUMB_PRESSED
        } else {
            sprites::POSITION_THUMB
        };
        let x = layout::POSITION.x + (fraction * f64::from(layout::POSITION_TRAVEL)).round() as u32;
        pen.sprite(thumb, x, layout::POSITION.y);
    }
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
