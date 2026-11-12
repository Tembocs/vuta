//! `cargo xtask deb`: a Debian package built with `dpkg-deb`.
//!
//! The package installs `/usr/bin/vuta` and `/usr/share/doc/vuta/copyright`,
//! depends on `git` and on the shared libraries `dpkg-shlibdeps` finds, and
//! is owned by root whoever builds it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::Common;
use crate::meta::{self, Meta, Version};

#[derive(Debug, Args)]
pub struct DebArgs {
    #[command(flatten)]
    common: Common,
    /// Maintainer field, "Name <email>" [default: DEBFULLNAME and DEBEMAIL,
    /// then git's user.name and user.email]
    #[arg(long, value_name = "NAME <EMAIL>")]
    maintainer: Option<String>,
    /// Debian revision, the part after the upstream version
    #[arg(long, default_value_t = 1, value_name = "N")]
    revision: u32,
}

/// The long description under the control file's Description line.
const LONG_DESCRIPTION: &str = "\
Vuta keeps a set of Git checkouts cloned and up to date. One SQLite
database holds the registry of repositories, their tags, post-sync hooks
and run history. Updates are fast-forward only: a checkout with local
changes, diverged commits or a detached HEAD is skipped and left alone.
Repositories sync in parallel and each gets one coloured result line.";

pub fn run(args: &DebArgs) -> Result<PathBuf> {
    if !cfg!(target_os = "linux") {
        bail!("Debian packages are built on Linux, with dpkg-deb");
    }
    let meta = Meta::load()?;
    let triple = meta::triple(&args.common)?;
    let arch = deb_arch(&triple)?;
    let version = deb_version(&meta.version, args.revision);
    let maintainer = maintainer(args.maintainer.as_deref())?;
    let binary = meta::release_binary(&args.common, &triple, &meta.name)?;

    let full_name = format!("{}_{version}_{arch}", meta.name);
    let staging = meta::target_dir()
        .join("package")
        .join("deb")
        .join(&full_name);
    meta::fresh_dir(&staging)?;

    let bin = staging.join("usr/bin").join(&meta.name);
    install(&binary, &bin, 0o755)?;
    let doc = staging.join("usr/share/doc").join(&meta.name);
    write(&doc.join("copyright"), &copyright(&meta)?, 0o644)?;

    let depends = depends(&bin);
    let control = control(&Control {
        meta: &meta,
        version: &version,
        arch,
        maintainer: &maintainer,
        installed_kib: installed_size_kib(&staging)?,
        depends: &depends,
    });
    write(&staging.join("DEBIAN/control"), &control, 0o644)?;
    set_dir_modes(&staging)?;

    let package = meta::out_dir(&args.common)?.join(format!("{full_name}.deb"));
    meta::run(
        Command::new("dpkg-deb")
            .args(["--root-owner-group", "-Zxz", "--build"])
            .arg(&staging)
            .arg(&package),
    )?;
    Ok(package)
}

/// Debian's architecture name for a Rust target triple.
fn deb_arch(triple: &str) -> Result<&'static str> {
    if !triple.contains("-linux-") {
        bail!("{triple} is not a Linux target");
    }
    let arch = triple.split('-').next().unwrap_or("");
    Ok(match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "i686" | "i586" => "i386",
        "armv7" if triple.ends_with("eabihf") => "armhf",
        "riscv64gc" => "riscv64",
        "powerpc64le" => "ppc64el",
        "s390x" => "s390x",
        "loongarch64" => "loong64",
        _ => bail!("no Debian architecture for {triple}"),
    })
}

/// `0.1.0` becomes `0.1.0-1`. A pre-release such as `0.2.0-beta.1`
/// becomes `0.2.0~beta.1-1`, since `~` sorts before the final release.
fn deb_version(version: &Version, revision: u32) -> String {
    let upstream = format!("{}.{}.{}", version.major, version.minor, version.patch);
    match &version.pre {
        Some(pre) => format!("{upstream}~{pre}-{revision}"),
        None => format!("{upstream}-{revision}"),
    }
}

