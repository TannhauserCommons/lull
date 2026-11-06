//! `lull setup` (ADR-0090): a few plain questions with the current answers as defaults, then
//! the same changes the web page makes, through the hub. Your music (a folder, or an existing
//! Jellyfin and the folder it reads), Soulseek and sharing, and how to reach lull from outside the
//! home (Tailscale on this computer, or a reverse proxy or domain).

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use lull_core::process::command;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::{Hub, addresses, print_pair_code, text};
use crate::BoxError;
use crate::managed::tailscale;

pub async fn run(hub: &Hub) -> Result<(), BoxError> {
    let host = hub.get("/api/host").await?;
    let setup = hub.get("/api/setup").await?;
    let now = Settings::read(hub).await?;
    let source = hub.get("/api/library/source").await?;
    let port = u16::try_from(host["port"].as_u64().unwrap_or(8484))?;
    println!(
        "Setting up lull {}. Press Enter to keep what's in [brackets].",
        text(&host["version"])
    );
    let mut changes = Map::new();

    println!();
    println!("Your music");
    // The mode in use: with Jellyfin configured and LULL_LIBRARY unset, that is Jellyfin.
    let jellyfin_now = source["mode"] == "jellyfin";
    let jellyfin = choose(
        "Where is it?",
        &["A folder on this computer", "An existing Jellyfin server"],
        usize::from(jellyfin_now),
    )? == 1;
    let music = if jellyfin {
        let url = ask(
            "  Jellyfin's address",
            now.value("LULL_JELLYFIN_URL").unwrap_or("http://localhost:8096"),
        )?;
        changes.insert("LULL_JELLYFIN_URL".into(), json!(url));
        if let Some(key) = secret(
            "  An API key from Jellyfin (Dashboard, API Keys)",
            now.is_set("LULL_JELLYFIN_API_KEY"),
        )? {
            changes.insert("LULL_JELLYFIN_API_KEY".into(), json!(key));
        }
        let user = ask(
            "  The Jellyfin user lull plays as (none: its first admin)",
            now.value("LULL_JELLYFIN_USER").unwrap_or("none"),
        )?;
        changes.insert("LULL_JELLYFIN_USER".into(), none_is_null(&user));
        ask(
            "  The folder Jellyfin's music library reads, as this computer sees it (saved songs go here)",
            &now.music_dir(hub.data()),
        )?
    } else {
        ask(
            "  The music folder (saved songs go here too)",
            &now.music_dir(hub.data()),
        )?
    };
    changes.insert("LULL_MUSIC_DIR".into(), json!(music));
    let music = PathBuf::from(music);
    music_folder(hub.data(), &music)?;

    println!();
    if setup["soulseekManaged"] == true {
        println!("Soulseek (free: a new name and password make a new account on first login)");
        let current = now.value("LULL_SOULSEEK_USERNAME").unwrap_or("");
        let user = ask("  Username", current)?;
        if !user.is_empty() {
            // The hub takes the account whole, so a new name asks for its password too.
            let keep = user == current && now.is_set("LULL_SOULSEEK_PASSWORD");
            if let Some(password) = secret("  Password", keep)? {
                changes.insert("LULL_SOULSEEK_USERNAME".into(), json!(user));
                changes.insert("LULL_SOULSEEK_PASSWORD".into(), json!(password));
            }
        }
        let share = yes(
            "  Share your music folder with other Soulseek users? Soulseek expects it",
            now.value("LULL_SHARE") != Some("none"),
        )?;
        changes.insert("LULL_SHARE".into(), json!(if share { "library" } else { "none" }));
    } else {
        println!("Soulseek is set up where slskd runs, not here.");
    }

    println!();
    println!("Reaching lull from outside your home");
    if let Some(url) = outside(port, now.value("LULL_PUBLIC_URL")).await? {
        changes.insert("LULL_PUBLIC_URL".into(), url);
    }

    // Only what differs, so the hub restarts its parts only when something changed.
    changes.retain(|k, v| now.raw(k) != Some(&*v) && !(v.is_null() && now.value(k).is_none()));
    let public = changes.get("LULL_PUBLIC_URL").cloned();
    if !changes.is_empty() {
        hub.send(Method::PATCH, "/api/config", Value::Object(changes)).await?;
    }
    // The parts restart to read the change; what follows needs them to have.
    if let Some(url) = &public {
        settled(hub, "/api/host", |h| &h["publicUrl"] == url).await?;
    }
    let music_dir = music.to_string_lossy().into_owned();
    let source = settled(hub, "/api/library/source", |s| {
        s["musicDir"] == music_dir.as_str() && (!jellyfin || !s["jellyfin"].is_null())
    })
    .await?;
    let mode = if jellyfin { "jellyfin" } else { "folder" };
    if source["mode"] != mode {
        hub.send(Method::PUT, "/api/library/source", json!({ "mode": mode }))
            .await?;
    }

    println!();
    println!("Done. lull status shows how each part is doing.");
    addresses(&hub.get("/api/host").await?);
    println!();
    print_pair_code(hub).await
}

