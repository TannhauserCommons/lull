//! What Classic's own player tells the hub, the calls the web UI makes for it otherwise
//! (web/app.js): a play when a song starts (`POST /api/plays`) and how it ended (`PATCH`), Like
//! (`PUT`/`DELETE /api/likes/{id}`) and Save (`POST /api/saves`). A hub out of reach doesn't lose
//! them: plays and likes become the outbox's ops (web/offline.js), handed to the page with the
//! queue when Classic closes.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::time::Duration;
use tauri::Url;
use ureq::tls::{RootCerts, TlsConfig};

/// How a call failed.
#[derive(Debug)]
pub enum Failure {
    /// No answer: the call can wait in the outbox.
    Offline,
    /// The hub answered no; its sentence says why.
    Refused(String),
}

#[derive(Clone)]
pub struct Hub {
    agent: ureq::Agent,
    origin: Url,
    token: String,
}

impl Hub {
    pub fn new(origin: Url, token: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .input_buffer_size(super::remote::INPUT_BUFFER)
            .output_buffer_size(super::remote::OUTPUT_BUFFER)
            .http_status_as_error(false)
            .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
            .user_agent(concat!("lull-app/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent, origin, token }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn origin(&self) -> &Url {
        &self.origin
    }

    /// A song started: the play's id, or `None` when this device listens privately (nothing kept).
    pub fn start_play(&self, track: &str, duration: Option<f64>, recommended: bool) -> Result<Option<String>, Failure> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Started {
            play_id: Option<String>,
        }
        let body = super::rules::play_start(track, duration, recommended);
        let answer = self.call("POST", "/api/plays", Some(body))?;
        let started: Started = serde_json::from_slice(&answer).map_err(|e| Failure::Refused(e.to_string()))?;
        Ok(started.play_id)
    }

    pub fn end_play(&self, play: &str, end: &Value) -> Result<(), Failure> {
        let path = format!("/api/plays/{}", encode(play));
        self.call("PATCH", &path, Some(end.clone())).map(drop)
    }

    /// A file on the hub (a cover), to read.
    pub fn get(&self, url: &str) -> Result<impl Read + use<>, String> {
        let response = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("the hub said {}", response.status()));
        }
        Ok(response.into_body().into_reader())
    }

    pub fn like(&self, track: &str, on: bool) -> Result<(), Failure> {
        let path = format!("/api/likes/{}", encode(track));
        self.call(if on { "PUT" } else { "DELETE" }, &path, None).map(drop)
    }

    pub fn save(&self, track: &str) -> Result<(), Failure> {
        self.call("POST", "/api/saves", Some(json!({ "ids": [track] })))
            .map(drop)
    }

    /// Any other call the player makes (settings, radio, prefetch, letting a stream go, a better
    /// copy's switch, lull Connect's state): the answer as JSON, `null` when there is none.
    pub fn json(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Failure> {
        let answer = self.call(method, path, body)?;
        if answer.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&answer).map_err(|e| Failure::Refused(e.to_string()))
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Vec<u8>, Failure> {
        let url = self.origin.join(path).map_err(|e| Failure::Refused(e.to_string()))?;
        let request = ureq::http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .header("Authorization", format!("Bearer {}", self.token));
        let sent = match body {
            Some(body) => self.agent.run(
                request
                    .header("Content-Type", "application/json")
                    .body(body.to_string())
                    .map_err(|e| Failure::Refused(e.to_string()))?,
            ),
            None => self
                .agent
                .run(request.body(()).map_err(|e| Failure::Refused(e.to_string()))?),
        };
        let response = sent.map_err(|_| Failure::Offline)?;
        let status = response.status();
        let mut answer = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(1024 * 1024)
            .read_to_end(&mut answer)
            .map_err(|_| Failure::Offline)?;
        if status.is_success() {
            return Ok(answer);
        }
        let said = serde_json::from_slice::<Value>(&answer)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("the hub said {status}"));
        Err(Failure::Refused(said))
    }
}

fn encode(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