/// The Maintainer field: the flag, then DEBFULLNAME/DEBEMAIL, then git.
fn maintainer(flag: Option<&str>) -> Result<String> {
    if let Some(m) = flag {
        return check_maintainer(m.to_owned());
    }
    let env = |k| {
        std::env::var(k)
            .ok()
            .filter(|v: &String| !v.trim().is_empty())
    };
    if let Some(email) = env("DEBEMAIL") {
        if email.contains('<') {
            return check_maintainer(email);
        }
        if let Some(name) = env("DEBFULLNAME") {
            return check_maintainer(format!("{name} <{email}>"));
        }
    }
    let git = |key| {
        meta::capture(Command::new("git").args(["config", "--get", key]))
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    match (git("user.name"), git("user.email")) {
        (Some(name), Some(email)) => check_maintainer(format!("{name} <{email}>")),
        _ => bail!(
            "no maintainer: pass --maintainer \"Name <email>\", or set DEBFULLNAME and DEBEMAIL"
        ),
    }
}

fn check_maintainer(m: String) -> Result<String> {
    let ok = m.contains('<') && m.trim_end().ends_with('>') && !m.contains('\n');
    if !ok {
        bail!("maintainer '{m}' must look like: Name <email>");
    }
    Ok(m.trim().to_owned())
}

/// Shared-library dependencies from `dpkg-shlibdeps`, plus `git`. Falls
/// back to plain `libc6` when the tool is missing or cannot read the
/// binary (for example when cross-building).
fn depends(binary: &Path) -> String {
    match shlibdeps(binary) {
        Ok(Some(libs)) => format!("git, {libs}"),
        Ok(None) => "git".to_owned(),
        Err(err) => {
            eprintln!("warning: dpkg-shlibdeps did not run ({err:#}); depending on libc6 only");
            "git, libc6".to_owned()
        }
    }
}

fn shlibdeps(binary: &Path) -> Result<Option<String>> {
    // dpkg-shlibdeps insists on a debian/control in its working directory.
    let work = meta::target_dir()
        .join("package")
        .join("deb")
        .join("shlibdeps");
    meta::fresh_dir(&work.join("debian"))?;
    fs::write(
        work.join("debian/control"),
        "Source: vuta\n\nPackage: vuta\nArchitecture: any\n",
    )?;
    let out = meta::capture(
        Command::new("dpkg-shlibdeps")
            .current_dir(&work)
            .arg("-O")
            .arg(binary),
    )?;
    let libs = out
        .lines()
        .find_map(|l| l.strip_prefix("shlibs:Depends="))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned);
    Ok(libs)
}

struct Control<'a> {
    meta: &'a Meta,
    version: &'a str,
    arch: &'a str,
    maintainer: &'a str,
    installed_kib: u64,
    depends: &'a str,
}

fn control(c: &Control) -> String {
    let mut text = format!(
        "Package: {}\nVersion: {}\nArchitecture: {}\nMaintainer: {}\nInstalled-Size: {}\n\
         Depends: {}\nSection: vcs\nPriority: optional\nHomepage: {}\nDescription: {}\n",
        c.meta.name,
        c.version,
        c.arch,
        c.maintainer,
        c.installed_kib,
        c.depends,
        c.meta.repository,
        c.meta.description,
    );
    text.push_str(&indent_paragraph(LONG_DESCRIPTION));
    text
}

/// Continuation lines of a control-file field: each line indented by one
/// space, and blank lines written as ` .`.
fn indent_paragraph(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.trim().is_empty() {
                " .\n".to_owned()
            } else {
                format!(" {l}\n")
            }
        })
        .collect()
}