/// Waits until `path` answers as `done` says: the hub restarts its parts after a change, and a
/// request made before they have would still see the old ones.
async fn settled(hub: &Hub, path: &str, done: impl Fn(&Value) -> bool) -> Result<Value, BoxError> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < until {
        let answer = hub.get(path).await?;
        if done(&answer) {
            return Ok(answer);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Err("lull didn't take the change within a minute; lull status shows what it's doing".into())
}

/// Tailscale's serve when this computer has Tailscale, else the address of a reverse proxy or
/// the listener's own domain. `None`: nothing to change. `Some(null)`: no outside address.
async fn outside(port: u16, current: Option<&str>) -> Result<Option<Value>, BoxError> {
    let current = current.filter(|u| u.starts_with("https://"));
    let ts = tokio::task::spawn_blocking(move || tailscale::status(port)).await?;
    if ts.serving {
        println!(
            "  Tailscale serves lull at {}",
            ts.url.as_deref().unwrap_or("your tailnet's address")
        );
        return Ok(ts.url.map(Value::from));
    }
    if ts.running {
        let at = ts.url.as_deref().unwrap_or("your tailnet's address");
        if yes(
            &format!("  Tailscale runs on this computer. Serve lull on your tailnet at {at}?"),
            true,
        )? {
            match tokio::task::spawn_blocking(move || tailscale::serve(port)).await? {
                Ok(now) => {
                    println!("  Tailscale serves lull at {}", now.url.as_deref().unwrap_or(at));
                    return Ok(now.url.map(Value::from));
                }
                Err(e) => println!("  {e}"),
            }
        }
    }
    println!("  A reverse proxy or your own domain can serve lull too: proxy it to port {port}.");
    loop {
        let url = ask(
            "  Its https address, if you have one (none: no outside address)",
            current.unwrap_or("none"),
        )?;
        if url == "none" {
            return Ok(current.map(|_| Value::Null));
        }
        if url.starts_with("https://") && !url.contains(char::is_whitespace) {
            return Ok(Some(json!(url.trim_end_matches('/'))));
        }
        println!("  That should start with https:// (browsers install lull only from https).");
    }
}

/// The hub's user must be able to read and write the music folder: a new one is made its own; an
/// existing one it can't use gets an access rule, with the listener's yes.
fn music_folder(data: &Path, dir: &Path) -> Result<(), BoxError> {
    use std::os::unix::fs::MetadataExt;

    let owner = std::fs::metadata(data)?;
    if owner.uid() == 0 {
        return Ok(()); // the hub runs as root: it can use any folder
    }
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
        std::os::unix::fs::chown(dir, Some(owner.uid()), Some(owner.gid()))?;
        println!("  Made {} for lull.", dir.display());
        return Ok(());
    }
    let Some(user) = user_name(owner.uid()) else {
        return Ok(());
    };
    if can_use(&user, dir) {
        return Ok(());
    }
    let mut rules = vec![format!(
        "setfacl -R -m u:{user}:rwX -m d:u:{user}:rwX {}",
        dir.display()
    )];
    // Every folder above it must let the user through, or the rule on the folder alone isn't enough.
    for above in dir.ancestors().skip(1).filter(|a| !passes(&user, a)) {
        rules.push(format!("setfacl -m u:{user}:x {}", above.display()));
    }
    println!("  lull's user ({user}) can't read and write {}.", dir.display());
    if yes("  Let it, with an access rule on the folder?", true)? {
        let applied = rules.iter().all(|rule| {
            let mut words = rule.split(' ');
            let program = words.next().unwrap_or_default();
            command(program)
                .args(words)
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        });
        if applied && can_use(&user, dir) {
            println!("  Done: lull can use {}.", dir.display());
            return Ok(());
        }
    }
    println!("  lull won't be able to use that folder until its user can. As root:");
    for rule in rules {
        println!("    {rule}");
    }
    Ok(())
}

