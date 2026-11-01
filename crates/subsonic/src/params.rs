//! A request's parameters, from the query string and, for a form POST (OpenSubsonic
//! `formPost`), the body. Kept as pairs: Subsonic repeats keys (`id=a&id=b`).

use std::str::FromStr;

use crate::response::Fail;

pub struct Params(Vec<(String, String)>);

impl Params {
    pub fn parse(query: Option<&str>, form: &[u8]) -> Self {
        let query = form_urlencoded::parse(query.unwrap_or_default().as_bytes());
        Params(
            query
                .chain(form_urlencoded::parse(form))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.all(key).next()
    }

    pub fn all<'a, 'k>(&'a self, key: &'k str) -> impl Iterator<Item = &'a str> + use<'a, 'k> {
        self.0.iter().filter(move |(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn required(&self, key: &str) -> Result<&str, Fail> {
        self.get(key).ok_or_else(|| Fail::missing(key))
    }

    /// A numeric parameter, `default` when absent; anything unparsable is the client's mistake.
    pub fn number<T: FromStr>(&self, key: &str, default: T) -> Result<T, Fail> {
        self.optional_number(key).map(|n| n.unwrap_or(default))
    }

    pub fn optional_number<T: FromStr>(&self, key: &str) -> Result<Option<T>, Fail> {
        self.get(key)
            .map(|v| {
                v.parse()
                    .map_err(|_| Fail::generic(format!("{key} must be a number, not {v:?}")))
            })
            .transpose()
    }

    /// `count` and `offset` style paging with the spec's cap.
    pub fn page(&self, count: &str, offset: &str, default: usize, max: usize) -> Result<(usize, usize), Fail> {
        Ok((self.number(count, default)?.min(max), self.number(offset, 0)?))
    }
}
