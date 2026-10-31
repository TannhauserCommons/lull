//! LRC text to timed lines. A line may carry several timestamps (`[00:12.40][01:10.00] chorus`);
//! tag lines such as `[ar: …]` and malformed ones are dropped.

use crate::Line;

pub fn parse(text: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = text.lines().flat_map(timed).collect();
    lines.sort_by(|a, b| a.t.total_cmp(&b.t));
    lines
}

/// One LRC line: every leading `[mm:ss.xx]` stamp gives the same text a time.
fn timed(line: &str) -> Vec<Line> {
    let mut stamps = Vec::new();
    let mut rest = line.trim_start();
    while let Some(inner) = rest.strip_prefix('[') {
        let Some((stamp, after)) = inner.split_once(']') else {
            break;
        };
        let Some(t) = seconds(stamp) else { break };
        stamps.push(t);
        rest = after;
    }
    let text = rest.trim();
    stamps
        .into_iter()
        .map(|t| Line {
            t,
            text: text.to_owned(),
        })
        .collect()
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss:xx` to seconds.
fn seconds(stamp: &str) -> Option<f64> {
    let (min, sec) = stamp.trim().split_once(':')?;
    let min: u32 = min.parse().ok()?;
    let sec: f64 = sec.replacen(':', ".", 1).parse().ok()?;
    (sec.is_finite() && sec >= 0.0).then(|| f64::from(min) * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_stamps_and_tags_become_ordered_lines() {
        let lines = parse("[ar: Someone]\n[00:12.40] first\n[00:05.00][01:10.5]chorus\n[00:20.00]\nnot timed");
        let got: Vec<(f64, &str)> = lines.iter().map(|l| (l.t, l.text.as_str())).collect();
        assert_eq!(
            got,
            vec![(5.0, "chorus"), (12.4, "first"), (20.0, ""), (70.5, "chorus")]
        );
    }
}
