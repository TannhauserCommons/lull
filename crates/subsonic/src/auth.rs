//! Who may call `/rest/*` (ADR-0002). There are no accounts: the username is always `lull`, and
//! the password is an app password from `POST /api/devices/subsonic`, sent plain (`p=`, maybe
//! `enc:` hex) or as `t=md5(password + salt)` with `s=salt`. OpenSubsonic's `apiKey=` takes
//! any paired device's token, an app password included.

use lull_core::config::Secret;
use lull_core::db::Db;
use lull_core::devices::{self, Device};
use md5::{Digest, Md5};

use serde_json::json;

use crate::Reply;
use crate::params::Params;
use crate::response::Fail;

/// The one username, as `POST /api/devices/subsonic` hands it out.
pub const USER: &str = "lull";

/// `getUser`: the one user, allowed what lull can do through Subsonic (listen, download,
/// playlists, scrobbles) and nothing it can't. Clients such as Feishin sign in with this call.
pub fn user(params: &Params) -> Result<Reply, Fail> {
    let name = params.required("username")?;
    if name != USER {
        return Err(Fail::not_found(format!("no user {name}; lull's one user is {USER}")));
    }
    Ok(Reply::of(
        "user",
        json!({
            "username": USER,
            "scrobblingEnabled": true,
            "adminRole": false,
            "settingsRole": false,
            "downloadRole": true,
            "uploadRole": false,
            "playlistRole": true,
            "coverArtRole": false,
            "commentRole": false,
            "podcastRole": false,
            "streamRole": true,
            "jukeboxRole": false,
            "shareRole": false,
            "videoConversionRole": false,
            "folder": [1],
        }),
    ))
}

/// The paired device the call comes from.
pub async fn check(db: &Db, params: &Params) -> Result<Device, Fail> {
    if let Some(key) = params.get("apiKey") {
        if ["u", "p", "t", "s"].iter().any(|k| params.has(k)) {
            return Err(Fail::conflicting_auth());
        }
        return devices::verify(db, key).await?.ok_or_else(Fail::invalid_api_key);
    }
    let user = params.required("u")?;
    let presented = Presented::of(params)?;
    let passwords = devices::subsonic(db).await?;
    match passwords.into_iter().find(|(_, password)| presented.matches(password)) {
        Some((device, _)) if user == USER => Ok(device),
        _ => Err(Fail::wrong_credentials()),
    }
}

enum Presented {
    Password(String),
    Token { token: String, salt: String },
}

impl Presented {
    fn of(params: &Params) -> Result<Self, Fail> {
        match (params.get("p"), params.get("t"), params.get("s")) {
            (Some(p), None, None) => Ok(Presented::Password(decode_password(p)?)),
            (None, Some(t), Some(s)) => Ok(Presented::Token {
                token: t.to_ascii_lowercase(),
                salt: s.to_owned(),
            }),
            (None, Some(_), None) => Err(Fail::missing("s")),
            (None, None, Some(_)) => Err(Fail::missing("t")),
            (None, None, None) => Err(Fail::missing("p, or t and s")),
            _ => Err(Fail::conflicting_auth()),
        }
    }

    fn matches(&self, password: &Secret) -> bool {
        match self {
            Presented::Password(p) => same(p, password.expose()),
            Presented::Token { token, salt } => same(token, &token_of(password.expose(), salt)),
        }
    }
}

/// `enc:` is the password's UTF-8 bytes in hex, an old Subsonic obfuscation.
fn decode_password(p: &str) -> Result<String, Fail> {
    let Some(hex) = p.strip_prefix("enc:") else {
        return Ok(p.to_owned());
    };
    let bad = || Fail::wrong_credentials();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| {
            hex.get(i..i + 2)
                .and_then(|b| u8::from_str_radix(b, 16).ok())
                .ok_or_else(bad)
        })
        .collect::<Result<Vec<u8>, _>>()?;
    String::from_utf8(bytes).map_err(|_| bad())
}

/// `md5(password + salt)` as lowercase hex.
pub fn token_of(password: &str, salt: &str) -> String {
    Md5::digest(format!("{password}{salt}"))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Compares in time that depends only on the lengths, so a guess can't be refined by timing.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}
