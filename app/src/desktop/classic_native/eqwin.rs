//! The equaliser window, under the main one, showing lull's equaliser (eq.rs) the way Webamp's
//! window shows it (web/desktop/webamp-session.js `linkEq`): ON, the preamp at the level the sound
//! gets (the automatic one unless set by hand), ten bands of ±12 dB and the curve through them.
//! Winamp's AUTO loaded a preset per song, which lull has no use for; here it is lull's automatic
//! level: lit while the level is automatic, and a press sets it by hand or back. PRESETS are lull's
//! own. Rolled up, it shows a small volume slider. Drawing and positions after Spotifast's
//! `src/ui/winamp/equalizer.rs` (MIT, credited in app/licenses/THIRD-PARTY.md).

use super::canvas::Pen;
use super::draw::{Control, Target, View};
use super::eq::{BANDS, MAX_DB};
use super::skin::{Area, HEIGHT, layout, sprites};

/// A slider's thumb, and how far it travels.
const THUMB: u32 = 11;
const TRAVEL: u32 = layout::EQ_PREAMP.height - THUMB;
/// The curve: a point every so many pixels across the graph, starting this far in.
const GRAPH_STEP: u32 = 12;
const GRAPH_PAD: u32 = 2;

/// The equaliser window's control under a point of it; `None` is the window itself, which drags.
pub fn at(x: u32, y: u32, shaded: bool) -> Option<Target> {
    let button = |control, area| Target { control, area, knob: 0 };
    let slider = |control, area| Target {
        control,
        area,
        knob: THUMB,
    };
    let mut controls = vec![
        button(Control::EqClose, layout::EQ_CLOSE),
        button(Control::EqShade, layout::EQ_SHADE),
    ];
    if shaded {
        controls.push(Target {
            control: Control::Volume,
            area: layout::EQ_SHADE_VOLUME,
            knob: 3,
        });
    } else {
        controls.extend([
            button(Control::EqOn, layout::EQ_ON),
            button(Control::EqAuto, layout::EQ_AUTO),
            button(Control::EqPresets, layout::EQ_PRESETS),
            slider(Control::EqPreamp, layout::EQ_PREAMP),
        ]);
        controls.extend((0..BANDS.len()).map(|band| slider(Control::EqBand(band), layout::eq_band(band))));
    }
    controls.into_iter().find(|t| t.area.contains(x, y))
}

/// A gain as a fraction of a slider, 0 at the bottom, and back.
pub fn fraction(db: f64) -> f64 {
    ((db + MAX_DB) / (2.0 * MAX_DB)).clamp(0.0, 1.0)
}

pub fn decibels(fraction: f64) -> f64 {
    fraction * 2.0 * MAX_DB - MAX_DB
}

pub fn window(pen: &mut Pen, view: &View) {
    pen.sprite(sprites::EQ_BACKGROUND, 0, 0);
    let bar = if view.focused {
        sprites::EQ_TITLE_BAR_ACTIVE
    } else {
        sprites::EQ_TITLE_BAR_INACTIVE
    };
    pen.at(bar, layout::EQ_TITLE_BAR);
    let close = if view.held(Control::EqClose) {
        sprites::EQ_CLOSE_BUTTON_PRESSED
    } else {
        sprites::EQ_CLOSE_BUTTON
    };
    pen.at(close, layout::EQ_CLOSE);
    if view.held(Control::EqShade) {
        pen.at(sprites::EQ_SHADE_BUTTON_PRESSED, layout::EQ_SHADE);
    }
    let eq = view.eq;
    let on = match (eq.on, view.held(Control::EqOn)) {
        (true, true) => sprites::EQ_ON_ON_PRESSED,
        (true, false) => sprites::EQ_ON_ON,
        (false, true) => sprites::EQ_ON_OFF_PRESSED,
        (false, false) => sprites::EQ_ON_OFF,
    };
    pen.at(on, layout::EQ_ON);
    let auto = match (eq.preamp.is_none(), view.held(Control::EqAuto)) {
        (true, true) => sprites::EQ_AUTO_ON_PRESSED,
        (true, false) => sprites::EQ_AUTO_ON,
        (false, true) => sprites::EQ_AUTO_OFF_PRESSED,
        (false, false) => sprites::EQ_AUTO_OFF,
    };
    pen.at(auto, layout::EQ_AUTO);
    let presets = if view.held(Control::EqPresets) {
        sprites::EQ_PRESETS_PRESSED
    } else {
        sprites::EQ_PRESETS
    };
    pen.at(presets, layout::EQ_PRESETS);
    let level = eq.level().clamp(-MAX_DB, MAX_DB);
    graph(pen, view, level);
    slider(pen, layout::EQ_PREAMP, fraction(level), view.held(Control::EqPreamp));
    for (band, db) in eq.bands.iter().enumerate() {
        slider(
            pen,
            layout::eq_band(band),
            fraction(*db),
            view.held(Control::EqBand(band)),
        );
    }
    let mask = pen.skin.regions.equaliser.clone();
    pen.mask(mask.as_ref(), HEIGHT);
}

