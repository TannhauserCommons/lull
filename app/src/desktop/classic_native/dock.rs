//! Winamp 2's windows apart and docked: where the main window, the equaliser and the playlist are
//! on the screen, as Winamp keeps them. A window dragged near another's edge, or near a screen's
//! work area's, snaps to it (Winamp's snap distance, 10 pixels by default); windows whose edges
//! touch are docked. Dragging the main window moves it with everything docked to it, directly or
//! through another; dragging the equaliser or the playlist moves it alone, which is how it comes
//! away. A window that changes size (rolled up, the playlist resized, Classic's size) keeps what is
//! docked to its bottom and right edges docked: they move by the change, and what is docked to them
//! with them. A window never stays off every screen. The snapping is Webamp's (`snapUtils`), at
//! Winamp's distance.
//!
//! Everything here is physical pixels of the desktop.

/// A window's or a work area's place: its top left, width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width, height }
    }

    fn right(self) -> i32 {
        self.x + self.width
    }

    fn bottom(self) -> i32 {
        self.y + self.height
    }

    fn moved(self, (dx, dy): (i32, i32)) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    /// How much of it lies in `other`.
    fn overlap(self, other: Rect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0);
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0);
        i64::from(w) * i64::from(h)
    }

    /// The smallest rectangle holding all of them.
    fn around(rects: impl IntoIterator<Item = Rect>) -> Option<Rect> {
        rects.into_iter().reduce(|a, b| {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
        })
    }
}

/// Winamp's snap distance (Preferences, "Snap windows at 10 pixels"), at the screen's scale.
pub fn snap_distance(dpi: f64) -> i32 {
    (10.0 * dpi).round() as i32
}

/// Where `a` would go to snap to `b` across and down, if anywhere: its edge to `b`'s facing edge,
/// or its edge in line with `b`'s same edge, when they're within `d` and the two overlap the other
/// way (Webamp's `snap`, the first that holds of each four).
fn snap(a: Rect, b: Rect, d: i32) -> (Option<i32>, Option<i32>) {
    let near = |p: i32, q: i32| (p - q).abs() < d;
    let overlap_x = a.x <= b.right() + d && b.x <= a.right() + d;
    let overlap_y = a.y <= b.bottom() + d && b.y <= a.bottom() + d;
    let x = overlap_y.then(|| {
        if near(a.x, b.right()) {
            Some(b.right())
        } else if near(a.right(), b.x) {
            Some(b.x - a.width)
        } else if near(a.x, b.x) {
            Some(b.x)
        } else if near(a.right(), b.right()) {
            Some(b.right() - a.width)
        } else {
            None
        }
    });
    let y = overlap_x.then(|| {
        if near(a.y, b.bottom()) {
            Some(b.bottom())
        } else if near(a.bottom(), b.y) {
            Some(b.y - a.height)
        } else if near(a.y, b.y) {
            Some(b.y)
        } else if near(a.bottom(), b.bottom()) {
            Some(b.bottom() - a.height)
        } else {
            None
        }
    });
    (x.flatten(), y.flatten())
}

/// Where `a` would go to keep within `area` when within `d` of its edges (Webamp's `snapWithin`).
fn snap_within(a: Rect, area: Rect, d: i32) -> (Option<i32>, Option<i32>) {
    let x = if (a.x - area.x).abs() < d {
        Some(area.x)
    } else if (area.right() - a.right()).abs() < d {
        Some(area.right() - a.width)
    } else {
        None
    };
    let y = if (a.y - area.y).abs() < d {
        Some(area.y)
    } else if (area.bottom() - a.bottom()).abs() < d {
        Some(area.bottom() - a.height)
    } else {
        None
    };
    (x, y)
}

/// Whether two windows are docked: an edge of one on the facing edge of the other, overlapping
/// along it.
pub fn docked(a: Rect, b: Rect) -> bool {
    let across = a.x < b.right() && b.x < a.right();
    let down = a.y < b.bottom() && b.y < a.bottom();
    ((a.bottom() == b.y || b.bottom() == a.y) && across) || ((a.right() == b.x || b.right() == a.x) && down)
}

/// The windows docked to `from`, directly or through another, `from` among them (Webamp's
/// `traceConnection`).
pub fn docked_set(windows: &[Option<Rect>], from: usize) -> Vec<usize> {
    let mut set = vec![from];
    let mut at = 0;
    while let Some(&window) = set.get(at) {
        at += 1;
        let Some(rect) = windows.get(window).copied().flatten() else {
            continue;
        };
        for (other, candidate) in windows.iter().enumerate() {
            if let Some(candidate) = candidate
                && !set.contains(&other)
                && docked(rect, *candidate)
            {
                set.push(other);
            }
        }
    }
    set
}