/// Whether `user` can list, read and write `dir`, asked as that user.
fn can_use(user: &str, dir: &Path) -> bool {
    test_as(
        user,
        &[
            "-r",
            &dir.to_string_lossy(),
            "-a",
            "-w",
            &dir.to_string_lossy(),
            "-a",
            "-x",
            &dir.to_string_lossy(),
        ],
    )
}

fn passes(user: &str, dir: &Path) -> bool {
    test_as(user, &["-x", &dir.to_string_lossy()])
}

fn test_as(user: &str, args: &[&str]) -> bool {
    command("runuser")
        .args(["-u", user, "--", "test"])
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// The name of the user with this id, from /etc/passwd.
fn user_name(uid: u32) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        (fields.nth(1)? == uid.to_string()).then(|| name.to_owned())
    })
}

/// The hub's settings as `GET /api/config` gives them.
struct Settings(BTreeMap<String, Value>);

impl Settings {
    async fn read(hub: &Hub) -> Result<Settings, BoxError> {
        let config = hub.get("/api/config").await?;
        Ok(Settings(
            config["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| (text(&i["key"]), i.clone()))
                .collect(),
        ))
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.0.get(key)?["value"].as_str()
    }

    fn raw(&self, key: &str) -> Option<&Value> {
        self.0.get(key).map(|i| &i["value"]).filter(|v| !v.is_null())
    }

    fn is_set(&self, key: &str) -> bool {
        self.0.get(key).is_some_and(|i| i["set"] == true)
    }

    /// The music folder now, or one in the data folder when the default isn't a folder here.
    fn music_dir(&self, data: &Path) -> String {
        match self.value("LULL_MUSIC_DIR") {
            Some(dir) => dir.to_owned(),
            None => data.join("music").to_string_lossy().into_owned(),
        }
    }
}

fn none_is_null(answer: &str) -> Value {
    if answer == "none" { Value::Null } else { json!(answer) }
}

/// One line of input, trimmed. The end of input is an error, so a pipe that runs dry stops.
fn line() -> Result<String, BoxError> {
    std::io::stdout().flush()?;
    let mut buf = String::new();
    if std::io::stdin().lock().read_line(&mut buf)? == 0 {
        return Err("setup stopped: no more input".into());
    }
    Ok(buf.trim().to_owned())
}

fn ask(question: &str, default: &str) -> Result<String, BoxError> {
    if default.is_empty() {
        print!("{question}: ");
    } else {
        print!("{question} [{default}]: ");
    }
    let answer = line()?;
    Ok(if answer.is_empty() { default.to_owned() } else { answer })
}

fn yes(question: &str, default: bool) -> Result<bool, BoxError> {
    loop {
        print!("{question} [{}]: ", if default { "Y/n" } else { "y/N" });
        match line()?.to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("  Please answer y or n."),
        }
    }
}

/// One of `options`, numbered from 1; returns its index.
fn choose(question: &str, options: &[&str], default: usize) -> Result<usize, BoxError> {
    println!("{question}");
    for (i, option) in options.iter().enumerate() {
        println!("  {}) {option}", i + 1);
    }
    loop {
        let answer = ask("Choose", &(default + 1).to_string())?;
        if let Some(i) = answer.parse::<usize>().ok().filter(|i| (1..=options.len()).contains(i)) {
            return Ok(i - 1);
        }
        println!("  Please choose 1 to {}.", options.len());
    }
}

/// A secret, typed without showing it. `None`: kept as it is (Enter, when one is set).
fn secret(question: &str, is_set: bool) -> Result<Option<String>, BoxError> {
    loop {
        print!("{question}{}: ", if is_set { " [keep]" } else { "" });
        let hidden = echo(false);
        let answer = line();
        if hidden {
            echo(true);
            println!();
        }
        let answer = answer?;
        if !answer.is_empty() {
            return Ok(Some(answer));
        }
        if is_set {
            return Ok(None);
        }
        println!("  It can't be empty.");
    }
}

/// Turns the terminal's echo off or on; false when there is no terminal (input from a pipe).
fn echo(on: bool) -> bool {
    command("stty")
        .arg(if on { "echo" } else { "-echo" })
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