/// Rolled up: its bar, the volume as a small slider (its thumb changes look by thirds), balance
/// centred, and its buttons.
pub fn shaded(pen: &mut Pen, view: &View) {
    let bar = if view.focused {
        sprites::EQ_SHADE_BAR_ACTIVE
    } else {
        sprites::EQ_SHADE_BAR_INACTIVE
    };
    pen.at(bar, layout::EQ_TITLE_BAR);
    if view.held(Control::EqShade) {
        pen.at(sprites::EQ_UNSHADE_BUTTON_PRESSED, layout::EQ_SHADE);
    }
    let close = if view.held(Control::EqClose) {
        sprites::EQ_SHADE_CLOSE_BUTTON_PRESSED
    } else {
        sprites::EQ_SHADE_CLOSE_BUTTON
    };
    pen.at(close, layout::EQ_CLOSE);
    let volume = view.volume.clamp(0.0, 1.0);
    let thumb = match volume {
        v if v < 1.0 / 3.0 => sprites::EQ_SHADE_VOLUME_THUMB_LOW,
        v if v < 2.0 / 3.0 => sprites::EQ_SHADE_VOLUME_THUMB_MIDDLE,
        _ => sprites::EQ_SHADE_VOLUME_THUMB_HIGH,
    };
    let area = layout::EQ_SHADE_VOLUME;
    pen.sprite(
        thumb,
        area.x + (volume * f64::from(area.width - 3)).round() as u32,
        area.y,
    );
    let balance = layout::EQ_SHADE_BALANCE;
    pen.sprite(
        sprites::EQ_SHADE_BALANCE_THUMB_MIDDLE,
        balance.x + (balance.width - 3) / 2,
        balance.y,
    );
    let mask = pen.skin.regions.equaliser_shade.clone();
    pen.mask(mask.as_ref(), layout::SHADE_HEIGHT);
}

/// A vertical slider: its track drawn at the value, the thumb riding on it.
fn slider(pen: &mut Pen, area: Area, value: f64, held: bool) {
    let frame = (value * f64::from(sprites::EQ_SLIDER_FRAMES - 1)).round() as u32;
    pen.at(sprites::eq_slider_frame(frame), area);
    let thumb = if held {
        sprites::EQ_THUMB_PRESSED
    } else {
        sprites::EQ_THUMB
    };
    let y = area.y + ((1.0 - value) * f64::from(TRAVEL)).round() as u32;
    pen.sprite(thumb, area.x + 1, y);
}

/// The curve through the bands in the skin's colours row by row, and the level's line under it.
fn graph(pen: &mut Pen, view: &View, level: f64) {
    let area = layout::EQ_GRAPH;
    pen.at(sprites::EQ_GRAPH, area);
    let rows = area.height;
    let row_of = |value: f64| ((1.0 - value) * f64::from(rows - 1)).round() as u32;
    pen.sprite(sprites::EQ_PREAMP_LINE, area.x, area.y + row_of(fraction(level)));
    let points: Vec<f64> = view.eq.bands.iter().map(|db| fraction(*db)).collect();
    let width = GRAPH_STEP * (BANDS.len() as u32 - 1);
    let mut last = row_of(points[0]);
    for x in 0..=width {
        let row = row_of(spline(&points, f64::from(x) / f64::from(GRAPH_STEP)));
        let (top, bottom) = if row < last { (row, last) } else { (last, row) };
        for r in top..=bottom {
            let line = sprites::EQ_GRAPH_LINE;
            if let Some(color) = pen.sheet_color(line, 0, r.min(line.height - 1)) {
                pen.put(area.x + GRAPH_PAD + x, area.y + r, color);
            }
        }
        last = row;
    }
}

/// A smooth curve through evenly spaced points (Catmull-Rom), at `t` measured in points.
fn spline(points: &[f64], t: f64) -> f64 {
    let last = points.len() - 1;
    let i = (t.floor() as usize).min(last.saturating_sub(1));
    let u = (t - i as f64).clamp(0.0, 1.0);
    let at = |index: isize| points[index.clamp(0, last as isize) as usize];
    let (p0, p1, p2, p3) = (
        at(i as isize - 1),
        at(i as isize),
        at(i as isize + 1),
        at(i as isize + 2),
    );
    let value = 0.5
        * (2.0 * p1
            + (-p0 + p2) * u
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u * u
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * u * u * u);
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slider_maps_twelve_db_either_way_and_back() {
        assert_eq!((fraction(-12.0), fraction(0.0), fraction(12.0)), (0.0, 0.5, 1.0));
        assert_eq!(decibels(fraction(6.0)), 6.0);
    }

    #[test]
    fn the_curve_passes_through_the_bands() {
        let points = [0.2, 0.8, 0.5, 0.5];
        for (i, p) in points.iter().enumerate() {
            assert!((spline(&points, i as f64) - p).abs() < 1e-9);
        }
    }
}