/// Windows `moving` (their places when the drag began), dragged `by`: how far they go once they
/// snap to the windows that stay and to the work areas' edges, the work area winning where both
/// snap (Webamp's order). Closed windows are `None`.
pub fn dragged(windows: &[Option<Rect>], moving: &[usize], by: (i32, i32), areas: &[Rect], d: i32) -> (i32, i32) {
    let moved: Vec<Rect> = moving
        .iter()
        .filter_map(|&i| windows.get(i).copied().flatten())
        .map(|r| r.moved(by))
        .collect();
    let staying: Vec<Rect> = windows
        .iter()
        .enumerate()
        .filter(|(i, _)| !moving.contains(i))
        .filter_map(|(_, r)| *r)
        .collect();
    let (mut dx, mut dy) = (None, None);
    for a in &moved {
        for b in &staying {
            let (x, y) = snap(*a, *b, d);
            dx = dx.or(x.map(|x| x - a.x));
            dy = dy.or(y.map(|y| y - a.y));
        }
    }
    if let Some(all) = Rect::around(moved.iter().copied()) {
        // The work area the windows are most in.
        if let Some(area) = areas.iter().max_by_key(|area| all.overlap(**area)) {
            let (x, y) = snap_within(all, *area, d);
            dx = x.map(|x| x - all.x).or(dx);
            dy = y.map(|y| y - all.y).or(dy);
        }
    }
    (by.0 + dx.unwrap_or(0), by.1 + dy.unwrap_or(0))
}

/// Window `changed` was `before` and is now its new size, its top left where it was: what is
/// docked to its bottom moves by the change in height, what is docked to its right by the change in
/// width, and what is docked to their bottoms and rights with them. The moves, window by window.
pub fn resized(windows: &[Option<Rect>], changed: usize, before: Rect) -> Vec<(usize, (i32, i32))> {
    let Some(after) = windows.get(changed).copied().flatten() else {
        return Vec::new();
    };
    let (dw, dh) = (after.width - before.width, after.height - before.height);
    let mut moves: Vec<(usize, (i32, i32))> = Vec::new();
    let push = |from: Rect, below: bool, by: (i32, i32), moves: &mut Vec<(usize, (i32, i32))>| {
        let mut queue = vec![from];
        while let Some(edge) = queue.pop() {
            for (i, rect) in windows.iter().enumerate() {
                let Some(rect) = *rect else {
                    continue;
                };
                let attached = if below {
                    rect.y == edge.bottom() && rect.x < edge.right() && edge.x < rect.right()
                } else {
                    rect.x == edge.right() && rect.y < edge.bottom() && edge.y < rect.bottom()
                };
                if i != changed && attached && !moves.iter().any(|(m, _)| *m == i) {
                    moves.push((i, by));
                    queue.push(rect);
                }
            }
        }
    };
    if dh != 0 {
        push(before, true, (0, dh), &mut moves);
    }
    if dw != 0 {
        let mut across = Vec::new();
        push(before, false, (dw, 0), &mut across);
        for (i, by) in across {
            match moves.iter_mut().find(|(m, _)| *m == i) {
                Some((_, (x, _))) => *x += by.0,
                None => moves.push((i, by)),
            }
        }
    }
    moves
}

