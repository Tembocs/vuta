# Vuta

Vuta ("pull" in Swahili) is a command-line tool that keeps many source checkouts cloned and up to date. Updates are fast-forward only, repositories sync in parallel, and all state lives in one SQLite database.

> **Status:** early development (milestone M0). `vuta db path` and `vuta doctor` work; the registry and sync commands are next. See [ARCHITECTURE.md](ARCHITECTURE.md) for the design and milestones.

## Build

Requires Rust 1.89 or newer and `git` on PATH.

```sh
cargo build --release
./target/release/vuta doctor
```

## Usage

```sh
vuta db path        # where the database lives
vuta doctor         # check git, the database, and every root
```

Global options: `--db <PATH>` (or `VUTA_DB`), `--color auto|always|never` (or `VUTA_COLOR`), `-q/--quiet`, `-v/--verbose`.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT) at your option.
