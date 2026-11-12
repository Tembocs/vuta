# Vuta

Vuta ("pull" in Swahili) is a command-line tool that keeps many source checkouts cloned and up to date. Updates are fast-forward only, repositories sync in parallel, and all state lives in one SQLite database.

> **Status:** early development (milestone M1). The registry commands work; syncing is next. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and milestones.

## Build

Requires Rust 1.89 or newer and `git` on PATH.

```sh
cargo build --release
./target/release/vuta doctor
```

## Usage

```sh
vuta root add src ~/src                       # a base directory for checkouts
vuta add https://github.com/zed-industries/zed.git rust/zed --tag rust --no-clone
vuta list                                     # one line per repository
vuta show zed                                 # everything about one repository
vuta set zed --branch main                    # change the record
vuta tag zed +editor -rust                    # add and remove tags
vuta remove zed --delete-files                # forget it, and delete the checkout after asking
vuta config set sync.jobs 8                   # settings: sync.jobs, history.keep_runs, ui.glyphs
vuta db path                                  # where the database lives
vuta doctor                                   # check git, the database, and every root
```

Global options: `--db <PATH>` (or `VUTA_DB`), `--color auto|always|never` (or `VUTA_COLOR`), `-q/--quiet`, `-v/--verbose`.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT) at your option.
