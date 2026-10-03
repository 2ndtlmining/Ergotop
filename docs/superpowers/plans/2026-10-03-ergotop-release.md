# Ergotop Release (Plan 3 of 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Ergotop shippable from its own repo: CI on every PR, prebuilt release binaries on tag, the Python version preserved as `python-legacy` and removed from `main`, and a README for the Rust app.

**Architecture:** Two GitHub Actions workflows. `ci.yml` runs fmt, clippy and tests on Linux, Windows and macOS. `release.yml` builds five targets on native runners, refreshes the embedded address-book snapshot first, packages archives, and publishes a GitHub Release only for `v*` tags (a manual run builds artifacts without publishing). Repository hygiene: `.gitattributes` forces LF so insta snapshots compare identically on Windows; a LICENSE file makes the README's MIT claim real.

**Tech Stack:** GitHub Actions (`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`), `gh` CLI in the release job.

**Spec:** `docs/superpowers/specs/2026-10-03-ergotop-rust-design.md` (§8 CI and release, §9 migration)

## Global Constraints

- Minimum Rust: 1.89 (`rust-version` in `Cargo.toml`); CI uses latest stable.
- CI gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` on `ubuntu-latest`, `windows-latest`, `macos-latest`.
- Release targets: `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`.
- Release archives: `ergotop-<version>-<target>.tar.gz` (`.zip` on Windows) containing the binary, `README.md`, `LICENSE`.
- Publishing happens only on a pushed `v*` tag. Creating tags, pushing the `python-legacy` ref and publishing a release are outward-facing: ask the user before each.
- License: MIT, `Copyright (c) 2026 2ndtlmining` (repo owner).
- Every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (`<trailer>` below).
- Branch `release` (based on `tui`, PR #26). Open its PR against `main` after #26 is merged.

## Review Focus

1. Insta snapshots checked out with CRLF on Windows runners must still match — handled by `.gitattributes` in Task 1 and proven by the Windows CI job.
2. A transient ergexplorer outage during a release must not ship an empty address-book snapshot — the refresh step keeps the committed file unless the download has ≥ 100 entries (Task 2).
3. A manual (`workflow_dispatch`) run of the release workflow must never publish — the publish job runs only for `refs/tags/v*` (Task 2).
4. Removing the Python package must not break the Rust build (nothing in `crates/` reads Python files at build time) — verified by the CI run in Task 3.
5. README install instructions must work on a fresh Ubuntu server (the user hit apt's cargo 1.75) — README states rustup and the 1.89 minimum explicitly (Task 4).

---

## File Structure

```
.gitattributes                      LF everywhere (snapshots, sources)
LICENSE                             MIT
.github/workflows/ci.yml            fmt + clippy + test, 3 OSes
.github/workflows/release.yml       build 5 targets, refresh snapshot, publish on v* tags
scripts/refresh-addressbook.sh      download + validate ergexplorer snapshot
README.md                           rewritten for the Rust app
removed: ergo_mempool_tui/, requirements.txt, scripts/migrate_origins.py
```

---

### Task 1: CI workflow, line endings, license

**Files:**
- Create: `.gitattributes`, `LICENSE`, `.github/workflows/ci.yml`

**Interfaces:** none (repository configuration).

- [ ] **Step 1: Add `.gitattributes` and renormalize**

`.gitattributes`:

```
* text=auto eol=lf
*.png binary
*.gif binary
```

Run:

```bash
git add --renormalize .
git status --short | head
```

Expected: either no output, or a list of files whose stored line endings changed to LF. Commit them in Step 4.

- [ ] **Step 2: Add `LICENSE`**

```
MIT License

Copyright (c) 2026 2ndtlmining

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 3: Write `.github/workflows/ci.yml`**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
```

- [ ] **Step 4: Validate the YAML locally**

Run: `python -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('ok')"`
Expected: `ok`. (If PyYAML is missing: `pip install pyyaml`.)

- [ ] **Step 5: Commit and push, then watch CI**

