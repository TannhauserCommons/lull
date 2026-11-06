//! One-step remote access (ADR-0002, ADR-0090): where the hub's computer has Tailscale,
//! `tailscale serve --bg <port>` gives the hub an https address on the listener's tailnet, which
//! phones need to install the web app. Nothing changes until the listener asks for it (the
//! desktop app's Settings button, `lull setup`); only Tailscale's state is read before that.

use lull_core::process::command;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `tailscale serve` waits for the listener when HTTPS is off in their tailnet; give up after this.
const SERVE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    /// Logged in and connected.
    pub running: bool,
    /// `https://<machine>.<tailnet>.ts.net`, once this machine has a tailnet name.
    pub url: Option<String>,
    /// `tailscale serve` already proxies that address to the hub.
    pub serving: bool,
}

pub fn status(port: u16) -> Status {
    let Some(cli) = cli() else {
        return Status {
            installed: false,
            running: false,
            url: None,
            serving: false,
        };
    };
    let me = json::<SelfStatus>(&cli, &["status", "--json"]);
    let running = me.as_ref().is_some_and(|s| s.backend_state == "Running");
    let url = me
        .and_then(|s| s.me)
        .map(|m| m.dns_name.trim_end_matches('.').to_owned())
        .filter(|name| !name.is_empty())
        .map(|name| format!("https://{name}"));
    let serving = running && serves(&cli, port);
    Status {
        installed: true,
        running,
        url,
        serving,
    }
}

/// Runs `tailscale serve --bg <port>` and reports the result.
pub fn serve(port: u16) -> Result<Status, String> {
    let cli = cli().ok_or("Tailscale isn't installed on this computer")?;
    let output = run_with_timeout(command(&cli).args(["serve", "--bg", &port.to_string()]))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = if said.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            said
        };
        return Err(format!("Tailscale said: {}", said.trim()));
    }
    let now = status(port);
    if now.serving {
        Ok(now)
    } else {
        Err("Tailscale accepted the command but doesn't serve the hub".into())
    }
}

/// The CLI on PATH, or where the Windows installer puts it.
fn cli() -> Option<PathBuf> {
    let name = format!("tailscale{}", std::env::consts::EXE_SUFFIX);
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(&name));
    let installed = cfg!(windows).then(|| PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe"));
    on_path.chain(installed).find(|p| p.is_file())
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SelfStatus {
    backend_state: String,
    #[serde(rename = "Self")]
    me: Option<Peer>,
}

#[derive(Deserialize)]
struct Peer {
    #[serde(rename = "DNSName")]
    dns_name: String,
}

/// `tailscale serve status --json`: `{ Web: { "<host>:443": { Handlers: { "/": { Proxy } } } } }`.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ServeStatus {
    #[serde(default)]
    web: std::collections::BTreeMap<String, Web>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Web {
    #[serde(default)]
    handlers: std::collections::BTreeMap<String, Handler>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Handler {
    #[serde(default)]
    proxy: String,
}

fn serves(cli: &PathBuf, port: u16) -> bool {
    let Some(serve) = json::<ServeStatus>(cli, &["serve", "status", "--json"]) else {
        return false;
    };
    let targets = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    serve.web.values().flat_map(|w| w.handlers.get("/")).any(|h| {
        let proxy = h.proxy.trim_end_matches('/');
        targets.iter().any(|t| proxy.ends_with(t.as_str())) || proxy == port.to_string()
    })
}

fn json<T: for<'de> Deserialize<'de>>(cli: &PathBuf, args: &[&str]) -> Option<T> {
    let output = run_with_timeout(command(cli).args(args)).ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn run_with_timeout(cmd: &mut Command) -> Result<Output, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't run tailscale: {e}"))?;
    // Read both pipes while waiting, so a chatty command can't fill one and stall.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + SERVE_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Tailscale is waiting for something; HTTPS may be off for your tailnet. Turn it on in the Tailscale admin console (DNS, HTTPS certificates) and try again.".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}
