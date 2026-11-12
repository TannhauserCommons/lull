//! Source presets (ADR-0017): named streaming profiles, one per device. Collection stores them
//! with the settings (`sources.presets`, `sources.devices`) and publishes them in the `settings`
//! event; soulseek streams for each request by the preset of the device that asks.

use std::collections::BTreeMap;

use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use serde::{Deserialize, Serialize};

use crate::quality::{FORMATS, Pick, StreamProfile, is_lossless};

/// The built-in presets' ids. They always exist; they can be edited, not renamed or deleted.
pub const DESKTOP: &str = "desktop";
pub const PHONE: &str = "phone";

/// The cap the Phone preset starts with, in kbps.
const PHONE_MAX_KBPS: u32 = 320;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub stream: StreamProfile,
}

impl Preset {
    pub fn builtin(&self) -> bool {
        is_builtin(&self.id)
    }
}

pub fn is_builtin(id: &str) -> bool {
    id == DESKTOP || id == PHONE
}

/// Desktop with `desktop` (the profile a hub had before presets, or the default), and Phone made
/// from it.
pub fn builtins(desktop: StreamProfile) -> Vec<Preset> {
    let phone = phone_from(&desktop);
    vec![
        Preset {
            id: DESKTOP.into(),
            name: "Desktop".into(),
            stream: desktop,
        },
        Preset {
            id: PHONE.into(),
            name: "Phone".into(),
            stream: phone,
        },
    ]
}

/// The Phone preset made from Desktop's profile: fastest first, at most 320 kbps (or Desktop's
/// lower cap), the lossless formats off and not lossless only; the rest as Desktop's.
pub fn phone_from(desktop: &StreamProfile) -> StreamProfile {
    let every_lossy = || -> Vec<String> {
        FORMATS
            .iter()
            .filter(|f| !is_lossless(f))
            .map(|f| (*f).to_owned())
            .collect()
    };
    let named: Vec<String> = match desktop.minimum.formats.as_slice() {
        [] => every_lossy(),
        some => some.iter().filter(|f| !is_lossless(f)).cloned().collect(),
    };
    // Desktop named only lossless formats: every lossy one, since some format must stream.
    let formats = if named.is_empty() { every_lossy() } else { named };
    let max_kbps = match desktop.max_kbps {
        0 => PHONE_MAX_KBPS,
        cap => cap.min(PHONE_MAX_KBPS),
    };
    let mut phone = desktop.clone();
    phone.minimum.lossless_only = false;
    phone.minimum.formats = formats;
    phone.minimum.kbps = phone.minimum.kbps.min(max_kbps);
    phone.max_kbps = max_kbps;
    phone.pick = Pick::Fastest;
    phone
}

/// Whether the request comes from a phone or tablet: its `User-Agent` names Android, an iPhone,
/// iPad or iPod, or "Mobi" (every mobile browser's "Mobile"); it sends `Sec-CH-UA-Mobile: ?1`; or
/// it is the Android app's own player, which asks as `lull-app`. Anything else, a computer.
pub fn phone(headers: &HeaderMap) -> bool {
    let text = |name| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default();
    if text("sec-ch-ua-mobile").trim() == "?1" {
        return true;
    }
    let agent = text(USER_AGENT.as_str());
    agent.starts_with("lull-app")
        || ["Android", "iPhone", "iPad", "iPod", "Mobi"]
            .iter()
            .any(|w| agent.contains(w))
}

/// The preset a device uses, and whether it chose it (else it is its kind's default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Used<'a> {
    pub preset: &'a Preset,
    pub chosen: bool,
}