/// A machine-readable copyright file (DEP-5). The MIT text is copied from
/// LICENSE-MIT; Debian ships the Apache text in common-licenses.
fn copyright(meta: &Meta) -> Result<String> {
    let mit = fs::read_to_string(meta::root().join("LICENSE-MIT"))?;
    let mit_body: String = mit
        .lines()
        .skip_while(|l| !l.starts_with("Permission"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!(
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n\
         Upstream-Name: {name}\n\
         Source: {repo}\n\
         \n\
         Files: *\n\
         Copyright: {copyright}\n\
         License: {license}\n\
         \n\
         License: MIT\n\
         {mit}\
         \n\
         License: Apache-2.0\n \
         On Debian systems, the full text of the Apache License, Version 2.0\n \
         can be found in /usr/share/common-licenses/Apache-2.0.\n",
        name = meta.name,
        repo = meta.repository,
        copyright = meta.copyright,
        license = meta.license.replace(" OR ", " or "),
        mit = indent_paragraph(&mit_body),
    ))
}

/// Installed-Size in KiB, as dpkg-gencontrol counts it: each file rounded
/// up to a whole KiB, and one KiB per directory.
fn installed_size_kib(staging: &Path) -> Result<u64> {
    fn walk(dir: &Path, total: &mut u64) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == "DEBIAN") {
                continue;
            }
            let meta = entry.metadata()?;
            if meta.is_dir() {
                *total += 1;
                walk(&path, total)?;
            } else {
                *total += meta.len().div_ceil(1024);
            }
        }
        Ok(())
    }
    let mut total = 0;
    walk(staging, &mut total)?;
    Ok(total)
}

fn install(from: &Path, to: &Path, mode: u32) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to).with_context(|| format!("cannot copy {}", from.display()))?;
    set_mode(to, mode)
}

fn write(path: &Path, text: &str, mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
    set_mode(path, mode)
}

/// Every directory 0755, whatever the umask.
fn set_dir_modes(dir: &Path) -> Result<()> {
    set_mode(dir, 0o755)?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            set_dir_modes(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("cannot set the mode of {}", path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectures() {
        assert_eq!(deb_arch("x86_64-unknown-linux-gnu").unwrap(), "amd64");
        assert_eq!(deb_arch("aarch64-unknown-linux-gnu").unwrap(), "arm64");
        assert_eq!(deb_arch("armv7-unknown-linux-gnueabihf").unwrap(), "armhf");
        assert_eq!(deb_arch("x86_64-unknown-linux-musl").unwrap(), "amd64");
        assert!(deb_arch("x86_64-pc-windows-msvc").is_err());
        assert!(deb_arch("sparc64-unknown-linux-gnu").is_err());
    }

    #[test]
    fn versions() {
        let v = Version::parse("0.1.0").unwrap();
        assert_eq!(deb_version(&v, 1), "0.1.0-1");
        let v = Version::parse("0.2.0-beta.1").unwrap();
        assert_eq!(deb_version(&v, 3), "0.2.0~beta.1-3");
    }

    #[test]
    fn maintainers() {
        assert_eq!(maintainer(Some("A B <a@b.c>")).unwrap(), "A B <a@b.c>");
        assert!(maintainer(Some("just a name")).is_err());
    }

    #[test]
    fn control_file() {
        let meta = Meta::load().unwrap();
        let text = control(&Control {
            meta: &meta,
            version: "0.1.0-1",
            arch: "amd64",
            maintainer: "A <a@b.c>",
            installed_kib: 5000,
            depends: "git, libc6 (>= 2.34)",
        });
        assert!(text.starts_with("Package: vuta\nVersion: 0.1.0-1\nArchitecture: amd64\n"));
        assert!(text.contains("\nDepends: git, libc6 (>= 2.34)\n"));
        assert!(text.contains(&format!("\nDescription: {}\n Vuta keeps", meta.description)));
        // Every continuation line starts with a space and none is empty.
        let body = text.split("\nDescription: ").nth(1).unwrap();
        assert!(
            body.lines()
                .skip(1)
                .all(|l| l.starts_with(' ') && l.len() > 1)
        );
        assert!(text.ends_with('\n') && !text.ends_with("\n\n"));
    }

    #[test]
    fn copyright_file() {
        let meta = Meta::load().unwrap();
        let text = copyright(&meta).unwrap();
        assert!(text.contains("License: MIT or Apache-2.0\n"));
        assert!(text.contains("\nLicense: MIT\n Permission is hereby granted"));
        assert!(text.contains(" .\n"), "blank lines are written as ' .'");
        assert!(!text.contains("\n\n\n"));
    }
}
