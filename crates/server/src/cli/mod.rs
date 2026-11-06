//! lull's commands on the hub's own computer (ADR-0090): `status`, `setup`, `config` and `pair`.
//! They talk to the running hub over its admin socket (`admin.rs`), so a setting has one place,
//! the one the web page changes, and is checked the same way.

mod setup;

use std::path::{Path, PathBuf};

use reqwest::Method;
use serde_json::{Value, json};

use crate::BoxError;
use crate::admin::SOCKET;

pub const USAGE: &str = "\
lull, the music hub

  sudo lull status                what runs, its addresses and how to reach it from outside
  sudo lull setup                 a few questions: your music, Soulseek, sharing, outside access
  sudo lull pair                  a link that pairs a phone or computer (10 minutes)
  sudo lull config                every setting
  sudo lull config get KEY        one setting
  sudo lull config set KEY VALUE  change one; lull restarts its parts to use it
  sudo lull config unset KEY      back to its default
  lull serve                      run the hub (the lull service does this)
";

/// Runs one command; `args` are what follows `lull`.
pub async fn run(args: &[String]) -> Result<(), BoxError> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["status"] => status(&Hub::connect()?).await,
        ["setup"] => setup::run(&Hub::connect()?).await,
        ["pair"] => pair(&Hub::connect()?).await,
        ["config"] => list(&Hub::connect()?).await,
        ["config", "get", key] => get(&Hub::connect()?, key).await,
        ["config", "set", key, value @ ..] if !value.is_empty() => {
            set(&Hub::connect()?, key, Some(value.join(" "))).await
        }
        ["config", "unset", key] => set(&Hub::connect()?, key, None).await,
        ["help" | "--help" | "-h"] => {
            print!("{USAGE}");
            Ok(())
        }
        _ => {
            print!("{USAGE}");
            Err("unknown command".into())
        }
    }
}

/// The running hub, over its admin socket.
pub(crate) struct Hub {
    http: reqwest::Client,
    data: PathBuf,
}

