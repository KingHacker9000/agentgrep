# Install and Verify Agentgrep

## Requirements

- Rust stable (1.75 or later): https://rustup.rs
- ripgrep (`rg`) on PATH: https://github.com/BurntSushi/ripgrep

Verify prerequisites:

```bash
rustc --version
rg --version
```

## Install from source

```bash
cargo install --path .
```

This installs the `agentgrep` binary into `~/.cargo/bin/`.

For local development only (no install):

```bash
cargo build
# then use: cargo run -- <command>
```

## Verify the install

Run these commands in order. Each should succeed without error.

### 1. Version check

```bash
agentgrep --version
```

Expected: prints `agentgrep` followed by the current version from `Cargo.toml`.

If you also have a local clone, verify the installed binary matches the source:

```bash
agentgrep --version
cargo run -- --version
```

Both should print the same version. If they differ, run `cargo install --path . --force` to sync.

### 2. Build index

Run from the root of a git repo:

```bash
agentgrep index
```

Expected: index built, file count reported, index stored locally.

### 3. Find a symbol

```bash
agentgrep find "SearchResult"
```

Expected: ranked file candidates with line snippets. At least one result if the codebase contains the term.

### 4. Map a file

```bash
agentgrep map src/rank.rs
```

Expected: file summary with symbols, incoming/outgoing edges, and next actions.

### 5. JSON mode check

```bash
agentgrep find "SearchResult" --json
```

Expected: valid JSON with a `candidates` array, `query` field, and `index_status` field.

## Smoke script (Windows)

Run the full smoke check:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/smoke.ps1
```

This runs:

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- help output captures for all commands
- functional self-test: index, find, map on the agentgrep repo itself

Output is written to `manual-test/`.

## Find the installed binary

### Windows

```powershell
where.exe agentgrep
# expected: C:\Users\<user>\.cargo\bin\agentgrep.exe
```

### Linux / macOS

```bash
which agentgrep
# expected: /home/<user>/.cargo/bin/agentgrep
```

## Troubleshooting

**`rg` not found**

`agentgrep find` requires `rg` on PATH. Install ripgrep:

```bash
# macOS
brew install ripgrep

# Windows
winget install BurntSushi.ripgrep.MSVC

# Cargo
cargo install ripgrep
```

**Index is stale**

```bash
agentgrep index --status
agentgrep index
```

**Binary not found after `cargo install`**

Ensure `~/.cargo/bin` is on your PATH:

```bash
# Linux / macOS
export PATH="$HOME/.cargo/bin:$PATH"
# Add to ~/.bashrc or ~/.zshrc to persist
```

```powershell
# Windows — add to user PATH via System Properties, or for the current session:
$env:PATH += ";$env:USERPROFILE\.cargo\bin"
```

**Installed binary shows wrong version**

The shell may be caching the old path. Re-install and refresh:

```bash
cargo install --path . --force
hash -r              # bash/zsh — clears the command cache
agentgrep --version  # should now match Cargo.toml
```

On Windows, close and reopen the terminal after `cargo install --path . --force`.

For full release verification, see [docs/RELEASE.md](./RELEASE.md).