```bash
git add .gitattributes LICENSE .github/workflows/ci.yml
git add --renormalize .
git commit -m "ci: fmt, clippy and tests on Linux, Windows, macOS; LF line endings; MIT license

<trailer>"
git push -u origin release
```

The workflow runs on `push` to `main` and on PRs, so trigger it by opening a draft PR from `release` (base `main`):

```bash
gh pr create --draft --base main --head release --title "Rust rewrite, part 3: CI, releases, README" --body "Draft — Plan 3 in progress."
gh run watch --exit-status $(gh run list --branch release --workflow CI --limit 1 --json databaseId --jq '.[0].databaseId')
```

Expected: all five jobs (fmt, clippy, test ×3) succeed. If the Windows test job fails on snapshot mismatches, confirm `.gitattributes` was committed and renormalized; fix code only for genuine platform bugs (record each as a ledger ruling).

---

### Task 2: Release workflow and snapshot refresh

**Files:**
- Create: `scripts/refresh-addressbook.sh`, `.github/workflows/release.yml`

**Interfaces:** `scripts/refresh-addressbook.sh [out_path]` — downloads the ergexplorer book; replaces `out_path` (default `assets/addressbook-snapshot.json`) only if the download parses and has ≥ 100 items; exits 0 either way and prints what it did.

- [ ] **Step 1: Write the refresh script**

`scripts/refresh-addressbook.sh`:

```bash
#!/usr/bin/env bash
# Refreshes the embedded ergexplorer address-book snapshot.
# Keeps the committed snapshot if the download fails or looks incomplete.
set -euo pipefail

out="${1:-assets/addressbook-snapshot.json}"
url="https://api.ergexplorer.com/addressbook/getAddresses?offset=0&limit=5000&type=all&order=nameAsc&query=&testnet=0"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

if ! curl -fsS --max-time 30 "$url" -o "$tmp"; then
  echo "address book download failed; keeping $out"
  exit 0
fi

count="$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["items"]))' "$tmp" 2>/dev/null || echo 0)"
if [ "$count" -lt 100 ]; then
  echo "address book download has $count items (< 100); keeping $out"
  exit 0
fi

mv "$tmp" "$out"
trap - EXIT
echo "address book snapshot refreshed: $count items"
```

Run: `chmod +x scripts/refresh-addressbook.sh && bash scripts/refresh-addressbook.sh /tmp/book-test.json && python3 -c "import json;print(len(json.load(open('/tmp/book-test.json'))['items']))"`
Expected: `address book snapshot refreshed: N items` and the same N (≥ 300) printed.

Run: `bash scripts/refresh-addressbook.sh /tmp/book-test.json` with no network (e.g. `curl` blocked) or simulate by pointing at a bad URL temporarily — Expected: `... keeping /tmp/book-test.json`, exit status 0.

- [ ] **Step 2: Write `.github/workflows/release.yml`**

```yaml
name: Release

on:
  push:
    tags: ["v*"]
  workflow_dispatch:

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: x86_64-pc-windows-msvc
            os: windows-latest
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-22.04
          - target: aarch64-unknown-linux-gnu
            os: ubuntu-24.04-arm
          - target: x86_64-apple-darwin
            os: macos-latest
          - target: aarch64-apple-darwin
            os: macos-latest
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
        with:
          key: ${{ matrix.target }}
      - name: Refresh address-book snapshot
        shell: bash
        run: bash scripts/refresh-addressbook.sh
      - name: Build
        run: cargo build --release --locked -p ergotop --target ${{ matrix.target }}
      - name: Package
        shell: bash
        run: |
          version="${GITHUB_REF_NAME}"
          if [[ "$GITHUB_REF" != refs/tags/* ]]; then version="dev-${GITHUB_SHA::7}"; fi
          name="ergotop-${version}-${{ matrix.target }}"
          mkdir -p "dist/$name"
          cp README.md LICENSE "dist/$name/"
          if [[ "${{ matrix.target }}" == *windows* ]]; then
            cp "target/${{ matrix.target }}/release/ergotop.exe" "dist/$name/"
            (cd dist && 7z a "$name.zip" "$name" > /dev/null)
          else
            cp "target/${{ matrix.target }}/release/ergotop" "dist/$name/"
            tar -C dist -czf "dist/$name.tar.gz" "$name"
          fi
          rm -rf "dist/$name"
          ls -l dist
      - uses: actions/upload-artifact@v4
        with:
          name: ergotop-${{ matrix.target }}
          path: dist/*
          if-no-files-found: error

  publish:
    if: startsWith(github.ref, 'refs/tags/v')
    needs: build
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true
      - name: Create GitHub release
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          ls -l dist
          gh release create "$GITHUB_REF_NAME" dist/* --repo "$GITHUB_REPOSITORY" --title "Ergotop $GITHUB_REF_NAME" --generate-notes
```

