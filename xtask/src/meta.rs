//! What every package task needs: the package metadata from the root
//! `Cargo.toml`, the target triple, the release build, and running tools.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::Common;

/// The fields of the root `[package]` table that packages carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub name: String,
    pub version: Version,
    pub description: String,
    pub license: String,
    pub repository: String,
    /// The `Copyright ...` line of LICENSE-MIT, without the prefix.
    pub copyright: String,
}

/// A Cargo version: `major.minor.patch` with an optional pre-release tag.
/// Build metadata (`+...`) is not used by Vuta and is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(text: &str) -> Result<Version> {
        if text.contains('+') {
            bail!("version {text}: build metadata is not supported");
        }
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_owned())),
            None => (text, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        let [major, minor, patch] = parts[..] else {
            bail!("version {text} is not major.minor.patch");
        };
        let number = |s: &str| {
            s.parse::<u64>()
                .with_context(|| format!("version {text}: '{s}' is not a number"))
        };
        Ok(Version {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
            pre,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// The repository root: the parent of this crate's directory.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the repository")
        .to_path_buf()
}

/// Cargo's target directory, honouring `CARGO_TARGET_DIR`.
pub fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => root().join(dir),
        None => root().join("target"),
    }
}

/// Where packages are written.
pub fn out_dir(common: &Common) -> Result<PathBuf> {
    let dir = match &common.out_dir {
        Some(dir) => std::path::absolute(dir)?,
        None => target_dir().join("package"),
    };
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

impl Meta {
    pub fn load() -> Result<Meta> {
        let root = root();
        let manifest =
            fs::read_to_string(root.join("Cargo.toml")).context("cannot read Cargo.toml")?;
        let license =
            fs::read_to_string(root.join("LICENSE-MIT")).context("cannot read LICENSE-MIT")?;
        Meta::parse(&manifest, &license)
    }

    /// Read the `[package]` table's plain `key = "value"` lines. That is
    /// all the root manifest uses, so no TOML parser is needed.
    fn parse(manifest: &str, mit_license: &str) -> Result<Meta> {
        let get = |key: &str| {
            package_field(manifest, key)
                .with_context(|| format!("Cargo.toml [package] has no {key}"))
        };
        let name = get("name")?;
        let version = Version::parse(&get("version")?)?;
        let description = get("description")?;
        let license = get("license")?;
        let repository = get("repository")?;
        let copyright = mit_license
            .lines()
            .find_map(|l| l.trim().strip_prefix("Copyright"))
            .map(|rest| rest.trim().trim_start_matches("(c)").trim().to_owned())
            .context("LICENSE-MIT has no Copyright line")?;
        Ok(Meta {
            name,
            version,
            description,
            license,
            repository,
            copyright,
        })
    }
}

/// A `key = "value"` line of the `[package]` table.
fn package_field(manifest: &str, key: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            let v = v.trim();
            return v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_owned);
        }
    }
    None
}

/// The host's target triple, from `rustc -vV`.
pub fn host_triple() -> Result<String> {
    let out = capture(Command::new("rustc").arg("-vV"))?;
    out.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc -vV printed no host line")
}

/// The triple to package: `--target`, else the host.
pub fn triple(common: &Common) -> Result<String> {
    match &common.target {
        Some(t) => Ok(t.clone()),
        None => host_triple(),
    }
}

/// Build the release binary for `triple` (unless `--no-build`) and return
/// its path. The build always names the target, so the binary is at
/// `target/<triple>/release/`.
pub fn release_binary(common: &Common, triple: &str, file_name: &str) -> Result<PathBuf> {
    if !common.no_build {
        run(
            Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
                .current_dir(root())
                .args([
                    "build",
                    "--release",
                    "--locked",
                    "--package",
                    "vuta",
                    "--target",
                    triple,
                ]),
        )?;
    }
    let binary = target_dir().join(triple).join("release").join(file_name);
    if !binary.is_file() {
        bail!(
            "{} does not exist; build it, or drop --no-build",
            binary.display()
        );
    }
    Ok(binary)
}

/// Run a command, inheriting the terminal, and fail on a non-zero exit.
pub fn run(cmd: &mut Command) -> Result<()> {
    let line = describe(cmd);
    eprintln!("$ {line}");
    let status = cmd
        .status()
        .with_context(|| format!("cannot run {}", cmd.get_program().to_string_lossy()))?;
    if !status.success() {
        bail!("{line} failed ({status})");
    }
    Ok(())
}

/// Run a command and return its stdout; fail on a non-zero exit.
pub fn capture(cmd: &mut Command) -> Result<String> {
    let line = describe(cmd);
    let out = cmd
        .output()
        .with_context(|| format!("cannot run {}", cmd.get_program().to_string_lossy()))?;
    if !out.status.success() {
        bail!(
            "{line} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn describe(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Remove and recreate a staging directory.
pub fn fresh_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir).with_context(|| format!("cannot remove {}", dir.display()))?;
    }
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!((v.major, v.minor, v.patch, v.pre), (1, 2, 3, None));
        let v = Version::parse("0.2.0-beta.1").unwrap();
        assert_eq!(v.pre.as_deref(), Some("beta.1"));
        assert_eq!(v.to_string(), "0.2.0-beta.1");
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.x").is_err());
        assert!(Version::parse("1.2.3+build").is_err());
    }

    #[test]
    fn reads_the_package_table() {
        let manifest = r#"
[package]
name = "vuta"
version = "0.1.0"
description = "Keep checkouts fresh"
license = "MIT OR Apache-2.0"
repository = "https://example.com/vuta"

[workspace]
members = ["xtask"]

[dependencies]
version = "not this one"
"#;
        let license = "MIT License\n\nCopyright (c) 2026 Someone\n\nPermission...";
        let meta = Meta::parse(manifest, license).unwrap();
        assert_eq!(meta.name, "vuta");
        assert_eq!(meta.version.to_string(), "0.1.0");
        assert_eq!(meta.description, "Keep checkouts fresh");
        assert_eq!(meta.copyright, "2026 Someone");
        assert!(Meta::parse("[package]\nname = \"x\"", license).is_err());
    }

    #[test]
    fn the_real_manifest_parses() {
        let meta = Meta::load().unwrap();
        assert_eq!(meta.name, "vuta");
    }
}
