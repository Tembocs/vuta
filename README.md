# Vuta

Vuta ("pull" in Swahili) is a command-line tool that keeps many source checkouts cloned and up to date. Updates are fast-forward only unless a repository opts into more, repositories sync in parallel, and all state lives in one SQLite database.

> **Status:** early development. Every v1 command works (milestones M0 to M4); there is no release yet. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and milestones.

## Build

Requires Rust 1.89 or newer and `git` on PATH.

```sh
cargo build --release
./target/release/vuta doctor
```

### Packages

`cargo xtask deb` builds a Debian package on Linux, and `cargo xtask msix` builds an MSIX package on Windows. Both write to `target/package/`. See [docs/PACKAGING.md](docs/PACKAGING.md) for requirements, signing and what each package installs.

## Usage

```sh
vuta root add src ~/src                       # a base directory for checkouts
vuta add https://github.com/zed-industries/zed.git rust/zed --tag rust   # register and clone
vuta sync                                     # clone what is missing, fast-forward the rest
vuta sync zed --fetch-only                    # only fetch, and report how far behind
vuta sync --tag rust -j 8 --strict            # filter, set parallelism, fail on skips
vuta scan --tag-from-parent                   # adopt checkouts already on disk
vuta hook add v "make" --platform unix        # run after v changes
vuta status                                   # local state, without fetching
vuta log                                      # recent runs and what happened
vuta list                                     # one line per repository
vuta show zed                                 # everything about one repository
vuta set zed --branch main                    # change the record
vuta set dotfiles --allow-dirty               # fast-forward around local edits upstream does not touch
vuta set mylib --strategy rebase              # replay local commits onto upstream
vuta add https://github.com/example/app.git --submodules   # clone and update with submodules
vuta tag zed +editor -rust                    # add and remove tags
vuta remove zed --delete-files                # forget it, and delete the checkout after asking
vuta config set sync.jobs 8                   # settings: sync.jobs, history.keep_runs, ui.glyphs
vuta db path                                  # where the database lives
vuta doctor                                   # check git, the database, and every root
```

Global options: `--db <PATH>` (or `VUTA_DB`), `--color auto|always|never` (or `VUTA_COLOR`), `-q/--quiet`, `-v/--verbose`.

The first `vuta add` or `vuta scan` with no root registered asks whether to use the current directory (or the directory being scanned) as the first root. Without a terminal it prints the `vuta root add` command instead. While `vuta sync` runs in a terminal, a live line per repository in progress shows what it is doing and for how long.

Updates are fast-forward only. A checkout with local changes, local commits that diverge from upstream, a detached HEAD, no upstream branch or a different `origin` is skipped and left untouched. `vuta sync` exits 0 when everything succeeded, 1 when a repository, hook or submodule update failed (or a repository was skipped, with `--strict`), 3 when it could not start, and 130 on Ctrl-C.

Three options per repository, all off by default, relax this where you ask for it. `vuta show` lists them, and `vuta export` carries them:

- `vuta set <name> --allow-dirty` fast-forwards a checkout with local edits, as long as upstream changes none of the edited files. Otherwise it is skipped, naming the files both sides changed.
- `vuta set <name> --strategy rebase` replays local commits onto upstream instead of skipping a diverged checkout. The tree must be clean. If the rebase stops, for example on a conflict, Vuta aborts it and reports a skip; the checkout is left as it was.
- `vuta set <name> --submodules` (or `vuta add --submodules`) runs `git submodule update --init --recursive` after each clone and update. A failure shows on the result line and fails the run, but the repository's own update stands and the next sync tries again. A submodule holding a commit nothing else holds is never moved.

`--no-allow-dirty`, `--strategy ff-only` and `--no-submodules` turn them off again.

### Moving to another machine

```sh
vuta export -o vuta-registry.json             # on the first machine
vuta import vuta-registry.json --map-root src=/home/me/code   # on the second
vuta sync                                     # clone everything
```

The file holds roots, repositories, tags, hooks and changed settings, but no history. `--map-root NAME=PATH` places a root elsewhere, or joins a root you already have at that path. Import only adds, lists every hook it adds (`--no-hooks` leaves them out), and `--dry-run` shows the plan first.

### Scripting

`vuta list`, `status`, `show`, `log`, `sync`, `scan`, `import`, `root list`, `hook list` and `config list` take `--format json` and then print one JSON value on stdout, with progress and hook output on stderr:

```sh
vuta sync --format json | jq -r '.results[] | select(.outcome == "failed") | .repo'
```

The fields are described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) section 9.

### Shell completions

Completion covers subcommands and flags, and also the names in your registry: repositories, roots, tags, hook ids and setting keys (`vuta sync z<TAB>` gives `zed`). `vuta completions <shell>` prints a short script that registers `vuta` with `bash`, `zsh`, `fish`, `powershell` or `elvish`; on each TAB the shell asks `vuta` itself. The Debian package installs the bash, zsh and fish scripts for you. Otherwise, load the script when the shell starts, so it always matches the installed `vuta`:

```sh
echo 'source <(vuta completions bash)' >> ~/.bashrc
echo 'source <(vuta completions zsh)' >> ~/.zshrc      # after compinit
echo 'vuta completions fish | source' > ~/.config/fish/completions/vuta.fish
```

In PowerShell, add `vuta completions powershell | Out-String | Invoke-Expression` to `$PROFILE`.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT) at your option.
