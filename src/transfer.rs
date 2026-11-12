//! The registry file that `vuta export` writes and `vuta import` reads.
//!
//! It carries what defines the registry on one machine: roots, repositories
//! with their tags and hooks, and settings that differ from the defaults.
//! It leaves out what belongs to one machine's history: runs, results and
//! the cached last revision.
//!
//! ```json
//! {
//!   "format": "vuta-registry",
//!   "version": 1,
//!   "roots": [{ "name": "src", "path": "/home/me/src", "default": true }],
//!   "settings": { "sync.jobs": "8" },
//!   "repos": [{
//!     "name": "v", "root": "src", "path": "v/v",
//!     "url": "https://github.com/vlang/v", "branch": null,
//!     "clone_filter": null, "clone_depth": null, "enabled": true,
//!     "tags": ["v"],
//!     "hooks": [{ "command": "make", "platform": "unix", "when": "changed", "enabled": true }]
//!   }]
//! }
//! ```

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// The value of `format`, which marks a file as a Vuta registry.
pub const FORMAT: &str = "vuta-registry";
/// The newest file version this Vuta reads and the one it writes.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub format: String,
    pub version: u32,
    /// The Vuta that wrote the file. Informational.
    #[serde(default)]
    pub vuta_version: Option<String>,
    /// When the file was written, in Unix seconds. Informational.
    #[serde(default)]
    pub exported_at: Option<i64>,
    #[serde(default)]
    pub roots: Vec<RootRecord>,
    /// Settings that differ from their defaults.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default)]
    pub repos: Vec<RepoRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecord {
    pub name: String,
    /// Absolute, on the machine that exported it.
    pub path: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRecord {
    pub name: String,
    /// The name of one of the file's roots.
    pub root: String,
    /// Relative to the root, with `/` separators.
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub clone_filter: Option<String>,
    #[serde(default)]
    pub clone_depth: Option<u32>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<HookRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRecord {
    pub command: String,
    /// `any`, `unix` or `windows`.
    #[serde(default = "any")]
    pub platform: String,
    /// `changed` or `always`.
    #[serde(default = "changed")]
    pub when: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

fn any() -> String {
    "any".to_owned()
}

fn changed() -> String {
    "changed".to_owned()
}

impl Registry {
    /// The file's text: pretty JSON with a final newline.
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("a registry always serializes");
        text.push('\n');
        text
    }

    /// Read a registry file. Accepts UTF-8 with or without a byte-order
    /// mark, and UTF-16 with one, which is what Windows PowerShell's `>`
    /// writes. Refuses files of another format or a newer version.
    pub fn parse(bytes: &[u8]) -> Result<Registry> {
        let text = decode(bytes)?;
        let value: serde_json::Value =
            serde_json::from_str(&text).context("the file is not valid JSON")?;
        match value.get("format").and_then(|f| f.as_str()) {
            Some(FORMAT) => {}
            _ => bail!("the file is not a Vuta registry; make one with: vuta export"),
        }
        match value.get("version").and_then(|v| v.as_u64()) {
            Some(v) if v <= u64::from(VERSION) && v > 0 => {}
            Some(v) => bail!(
                "the file is registry version {v}, but this vuta reads up to version {VERSION}; \
                 upgrade vuta"
            ),
            None => bail!("the file has no registry version"),
        }
        serde_json::from_value(value).context("the registry file is malformed")
    }
}

fn decode(bytes: &[u8]) -> Result<String> {
    let utf16 = |bytes: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| from(pair))
            .collect();
        String::from_utf16(&units).context("the file is not valid UTF-16")
    };
    match bytes {
        [0xef, 0xbb, 0xbf, rest @ ..] => std::str::from_utf8(rest)
            .map(str::to_owned)
            .context("the file is not valid UTF-8"),
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .context("the file is not valid UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "format": "vuta-registry", "version": 1,
        "roots": [{ "name": "src", "path": "/src" }],
        "repos": [{ "name": "v", "root": "src", "path": "v", "url": "u",
                    "hooks": [{ "command": "make" }] }]
    }"#;

    #[test]
    fn missing_fields_take_defaults() {
        let registry = Registry::parse(MINIMAL.as_bytes()).unwrap();
        let repo = &registry.repos[0];
        assert!(repo.enabled && repo.tags.is_empty() && repo.branch.is_none());
        let hook = &repo.hooks[0];
        assert_eq!(
            (hook.platform.as_str(), hook.when.as_str()),
            ("any", "changed")
        );
        assert!(hook.enabled);
        assert!(!registry.roots[0].default);
    }

    #[test]
    fn round_trips() {
        let registry = Registry::parse(MINIMAL.as_bytes()).unwrap();
        let again = Registry::parse(registry.to_json().as_bytes()).unwrap();
        assert_eq!(again, registry);
    }

    #[test]
    fn reads_boms_and_utf16() {
        let mut with_bom = vec![0xef, 0xbb, 0xbf];
        with_bom.extend(MINIMAL.as_bytes());
        assert!(Registry::parse(&with_bom).is_ok());

        let mut utf16le = vec![0xff, 0xfe];
        utf16le.extend(MINIMAL.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(
            Registry::parse(&utf16le).unwrap(),
            Registry::parse(MINIMAL.as_bytes()).unwrap()
        );
    }

    #[test]
    fn refuses_other_files() {
        let err = |text: &str| Registry::parse(text.as_bytes()).unwrap_err().to_string();
        assert!(err("not json").contains("not valid JSON"));
        assert!(err(r#"{"name": "x"}"#).contains("not a Vuta registry"));
        assert!(err(r#"{"format": "vuta-registry", "version": 2}"#).contains("upgrade vuta"));
        assert!(
            err(r#"{"format": "vuta-registry", "version": 1, "repos": [{}]}"#)
                .contains("malformed")
        );
    }
}
