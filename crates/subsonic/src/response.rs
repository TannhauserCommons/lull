//! The `subsonic-response` envelope in JSON or XML (`f=`), and Subsonic's error codes.
//! Bodies are built once as JSON; XML is derived from it by the convention every Subsonic
//! server follows: scalars become attributes, objects child elements, arrays repeated
//! elements, and a `value` key the element's text.

use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

/// The Subsonic API version lull speaks, plus OpenSubsonic.
pub const VERSION: &str = "1.16.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Xml,
    Json,
}

impl Format {
    /// Subsonic's default is XML. JSONP is not offered; asking for it answers in XML.
    pub fn of(f: Option<&str>) -> Self {
        match f {
            Some("json") => Format::Json,
            _ => Format::Xml,
        }
    }
}

/// A failed call: a Subsonic error code and a sentence for the listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: u16,
    pub message: String,
}

impl Fail {
    pub fn generic(message: impl Into<String>) -> Self {
        Fail {
            code: 0,
            message: message.into(),
        }
    }
    pub fn missing(param: &str) -> Self {
        Fail {
            code: 10,
            message: format!("required parameter is missing: {param}"),
        }
    }
    pub fn wrong_credentials() -> Self {
        Fail {
            code: 40,
            message: "wrong username or password".into(),
        }
    }
    pub fn conflicting_auth() -> Self {
        Fail {
            code: 43,
            message: "multiple conflicting authentication mechanisms provided".into(),
        }
    }
    pub fn invalid_api_key() -> Self {
        Fail {
            code: 44,
            message: "invalid API key".into(),
        }
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Fail {
            code: 70,
            message: what.into(),
        }
    }
}

impl From<lull_core::Error> for Fail {
    fn from(e: lull_core::Error) -> Self {
        match e {
            lull_core::Error::NotFound(what) => Fail::not_found(what),
            e => Fail::generic(e.to_string()),
        }
    }
}

pub fn ok(format: Format, body: Map<String, Value>) -> Response {
    render(format, "ok", body)
}

pub fn fail(format: Format, fail: Fail) -> Response {
    let mut body = Map::new();
    body.insert("error".into(), json!({ "code": fail.code, "message": fail.message }));
    render(format, "failed", body)
}

fn render(format: Format, status: &str, mut body: Map<String, Value>) -> Response {
    body.insert("status".into(), status.into());
    body.insert("version".into(), VERSION.into());
    body.insert("type".into(), "lull".into());
    body.insert("serverVersion".into(), env!("CARGO_PKG_VERSION").into());
    body.insert("openSubsonic".into(), true.into());
    let body = without_nulls(Value::Object(body));
    match format {
        Format::Json => axum::Json(json!({ "subsonic-response": body })).into_response(),
        Format::Xml => {
            let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
            if let Value::Object(mut map) = body {
                map.insert("xmlns".into(), "http://subsonic.org/restapi".into());
                element(&mut xml, "subsonic-response", &Value::Object(map));
            }
            ([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], xml).into_response()
        }
    }
}

/// Unknown facts are left out rather than sent as `null`, which typed clients reject.
fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, without_nulls(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_nulls).collect()),
        v => v,
    }
}

fn element(out: &mut String, name: &str, value: &Value) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| element(out, name, item)),
        Value::Object(map) => {
            out.push('<');
            out.push_str(name);
            for (key, v) in map.iter().filter(|(k, v)| *k != "value" && is_scalar(v)) {
                out.push_str(&format!(r#" {key}="{}""#, escape(&scalar(v))));
            }
            let text = map.get("value").filter(|v| is_scalar(v));
            let children: Vec<_> = map.iter().filter(|(_, v)| !is_scalar(v)).collect();
            if text.is_none() && children.is_empty() {
                out.push_str("/>");
                return;
            }
            out.push('>');
            if let Some(text) = text {
                out.push_str(&escape(&scalar(text)));
            }
            for (key, child) in children {
                element(out, key, child);
            }
            out.push_str(&format!("</{name}>"));
        }
        v => out.push_str(&format!("<{name}>{}</{name}>", escape(&scalar(v)))),
    }
}

fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Object(_) | Value::Array(_))
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}