Run: `python -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"`
Expected: `ok`.

- [ ] **Step 3: Commit, push, and run the workflow manually (no publish)**

```bash
git add scripts/refresh-addressbook.sh .github/workflows/release.yml
git update-index --chmod=+x scripts/refresh-addressbook.sh
git commit -m "ci: release workflow building 5 targets; snapshot refresh script

<trailer>"
git push
gh workflow run Release --ref release
gh run watch --exit-status $(gh run list --branch release --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId')
```

Expected: five `build` jobs succeed, `publish` is skipped. Then:

Run: `gh run download $(gh run list --branch release --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId') -D /tmp/ergotop-dist && ls -R /tmp/ergotop-dist`
Expected: five archives named `ergotop-dev-<sha7>-<target>.(tar.gz|zip)`. Extract the `x86_64-unknown-linux-gnu` one in the Docker image and run `./ergotop --version` → `ergotop 0.1.0`.

Note: `workflow_dispatch` is only available once the workflow file exists on the default branch. If `gh workflow run` reports the workflow is not found, record a ledger ruling and instead verify the build matrix by temporarily adding `pull_request:` to `release.yml`'s triggers, pushing, watching the run, then removing the trigger again in a follow-up commit.

---

### Task 3: Preserve and remove the Python version

**Files:**
- Delete: `ergo_mempool_tui/` (all files), `requirements.txt`, `scripts/migrate_origins.py`
- Modify: `.gitignore` (drop Python-only entries)

**Interfaces:** none.

- [ ] **Step 1: Ask the user, then create the `python-legacy` refs**

Ask: "OK to push tag and branch `python-legacy` at 31aad86 (the last Python-only commit) to GitHub?" Only after yes:

```bash
git tag -a python-legacy 31aad86 -m "Last Python/Textual version of Ergotop"
git branch python-legacy 31aad86
git push origin python-legacy:refs/tags/python-legacy
git push origin refs/heads/python-legacy:refs/heads/python-legacy
```

Expected: `gh api repos/2ndtlmining/Ergotop/git/refs/tags/python-legacy --jq .ref` prints `refs/tags/python-legacy`.

- [ ] **Step 2: Remove the Python files**

```bash
git rm -r -q ergo_mempool_tui requirements.txt scripts/migrate_origins.py
cat .gitignore
```

Edit `.gitignore` to keep only entries that apply to the Rust project (`/target`, `/.superpowers`, editor/OS files); remove Python-only lines such as `__pycache__/`, `*.pyc`, `.venv/`, `*.egg-info/`.

Run: `git grep -n -I -E "ergo_mempool_tui|requirements.txt|migrate_origins" -- . ':!docs'`
Expected: no matches outside `docs/` (README is rewritten in Task 4; if it matches now, that is expected and resolved there).

- [ ] **Step 3: Verify the Rust build and tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all tests PASS (address-book snapshot and built-in addresses are committed assets; nothing reads Python files).

- [ ] **Step 4: Commit and confirm CI**

