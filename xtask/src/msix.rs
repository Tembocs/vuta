//! `cargo xtask msix`: an MSIX package built with the Windows SDK's
//! `makeappx`, and signed with `signtool` when a certificate is given.
//!
//! The package puts `vuta` on PATH through an app execution alias for a
//! console program, adds no Start menu entry, and turns off file-system
//! write virtualization so the database stays at the documented
//! `%LOCALAPPDATA%\vuta\data\vuta.db` and survives an uninstall.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::meta::{self, Meta, Version};
use crate::{Common, png};

/// The publisher Windows accepts for an unsigned package, installed with
/// `Add-AppxPackage -AllowUnsigned`. The OID marks it as unsigned.
pub const UNSIGNED_PUBLISHER: &str = "CN=Vuta, OID.2.25.311729368913984317654407730594956997722=1";

/// The oldest Windows that supports every manifest feature used here
/// (`desktop6:FileSystemWriteVirtualization` needs version 2004).
const MIN_WINDOWS: &str = "10.0.19041.0";
const MAX_TESTED_WINDOWS: &str = "10.0.26100.0";

/// Environment variable holding the PFX password, so it never appears on
/// a command line.
const PFX_PASSWORD_ENV: &str = "VUTA_PFX_PASSWORD";

#[derive(Debug, Args)]
pub struct MsixArgs {
    #[command(flatten)]
    common: Common,
    /// Sign with this PFX certificate. Its password, if any, is read from
    /// VUTA_PFX_PASSWORD. Without it the package is unsigned.
    #[arg(long, value_name = "FILE", requires = "publisher")]
    pfx: Option<PathBuf>,
    /// Publisher, exactly the certificate's subject, such as "CN=Example"
    /// [default without --pfx: an unsigned-package publisher]
    #[arg(long, value_name = "SUBJECT")]
    publisher: Option<String>,
    /// Publisher name shown to people
    #[arg(long, value_name = "NAME", default_value = "Vuta")]
    publisher_display_name: String,
    /// Timestamp server for the signature, such as http://timestamp.digicert.com
    #[arg(long, value_name = "URL", requires = "pfx")]
    timestamp_url: Option<String>,
    /// The fourth part of the package version, to tell rebuilds apart
    #[arg(long, default_value_t = 0, value_name = "N")]
    revision: u16,
    /// Directory holding makeappx.exe and signtool.exe [default: PATH, then
    /// the newest Windows 10 SDK]
    #[arg(long, value_name = "DIR")]
    sdk_bin: Option<PathBuf>,
}

pub fn run(args: &MsixArgs) -> Result<PathBuf> {
    if !cfg!(windows) {
        bail!("MSIX packages are built on Windows, with makeappx from the Windows SDK");
    }
    let meta = Meta::load()?;
    let triple = meta::triple(&args.common)?;
    let arch = msix_arch(&triple)?;
    let version = msix_version(&meta.version, args.revision)?;
    let publisher = match &args.publisher {
        Some(p) => p.clone(),
        None => UNSIGNED_PUBLISHER.to_owned(),
    };
    if args.pfx.is_none()
        && !publisher.contains("OID.2.25.311729368913984317654407730594956997722=1")
    {
        eprintln!(
            "warning: an unsigned package installs only with the unsigned-package OID in its \
             publisher; omit --publisher or pass --pfx"
        );
    }
    let makeappx = sdk_tool(args.sdk_bin.as_deref(), "makeappx.exe")?;
    let exe = format!("{}.exe", meta.name);
    let binary = meta::release_binary(&args.common, &triple, &exe)?;

    let layout = meta::target_dir()
        .join("package")
        .join("msix")
        .join(format!("layout-{arch}"));
    meta::fresh_dir(&layout)?;
    fs::copy(&binary, layout.join(&exe)).context("cannot copy the binary")?;
    for license in ["LICENSE-MIT", "LICENSE-APACHE"] {
        fs::copy(
            meta::root().join(license),
            layout.join(format!("{license}.txt")),
        )?;
    }
    fs::create_dir_all(layout.join("Assets"))?;
    for (file, size) in ASSETS {
        fs::write(
            layout.join("Assets").join(file),
            png::encode(*size, *size, &png::logo(*size)),
        )?;
    }
    let manifest = manifest(&Manifest {
        meta: &meta,
        version: &version,
        arch,
        publisher: &publisher,
        publisher_display_name: &args.publisher_display_name,
        exe: &exe,
    });
    fs::write(layout.join("AppxManifest.xml"), manifest)?;

    let package = meta::out_dir(&args.common)?.join(format!("{}_{version}_{arch}.msix", meta.name));
    meta::run(
        Command::new(&makeappx)
            .args(["pack", "/o", "/h", "SHA256", "/d"])
            .arg(&layout)
            .arg("/p")
            .arg(&package),
    )?;

    match &args.pfx {
        Some(pfx) => {
            let signtool = sdk_tool(args.sdk_bin.as_deref(), "signtool.exe")?;
            let mut cmd = Command::new(signtool);
            cmd.args(["sign", "/fd", "SHA256", "/f"]).arg(pfx);
            if let Ok(password) = std::env::var(PFX_PASSWORD_ENV) {
                cmd.arg("/p").arg(password);
            }
            if let Some(url) = &args.timestamp_url {
                cmd.args(["/tr", url, "/td", "SHA256"]);
            }
            cmd.arg(&package);
            // Run without echoing the command, which may hold the password.
            let status = cmd.status().context("cannot run signtool")?;
            if !status.success() {
                bail!(
                    "signtool failed ({status}); the --publisher must equal the certificate's subject"
                );
            }
            eprintln!(
                "signed; install with: Add-AppxPackage {}",
                package.display()
            );
        }
        None => eprintln!(
            "unsigned; install with: Add-AppxPackage -AllowUnsigned {}",
            package.display()
        ),
    }
    Ok(package)
}