impl Hub {
    /// The socket in `LULL_DATA_DIR`, else in the Linux package's data folder, else the image's.
    fn connect() -> Result<Hub, BoxError> {
        let candidates: Vec<PathBuf> = match std::env::var_os("LULL_DATA_DIR").filter(|d| !d.is_empty()) {
            Some(dir) => vec![dir.into()],
            None => vec!["/var/lib/lull".into(), "/data".into()],
        };
        let data = candidates
            .iter()
            .find(|d| d.join(SOCKET).exists())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "No lull hub runs here: no {SOCKET} in {}. Start it with: sudo systemctl start lull",
                    candidates
                        .iter()
                        .map(|d| d.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" or ")
                )
            })?;
        let http = reqwest::Client::builder().unix_socket(data.join(SOCKET)).build()?;
        Ok(Hub { http, data })
    }

    pub(crate) fn data(&self) -> &Path {
        &self.data
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Value, BoxError> {
        self.call(Method::GET, path, None).await
    }

    pub(crate) async fn send(&self, method: Method, path: &str, body: Value) -> Result<Value, BoxError> {
        self.call(method, path, Some(body)).await
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, BoxError> {
        let mut req = self.http.request(method, format!("http://lull{path}"));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.map_err(|e| self.unreachable(&e))?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let said = body["message"]
            .as_str()
            .map_or_else(|| status.to_string(), str::to_owned);
        Err(said.into())
    }

    fn unreachable(&self, e: &reqwest::Error) -> BoxError {
        let denied = std::fs::metadata(self.data.join(SOCKET)).is_ok()
            && std::os::unix::net::UnixStream::connect(self.data.join(SOCKET))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
        if denied {
            "Only root and the hub's own user can talk to the hub. Run it with sudo.".into()
        } else {
            format!("The hub doesn't answer ({e}). Is it running? sudo systemctl status lull").into()
        }
    }
}

/// `lull status`.
async fn status(hub: &Hub) -> Result<(), BoxError> {
    let host = hub.get("/api/host").await?;
    let health = hub.get("/health").await?;
    println!("lull {} · data in {}", text(&host["version"]), hub.data().display());
    println!();
    if let Some(services) = health["services"].as_object() {
        for (name, s) in services {
            println!(
                "  {:<11} {:<5} {}",
                part_name(name),
                text(&s["state"]),
                text(&s["detail"])
            );
        }
    }
    println!();
    if host["paired"] == true {
        println!("Paired: yes. Pair another device with: sudo lull pair");
    } else {
        println!("Paired: not yet. Pair your first device with: sudo lull pair");
    }
    addresses(&host);
    Ok(())
}

/// Where devices reach the hub: on this network, and from outside the home.
pub(crate) fn addresses(host: &Value) {
    let port = host["port"].as_u64().unwrap_or(8484);
    let public = host["publicUrl"].as_str().filter(|u| u.starts_with("https://"));
    println!();
    if let Some(lan) = host["lan"].as_str() {
        let name = crate::managed::advert::computer_name().map_or_else(String::new, |n| format!(" as \"lull on {n}\""));
        println!("On this network: http://{lan}:{port} (the apps also find it{name})");
    }
    match public {
        Some(url) => println!("From outside your home: {url}"),
        None => {
            println!("From outside your home: not set up.");
            println!("  With Tailscale on this computer: sudo lull setup");
            println!(
                "  Behind a reverse proxy or your own domain (proxy it to port {port}): sudo lull config set LULL_PUBLIC_URL https://your.address"
            );
        }
    }
}

/// `lull pair`.
async fn pair(hub: &Hub) -> Result<(), BoxError> {
    print_pair_code(hub).await
}

pub(crate) async fn print_pair_code(hub: &Hub) -> Result<(), BoxError> {
    let code = hub.send(Method::POST, "/api/pair/codes", json!({})).await?;
    let host = hub.get("/api/host").await?;
    println!("To pair a phone or computer, open this on it within 10 minutes:");
    println!("  {}", text(&code["url"]));
    // The link carries the outside address; a device at home may not reach that one.
    if let Some(lan) = host["lan"].as_str() {
        let home = format!("http://{lan}:{}", host["port"].as_u64().unwrap_or(8484));
        if !text(&code["url"]).starts_with(&home) {
            println!("  at home: {home}/#/pair/{}", text(&code["code"]));
        }
    }
    println!(
        "or, in the lull app, choose Connect to a hub and enter the code {}",
        text(&code["code"])
    );
    Ok(())
}

/// `lull config`.
async fn list(hub: &Hub) -> Result<(), BoxError> {
    let config = hub.get("/api/config").await?;
    for item in config["items"].as_array().into_iter().flatten() {
        println!("{:<26} {}", text(&item["key"]), shown(item));
    }
    Ok(())
}

/// `lull config get KEY`.
async fn get(hub: &Hub, key: &str) -> Result<(), BoxError> {
    let config = hub.get("/api/config").await?;
    let item = config["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["key"] == key)
        .ok_or_else(|| format!("{key} isn't a setting; lull config lists them"))?;
    println!("{}", shown(item));
    Ok(())
}

/// `lull config set KEY VALUE` and `lull config unset KEY`. The library's source switches
/// through its own route, which moves the library over first.
async fn set(hub: &Hub, key: &str, value: Option<String>) -> Result<(), BoxError> {
    if key == "LULL_LIBRARY" {
        let mode = value.unwrap_or_else(|| "folder".into());
        hub.send(Method::PUT, "/api/library/source", json!({ "mode": mode }))
            .await?;
        println!("The library is switching to {mode}; lull status shows when it's done.");
        return Ok(());
    }
    let mut change = serde_json::Map::new();
    change.insert(key.to_owned(), json!(value));
    hub.send(Method::PATCH, "/api/config", Value::Object(change)).await?;
    println!("Saved. lull restarts its parts to use it.");
    Ok(())
}

/// A setting as `lull config` shows it: a secret only says whether it is set.
fn shown(item: &Value) -> String {
    let value = if item["secret"] == true {
        if item["set"] == true { "(set)" } else { "(not set)" }.to_owned()
    } else {
        item["value"]
            .as_str()
            .map_or_else(|| "(default)".to_owned(), str::to_owned)
    };
    match item["source"].as_str() {
        Some("env") => format!("{value}  [set by the service, can't change here]"),
        _ => value,
    }
}

/// A part's name as Settings shows it.
fn part_name(name: &str) -> String {
    match name {
        "youtube" => "YouTube".into(),
        "lastfm" => "Last.fm".into(),
        "listenbrainz" => "ListenBrainz".into(),
        "acoustid" => "AcoustID".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

pub(crate) fn text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}