/// The preset `device` uses: the one it chose (`devices`), else Phone for a phone and Desktop
/// for anything else. `None` only for a list without its built-ins, which the store never holds.
pub fn used<'a>(
    presets: &'a [Preset],
    devices: &BTreeMap<String, String>,
    device: Option<&str>,
    phone: bool,
) -> Option<Used<'a>> {
    let find = |id: &str| presets.iter().find(|p| p.id == id);
    if let Some(chosen) = device.and_then(|d| devices.get(d)).and_then(|id| find(id)) {
        return Some(Used {
            preset: chosen,
            chosen: true,
        });
    }
    find(if phone { PHONE } else { DESKTOP }).map(|preset| Used { preset, chosen: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::{Minimum, StreamBelow};

    fn agent(ua: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = ua.parse() {
            h.insert(USER_AGENT, v);
        }
        h
    }

    #[test]
    fn phones_tablets_and_the_android_apps_player_are_phones_and_computers_are_not() {
        let phones = [
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
            "Mozilla/5.0 (Linux; Android 14; Pixel 8; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/130.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Android 14; Mobile; rv:132.0) Gecko/132.0 Firefox/132.0",
            "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0 Mobile/15E148 Safari/604.1",
            "lull-app",
        ];
        for ua in phones {
            assert!(phone(&agent(ua)), "{ua}");
        }
        let computers = [
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15",
            "",
        ];
        for ua in computers {
            assert!(!phone(&agent(ua)), "{ua}");
        }
        let mut hinted = agent("Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36");
        hinted.insert("sec-ch-ua-mobile", axum::http::HeaderValue::from_static("?1"));
        assert!(phone(&hinted));
        let mut desktop_hint = agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
        desktop_hint.insert("sec-ch-ua-mobile", axum::http::HeaderValue::from_static("?0"));
        assert!(!phone(&desktop_hint));
    }

    #[test]
    fn phone_is_desktop_fastest_first_capped_at_320_without_lossless() {
        let desktop = StreamProfile {
            minimum: Minimum {
                lossless_only: false,
                kbps: 256,
                formats: Vec::new(),
            },
            pick: Pick::Highest,
            below: StreamBelow::KeepLooking,
            max_kbps: 0,
        };
        let phone = phone_from(&desktop);
        assert_eq!(phone.max_kbps, 320);
        assert_eq!(phone.pick, Pick::Fastest);
        assert_eq!(phone.below, StreamBelow::KeepLooking, "the rest is Desktop's");
        assert_eq!(phone.minimum.kbps, 256);
        assert_eq!(phone.minimum.formats, ["m4a", "aac", "mp3", "ogg", "opus"]);

        let lossless = StreamProfile {
            minimum: Minimum {
                lossless_only: true,
                kbps: 0,
                formats: vec!["flac".into()],
            },
            max_kbps: 192,
            ..desktop
        };
        let phone = phone_from(&lossless);
        assert!(
            !phone.minimum.lossless_only,
            "lossless only would leave a phone nothing"
        );
        assert_eq!(phone.minimum.formats, ["m4a", "aac", "mp3", "ogg", "opus"]);
        assert_eq!(phone.max_kbps, 192, "a lower cap of Desktop's stays");
    }

    #[test]
    fn a_device_uses_its_choice_else_its_kinds_default() {
        let mut presets = builtins(StreamProfile::default());
        presets.push(Preset {
            id: "p-car".into(),
            name: "Car".into(),
            stream: StreamProfile::default(),
        });
        let devices = BTreeMap::from([
            ("d-1".to_owned(), "p-car".to_owned()),
            ("d-2".to_owned(), "p-gone".to_owned()),
        ]);
        let id = |device, phone| used(&presets, &devices, device, phone).map(|u| (u.preset.id.as_str(), u.chosen));
        assert_eq!(id(Some("d-1"), true), Some(("p-car", true)));
        assert_eq!(id(Some("d-3"), true), Some((PHONE, false)));
        assert_eq!(id(Some("d-3"), false), Some((DESKTOP, false)));
        assert_eq!(id(None, false), Some((DESKTOP, false)));
        assert_eq!(id(Some("d-2"), false), Some((DESKTOP, false)), "a preset that is gone");
    }
}