/// The logo files the manifest names, with their sizes in pixels.
const ASSETS: &[(&str, u32)] = &[
    ("StoreLogo.png", 50),
    ("Square44x44Logo.png", 44),
    ("Square150x150Logo.png", 150),
];

/// MSIX's ProcessorArchitecture for a Rust target triple.
fn msix_arch(triple: &str) -> Result<&'static str> {
    if !triple.contains("-windows-") {
        bail!("{triple} is not a Windows target");
    }
    Ok(match triple.split('-').next().unwrap_or("") {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "i686" | "i586" => "x86",
        _ => bail!("no MSIX architecture for {triple}"),
    })
}

/// `0.1.0` with revision 0 becomes `0.1.0.0`. Each part must fit in 16
/// bits. A pre-release cannot be expressed and would sort equal to the
/// final release, so it is refused.
fn msix_version(version: &Version, revision: u16) -> Result<String> {
    if let Some(pre) = &version.pre {
        bail!(
            "MSIX versions have no pre-release part, so {version} would collide with the \
             release; drop the '-{pre}' or package the release itself"
        );
    }
    let part =
        |n: u64| u16::try_from(n).with_context(|| format!("version {version}: {n} exceeds 65535"));
    Ok(format!(
        "{}.{}.{}.{revision}",
        part(version.major)?,
        part(version.minor)?,
        part(version.patch)?
    ))
}

struct Manifest<'a> {
    meta: &'a Meta,
    version: &'a str,
    arch: &'a str,
    publisher: &'a str,
    publisher_display_name: &'a str,
    exe: &'a str,
}

fn manifest(m: &Manifest) -> String {
    let description = xml(&m.meta.description);
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap5="http://schemas.microsoft.com/appx/manifest/uap/windows10/5"
  xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
  xmlns:desktop6="http://schemas.microsoft.com/appx/manifest/desktop/windows10/6"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap uap5 desktop4 desktop6 rescap">
  <Identity Name="{name}" Publisher="{publisher}" Version="{version}" ProcessorArchitecture="{arch}" />
  <Properties>
    <DisplayName>Vuta</DisplayName>
    <PublisherDisplayName>{publisher_display_name}</PublisherDisplayName>
    <Description>{description}</Description>
    <Logo>Assets\StoreLogo.png</Logo>
    <desktop6:FileSystemWriteVirtualization>disabled</desktop6:FileSystemWriteVirtualization>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="{MIN_WINDOWS}" MaxVersionTested="{MAX_TESTED_WINDOWS}" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Applications>
    <Application Id="vuta" Executable="{exe}" EntryPoint="Windows.FullTrustApplication"
                 desktop4:Subsystem="console" desktop4:SupportsMultipleInstances="true">
      <uap:VisualElements DisplayName="Vuta" Description="{description}" BackgroundColor="transparent"
                          Square150x150Logo="Assets\Square150x150Logo.png"
                          Square44x44Logo="Assets\Square44x44Logo.png" AppListEntry="none" />
      <Extensions>
        <uap5:Extension Category="windows.appExecutionAlias">
          <uap5:AppExecutionAlias desktop4:Subsystem="console">
            <uap5:ExecutionAlias Alias="{exe}" />
          </uap5:AppExecutionAlias>
        </uap5:Extension>
      </Extensions>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
    <rescap:Capability Name="unvirtualizedResources" />
  </Capabilities>
</Package>
"#,
        name = xml(&package_name(&m.meta.name)),
        publisher = xml(m.publisher),
        version = m.version,
        arch = m.arch,
        publisher_display_name = xml(m.publisher_display_name),
        exe = xml(m.exe),
    )
}

