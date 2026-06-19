# Release Checklist

This document covers how to bump, build, verify, and ship a release of Agentgrep.

---

## Version bump checklist

1. Update `version` in `Cargo.toml`:

   ```toml
   [package]
   version = "0.X.Y"
   ```

2. Run `cargo check` to update `Cargo.lock`:

   ```bash
   cargo check
   ```

3. Verify `cargo run -- --version` reports the new version:

   ```bash
   cargo run -- --version
   # expected: agentgrep 0.X.Y
   ```

4. Add a new entry to `CHANGELOG.md`:

   ```markdown
   ## [0.X.Y] — YYYY-MM-DD

   Short summary of what changed. Group by feature area, not by commit.
   ```

5. Update `docs/INSTALL.md` if the version example there is hardcoded.

6. Commit with message: `Bump version to 0.X.Y`

---

## Pre-release checks

Run in order. All must pass.

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo test --all-targets
powershell -ExecutionPolicy Bypass -File scripts/smoke.ps1
cargo run -- --version
```

Then run the install verification script:

```powershell
cargo install --path . --force
powershell -ExecutionPolicy Bypass -File scripts/verify-install.ps1
```

---

## Build and install

### Install the binary locally

```bash
cargo install --path . --force
```

`--force` overwrites a previously installed version.

The binary is installed to:

- Linux/macOS: `~/.cargo/bin/agentgrep`
- Windows: `%USERPROFILE%\.cargo\bin\agentgrep.exe`

### Release build only (no install)

```bash
cargo build --release
# binary: target/release/agentgrep (Linux/macOS)
# binary: target\release\agentgrep.exe (Windows)
```

---

## Verify the installed binary

After `cargo install --path . --force`, confirm the right binary is on PATH.

### Windows

```powershell
where.exe agentgrep
# expected: C:\Users\<user>\.cargo\bin\agentgrep.exe

agentgrep --version
# expected: agentgrep 0.X.Y
```

### Linux / macOS

```bash
which agentgrep
# expected: /home/<user>/.cargo/bin/agentgrep

agentgrep --version
# expected: agentgrep 0.X.Y
```

### Verify installed binary matches dev binary

If `agentgrep --version` and `cargo run -- --version` print different versions, the installed binary is stale.

Fix with:

```bash
cargo install --path . --force
```

Then re-verify:

```bash
agentgrep --version
cargo run -- --version
```

Both should print the same version.

---

## Shell completions

Generate completions to stdout and source them in your shell:

```bash
# Bash
agentgrep completions bash > ~/.local/share/bash-completion/completions/agentgrep

# Zsh
agentgrep completions zsh > ~/.zfunc/_agentgrep

# Fish
agentgrep completions fish > ~/.config/fish/completions/agentgrep.fish

# PowerShell
agentgrep completions powershell | Out-File -Encoding utf8 $PROFILE.CurrentUserAllHosts
```

The `completions` subcommand is hidden from `--help` to keep the help output clean.

---

## What is in a release

Files that must be present in any release build or source snapshot:

| Path | Purpose |
|---|---|
| `Cargo.toml` | Package metadata, dependencies, version |
| `Cargo.lock` | Pinned dependency tree (commit this) |
| `src/` | All Rust source |
| `README.md` | User-facing documentation |
| `docs/` | Architecture, JSON contract, agent guide, install guide, release guide |
| `scripts/smoke.ps1` | Smoke verification |
| `scripts/verify-install.ps1` | Install verification |

Files to exclude from release archives (if ever packaged):

| Path | Reason |
|---|---|
| `target/` | Build artifacts, large, reproducible |
| `manual-test/` | Local test captures, not user-facing |
| `.agentgrep/` | Local index cache, repo-specific |
| `.git/agentgrep/` | Local index cache, repo-specific |
| `.github/` | CI config, not needed for binary |

---

## Troubleshooting PATH and version mismatch

### Problem: `agentgrep: command not found`

Cargo's bin directory is not on PATH.

**Linux / macOS:**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
# Add to ~/.bashrc or ~/.zshrc to persist
```

**Windows (PowerShell):**

```powershell
$env:PATH += ";$env:USERPROFILE\.cargo\bin"
# Or add via System Properties > Environment Variables > User PATH
```

### Problem: `agentgrep --version` shows old version after install

The shell may be caching the old binary path.

**Fix:**

```bash
# Re-install
cargo install --path . --force

# Then start a new terminal, or on bash/zsh:
hash -r

# Verify
agentgrep --version
```

On Windows: close and reopen PowerShell, or run:

```powershell
$env:PATH = [System.Environment]::GetEnvironmentVariable('PATH', 'User') + ';' + [System.Environment]::GetEnvironmentVariable('PATH', 'Machine')
agentgrep --version
```

### Problem: `cargo run -- --version` shows different version than `agentgrep --version`

The installed binary predates the current source. Fix with:

```bash
cargo install --path . --force
```

### Problem: two `agentgrep` binaries on PATH

```bash
# Linux/macOS
which -a agentgrep

# Windows
where.exe agentgrep
```

If both `~/.cargo/bin/agentgrep` and another path appear, the first one wins. Ensure `~/.cargo/bin` is listed before system-wide bin directories.

---

## Future release tasks

Not yet done. Revisit after dogfooding:

- GitHub Releases with attached binaries (Linux, macOS, Windows)
- Checksums for release binaries
- `cargo publish` to crates.io (after API stabilizes)