```bash
git add -A .gitignore
git commit -m "chore: remove Python version (preserved as python-legacy)

<trailer>"
git push
gh run watch --exit-status $(gh run list --branch release --workflow CI --limit 1 --json databaseId --jq '.[0].databaseId')
```

Expected: CI green on all three OSes.

---

### Task 4: README for the Rust app

**Files:**
- Modify: `README.md` (full rewrite)

**Interfaces:** none.

- [ ] **Step 1: Write the README**

Replace `README.md` with:

````markdown
# Ergotop

A fast, accurate terminal visualizer for the [Ergo](https://ergoplatform.org) mempool — like `htop` for Ergo. Watch transactions fall into the next block, see exactly which ones get mined, and compare what your own nodes and the public explorers see.

Written in Rust (ratatui + tokio). The previous Python/Textual version lives on the [`python-legacy`](https://github.com/2ndtlmining/Ergotop/tree/python-legacy) branch.

## Features

- **Your nodes first.** Polls your own Ergo nodes every second (only new transactions are fetched) and falls back to the public explorers automatically.
- **Accurate.** Real fees (the fee output, no guesses), value excluding change, exact mined transactions per block, real `maxBlockSize` from the node, mined vs dropped told apart.
- **Packing visualizer.** Transactions are selected for the next block by fee per byte, packed bottom-up like the Ergomempool web app, and animated: new ones fall in, mined ones flash and rise out. `l` toggles the ERG hexagon.
- **Sources view.** Status, latency and transaction count for every node and explorer, plus the transactions only one source has.
- **Address book.** Classifies transactions with the [ergexplorer.com address book](https://ergexplorer.com/addressbook) (cached, with an offline snapshot built in), your own `addresses.toml`, and built-in contract rules.
- **Fast.** A full dashboard frame with 10,000 transactions renders in about 1.5 ms; the UI only redraws when something changes.

## Install

### Prebuilt binaries

Download the archive for your platform from [Releases](https://github.com/2ndtlmining/Ergotop/releases), extract it and run `ergotop` (`ergotop.exe` on Windows).

### From source

Requires **Rust 1.89 or newer** via [rustup](https://rustup.rs). Distribution packages are often too old (Ubuntu's `apt install cargo` is 1.75 and will fail).

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo install --git https://github.com/2ndtlmining/Ergotop ergotop
```

Or clone and run: `cargo run --release -p ergotop`. On Debian/Ubuntu you may also need `build-essential`.

## Usage

```bash
ergotop                      # interactive TUI
ergotop --headless           # text output, one line per event
ergotop --config ./mycfg     # read ergotop.toml / addresses.toml from a directory
ergotop --log ergotop.log    # write diagnostics to a file
```

| Key | Action |
|---|---|
| `1` `2` `3` | Dashboard / Packing / Sources |
| `↑` `↓` `PgUp` `PgDn` | Move selection |
| `Enter` | Transaction detail (Sources view: transactions only in that source) |
| `s` | Cycle sort: fee → value → size → age → origin |
| `/` | Filter: name, kind (`exchange`), tx id prefix, `>100`, `<1` (ERG) |
| `Esc` | Clear filter / close popup |
| `c` | Copy tx id (OSC 52 — works over SSH in most terminals) |
| `e` | Open tx in the explorer |
| `l` | Toggle hexagon packing |
| `t` | Cycle theme (neon-green, amber-terminal, blue-ice, high-contrast) |
| `?` | Help |
| `q` | Quit |

The status bar shows the active source: `● node-a` (your node), `○ explorer fallback: p2p`, `… connecting to sources`, or `✕ no data source` (press `3` to see why).

A terminal with true-color support is recommended (Windows Terminal, iTerm2, most Linux terminals).

## Configuration

All files are optional. Ergotop looks in:

| OS | Config directory | Cache (address book) |
|---|---|---|
| Linux | `~/.config/ergotop/` | `~/.cache/ergotop/` |
| macOS | `~/Library/Application Support/ergotop/` | `~/Library/Caches/ergotop/` |
| Windows | `%APPDATA%\ergotop\` | `%LOCALAPPDATA%\ergotop\` |

`ergotop.toml`:

```toml
[[node]]
url = "http://192.168.1.50:9053"
name = "node-a"

[[node]]
url = "http://192.168.1.51:9053"   # nodes are tried in this order

[explorers]
enabled = ["p2p", "public"]          # backup + cross-check; a full URL also works

[ui]
theme = "neon-green"
fps = 30
start_view = "packing"               # dashboard | packing | sources
```

With no `[[node]]` entries Ergotop tries `http://127.0.0.1:9053`. Environment variables override the file: `ERGO_NODE_URL` (one node), `ERGO_API_URL` (one explorer).

Nodes work best as full UTXO nodes with `extraIndex = true`: that enables token names, index-lag health and detail for already-mined transactions. Non-indexed nodes still work for the mempool.

`addresses.toml` — your own labels (they win over the address book):

```toml
[[address]]
address = "9f..."
name = "My mining wallet"
kind = "Local"        # optional: Exchange, Service, Mining pool, Meme, Local
color = "#ff00ff"     # optional
```

## How accuracy is kept

- The active source is the first healthy node in config order, otherwise the first healthy explorer. Switching sources rebuilds the pool once instead of reporting a burst of adds and drops.
- A transaction that leaves the mempool counts as **mined** if it is in a recent block (checked for 15 s), otherwise **dropped**.
- The public explorers sit behind load balancers whose mempool snapshots disagree from poll to poll. With an explorer active, a transaction must be missing from every working explorer for three polls before it leaves the pool, and responses with an empty item list but a non-zero total are treated as errors. Use your own node for the most precise view.

## Development

```bash
cargo test --workspace                       # unit, snapshot and property tests
INSTA_UPDATE=always cargo test -p ergotop    # re-record UI snapshots, then review the .snap diffs
cargo bench -p ergotop --bench frame         # frame-time benchmarks (10k txs)
```

Workspace layout:

```
crates/ergotop-core   data sources, reconciler, classifier, packing (no UI)
crates/ergotop        TUI, headless mode, visualizer
assets/               address-book snapshot, built-in addresses, contract rules
docs/superpowers/     design spec and implementation plans
```

`scripts/refresh-addressbook.sh` refreshes the embedded address-book snapshot (the release workflow runs it before every build). `assets/builtin-addresses.toml` was generated once from the Python version's origin tables.

## License

MIT — see [LICENSE](LICENSE).
````

- [ ] **Step 2: Check the README against the code**

Run: `git grep -n -E "fps|start_view|ERGO_NODE_URL|ERGO_API_URL|--headless|--log|--config" -- crates | head -20`
Expected: every flag, env var and config key named in the README appears in the code. Fix the README for any mismatch.

- [ ] **Step 3: Commit and push**

```bash
git add README.md
git commit -m "docs: README for the Rust app

<trailer>"
git push
```

Expected: CI green. Mark the draft PR ready for review: `gh pr ready`.

---

### Task 5: First release (user-confirmed)

**Files:** none.

- [ ] **Step 1: After the user merges PR #26 and this PR, ask before tagging**

Ask: "Tag `v0.1.0` on `main` and publish the release?" Only after yes:

```bash
git switch main && git pull --ff-only
git tag -a v0.1.0 -m "Ergotop v0.1.0"
git push origin v0.1.0
gh run watch --exit-status $(gh run list --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId')
gh release view v0.1.0
```

Expected: release `v0.1.0` with five archives attached.

---

## Self-Review Notes

- Spec §8: CI (fmt, clippy -D warnings, tests on 3 OSes) → Task 1; release on `v*` with five targets and snapshot refresh → Task 2; `cargo install --git` documented → Task 4. §9 migration steps 1–4 → Tasks 3–5 (step 2 "develop on `rust-rewrite`" happened in Plans 1–2).
- Outward-facing actions (tag/branch pushes, release publication) each have an explicit ask-first step.