/// The Identity Name: letters, digits, `.` and `-`, three or more
/// characters. `vuta` becomes `Vuta`.
fn package_name(crate_name: &str) -> String {
    let mut chars = crate_name.chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase());
    first
        .into_iter()
        .chain(chars)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Find a Windows SDK tool: `--sdk-bin`, then PATH, then the newest
/// `Windows Kits\10\bin\<version>\<host arch>` that has it.
fn sdk_tool(sdk_bin: Option<&Path>, tool: &str) -> Result<PathBuf> {
    if let Some(dir) = sdk_bin {
        let path = dir.join(tool);
        if path.is_file() {
            return Ok(path);
        }
        bail!("{} has no {tool}", dir.display());
    }
    if let Some(path) = std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(tool))
        .find(|p| p.is_file())
    {
        return Ok(path);
    }
    let program_files = std::env::var_os("ProgramFiles(x86)")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .context("ProgramFiles is not set")?;
    let bin = program_files.join("Windows Kits").join("10").join("bin");
    let host = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let mut versions: Vec<(Vec<u32>, PathBuf)> = fs::read_dir(&bin)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let key: Option<Vec<u32>> = name.split('.').map(|p| p.parse().ok()).collect();
            let path = entry.path().join(host).join(tool);
            Some((key?, path)).filter(|(_, p)| p.is_file())
        })
        .collect();
    versions.sort();
    versions.pop().map(|(_, path)| path).with_context(|| {
        format!("cannot find {tool}; install the Windows SDK, or pass --sdk-bin <dir>")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectures() {
        assert_eq!(msix_arch("x86_64-pc-windows-msvc").unwrap(), "x64");
        assert_eq!(msix_arch("aarch64-pc-windows-msvc").unwrap(), "arm64");
        assert_eq!(msix_arch("i686-pc-windows-gnu").unwrap(), "x86");
        assert!(msix_arch("x86_64-unknown-linux-gnu").is_err());
    }

    #[test]
    fn versions() {
        assert_eq!(
            msix_version(&Version::parse("0.1.0").unwrap(), 0).unwrap(),
            "0.1.0.0"
        );
        assert_eq!(
            msix_version(&Version::parse("1.2.3").unwrap(), 7).unwrap(),
            "1.2.3.7"
        );
        assert!(msix_version(&Version::parse("0.2.0-beta.1").unwrap(), 0).is_err());
        assert!(msix_version(&Version::parse("70000.0.0").unwrap(), 0).is_err());
    }

    #[test]
    fn names_and_escaping() {
        assert_eq!(package_name("vuta"), "Vuta");
        assert_eq!(package_name("my_tool"), "My-tool");
        assert_eq!(xml(r#"a<b>&"c'"#), "a&lt;b&gt;&amp;&quot;c&apos;");
    }

    #[test]
    fn manifest_has_identity_alias_and_assets() {
        let meta = Meta::load().unwrap();
        let text = manifest(&Manifest {
            meta: &meta,
            version: "0.1.0.0",
            arch: "x64",
            publisher: UNSIGNED_PUBLISHER,
            publisher_display_name: "Vuta & co",
            exe: "vuta.exe",
        });
        assert!(text.contains(r#"<Identity Name="Vuta" Publisher="CN=Vuta, OID.2.25"#));
        assert!(text.contains(r#"Version="0.1.0.0" ProcessorArchitecture="x64""#));
        assert!(text.contains(r#"<uap5:ExecutionAlias Alias="vuta.exe" />"#));
        assert!(text.contains("Vuta &amp; co"));
        for (file, _) in ASSETS {
            assert!(text.contains(&format!(r"Assets\{file}")), "{file}");
        }
        // Every opened element is closed: a cheap well-formedness check.
        let opens = text.matches('<').count();
        let closes = text.matches('>').count();
        assert_eq!(opens, closes);
        assert_eq!(text.matches("<Application ").count(), 1);
        assert_eq!(text.matches("</Application>").count(), 1);
    }
}