/// Each window back on a screen if it has left them all (a monitor gone, a smaller one): into the
/// work area it was nearest, by as little as it takes; `together` (the main window's docked set)
/// as one. The moves, window by window.
pub fn on_screen(windows: &[Option<Rect>], together: &[usize], areas: &[Rect]) -> Vec<(usize, (i32, i32))> {
    let mut moves = Vec::new();
    let mut place = |group: &[usize]| {
        let Some(all) = Rect::around(group.iter().filter_map(|&i| windows.get(i).copied().flatten())) else {
            return;
        };
        if areas
            .iter()
            .any(|area| all.overlap(*area) == i64::from(all.width) * i64::from(all.height))
        {
            return;
        }
        let distance = |area: &Rect| {
            let dx = (area.x - all.x).max(all.right() - area.right()).max(0);
            let dy = (area.y - all.y).max(all.bottom() - area.bottom()).max(0);
            i64::from(dx) + i64::from(dy)
        };
        let Some(area) = areas.iter().min_by_key(|area| distance(area)) else {
            return;
        };
        let clamp = |p: i32, size: i32, start: i32, length: i32| p.min(start + length - size).max(start) - p;
        let by = (
            clamp(all.x, all.width, area.x, area.width),
            clamp(all.y, all.height, area.y, area.height),
        );
        moves.extend(group.iter().map(|&i| (i, by)));
    };
    place(together);
    for i in (0..windows.len()).filter(|i| !together.contains(i)) {
        place(&[i]);
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Rect = Rect::new(100, 100, 275, 116);
    const SCREEN: [Rect; 1] = [Rect::new(0, 0, 1920, 1040)];

    #[test]
    fn a_window_near_anothers_edge_snaps_to_it() {
        // The equaliser dragged to 6 px under the main window, a little to the right: it docks
        // under it, its left in line.
        let eq = Rect::new(104, 222, 275, 116);
        assert_eq!(snap(eq, MAIN, 10), (Some(100), Some(216)));
        // Too far below: nothing either way (they no longer overlap across).
        assert_eq!(snap(Rect::new(100, 230, 275, 116), MAIN, 10), (None, None));
        assert_eq!(snap(Rect::new(500, 500, 275, 116), MAIN, 10), (None, None));
    }

    #[test]
    fn the_main_window_drags_what_is_docked_to_it_and_the_others_come_away() {
        let windows = [
            Some(MAIN),
            Some(Rect::new(100, 216, 275, 116)),
            Some(Rect::new(375, 100, 275, 232)),
        ];
        assert_eq!(docked_set(&windows, 0), [0, 1, 2]);
        // The playlist docked to the equaliser's right, not to the main window's: still in the set.
        let chain = [
            Some(MAIN),
            Some(Rect::new(100, 216, 275, 116)),
            Some(Rect::new(375, 216, 275, 116)),
        ];
        assert_eq!(docked_set(&chain, 0), [0, 1, 2]);
        // Apart: only itself.
        let apart = [Some(MAIN), Some(Rect::new(600, 600, 275, 116)), None];
        assert_eq!(docked_set(&apart, 0), [0]);
    }

    #[test]
    fn a_dragged_window_snaps_to_the_others_and_the_work_area() {
        let windows = [Some(MAIN), Some(Rect::new(600, 600, 275, 116)), None];
        // The equaliser dragged to 4 px right of the main window's right edge, 3 px below its top.
        let by = (375 + 4 - 600, 103 - 600);
        assert_eq!(dragged(&windows, &[1], by, &SCREEN, 10), (375 - 600, 100 - 600));
        // The main window dragged to 7 px from the screen's left: it goes to the edge.
        assert_eq!(dragged(&windows, &[0], (-93, 0), &SCREEN, 10), (-100, 0));
    }

    #[test]
    fn rolling_a_window_up_brings_what_is_docked_under_it_along() {
        let shaded = Rect::new(100, 100, 275, 14);
        let windows = [
            Some(shaded),
            Some(Rect::new(100, 216, 275, 116)),
            Some(Rect::new(100, 332, 275, 116)),
            Some(Rect::new(700, 216, 275, 116)),
        ];
        // The equaliser under it, the playlist under that; the window off to the side stays.
        assert_eq!(resized(&windows, 0, MAIN), [(1, (0, -102)), (2, (0, -102))]);
    }

    #[test]
    fn a_wider_playlist_pushes_what_is_docked_to_its_right() {
        let playlist = Rect::new(100, 216, 350, 116);
        let windows = [Some(MAIN), Some(playlist), Some(Rect::new(375, 216, 275, 116))];
        assert_eq!(resized(&windows, 1, Rect::new(100, 216, 275, 116)), [(2, (75, 0))]);
    }

    #[test]
    fn a_window_off_every_screen_comes_back_and_the_docked_set_together() {
        let windows = [
            Some(Rect::new(2000, 100, 275, 116)),
            Some(Rect::new(2000, 216, 275, 116)),
            Some(Rect::new(-500, -500, 275, 116)),
        ];
        let moves = on_screen(&windows, &[0, 1], &SCREEN);
        assert_eq!(moves, [(0, (-355, 0)), (1, (-355, 0)), (2, (500, 500))]);
        // On screen already: nothing.
        assert!(on_screen(&[Some(MAIN), None, None], &[0], &SCREEN).is_empty());
    }
}
