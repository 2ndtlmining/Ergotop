# Ergotop

A fast, accurate terminal visualizer for the [Ergo](https://ergoplatform.org) mempool — like `htop` for Ergo. Watch transactions fall into the next block, see exactly which ones get mined, and compare what your own nodes and the public explorers see.

Written in Rust (ratatui + tokio). The previous Python/Textual version lives on the [`python-legacy`](https://github.com/2ndtlmining/Ergotop/tree/python-legacy) branch.

## Features

- **Your nodes first.** Polls your own Ergo nodes every second (only new transactions are fetched) and falls back to the public explorers automatically.
- **Accurate.** Real fees (the fee output, no guesses), value excluding change, exact mined transactions per block, real `maxBlockSize` from the node, mined vs dropped told apart.
- **Packing visualizer.** Transactions are selected for the next block by fee per byte, packed bottom-up like the Ergomempool web app, and animated: new ones fall in, mined ones flash and rise out. `l` toggles the ERG hexagon.
- **Fee rates.** Every transaction's fee per byte (the default sort), mempool median / p90, and the lowest rate still making the next block when it is full. USD values when the ERG price is known; whale transactions are highlighted.
- **Sources view.** Status, latency and transaction count for every node and explorer, plus the transactions only one source has.
- **Address book.** Classifies transactions with the [ergexplorer.com address book](https://ergexplorer.com/addressbook) (cached, with an offline snapshot built in), your own `addresses.toml`, and built-in contract rules.
- **Fast.** A full dashboard frame with 10,000 transactions renders in about 1.1 ms (sorted rows are keyed once per sort, fee-rate stats only recomputed when data changes); the UI only redraws when something changes, at most once a second when idle.

## Install

### Prebuilt binaries (internal)

There are no public releases. To get binaries for Windows, Linux (x86_64 / ARM64) and macOS (Intel / Apple Silicon) without installing Rust, run the **Release** workflow by hand — it builds and uploads artifacts but never publishes:

```bash
gh workflow run Release --ref main
gh run download $(gh run list --workflow Release --limit 1 --json databaseId --jq '.[0].databaseId')
```

or use GitHub → Actions → Release → *Run workflow*, then download the artifact for your platform from the run page. Linux builds need glibc 2.34+ (Ubuntu 22.04, Debian 12, Raspberry Pi OS bookworm or newer).

### From source

Requires **Rust 1.89 or newer** via [rustup](https://rustup.rs). Distribution packages are often too old (Ubuntu's `apt install cargo` is 1.75 and will fail).

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
cargo install --locked --git https://github.com/2ndtlmining/Ergotop ergotop
```

Or clone and run: `cargo run --release -p ergotop`. On Debian/Ubuntu you also need a C compiler for the TLS/crypto dependencies: `apt install build-essential`.

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
| `↑` `↓` `PgUp` `PgDn` | Move selection (in the transaction popup: scroll) |
| `g` `G` `Home` `End` | Top / bottom |
| `Enter` | Transaction detail with its explorer link (Sources view: transactions only in that source) |
| `s` | Cycle sort: fee rate (nanoERG/byte, default) → fee → value → size → age → origin |
| `S` | Reverse the sort direction (▼/▲ shown on the column) |
| `/` | Filter: name, kind (`exchange`), tx id prefix, `>100`, `<1` (ERG) |
| `Esc` | Clear filter / close popup |
| `c` | Copy tx id (OSC 52 — works over SSH in most terminals); Sources view: the source URL |
| `e` | Open tx in the explorer; Sources view: open the source URL |
| `l` | Toggle hexagon packing |
| `t` | Cycle theme (neon-green, amber-terminal, blue-ice, high-contrast) |
| `m` | Toggle motion (animations on/off) |
| `r` | Refresh all sources now (also cuts a failure backoff short) |
| `?` | Help |
| `q` | Quit |

The status bar shows the active source and how fresh its data is: `● node-a · 1s ago` (your node), `○ explorer fallback: p2p · 3s ago`, `◐ node-a · stale 42s` (no update for over 10 s from a node or 30 s from an explorer; also shown in the TRANSACTIONS title), `✕ offline · data 1m 12s old` (no usable source, last data kept on screen), or `… connecting to sources`. Press `3` to see why a source is down.

A terminal with true-color support is recommended (Windows Terminal, iTerm2, most Linux terminals).

## Point it at your node

Ergotop reads your node addresses from `ergotop.toml` in its config folder. Start from the commented example in [`examples/ergotop.toml`](examples/ergotop.toml):

```bash
# Linux / macOS (from the repo folder)
mkdir -p ~/.config/ergotop                       # macOS: ~/Library/Application Support/ergotop
cp examples/ergotop.toml ~/.config/ergotop/ergotop.toml
nano ~/.config/ergotop/ergotop.toml              # set url = "http://<your-node-ip>:9053"
```

```powershell
# Windows
mkdir $env:APPDATA\ergotop -Force
copy examples\ergotop.toml $env:APPDATA\ergotop\ergotop.toml
notepad $env:APPDATA\ergotop\ergotop.toml
```

Then run `ergotop`: the status bar shows `● node-a` when your node is the live source (press `3` for every source's status). Without a config file Ergotop tries `http://127.0.0.1:9053` and falls back to the public explorers.

Other ways to set the node:

- `ergotop --config /path/to/folder` — read `ergotop.toml` / `addresses.toml` from that folder instead
- `ERGO_NODE_URL=http://192.168.1.50:9053 ergotop` — a single node, no file needed

Your own address labels go in `addresses.toml` next to it — see [`examples/addresses.toml`](examples/addresses.toml).

## Configuration

All files are optional (commented examples: [`examples/`](examples/)). Ergotop looks in:

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
motion = true                        # false: no animations
whale_erg = 10000                    # highlight txs moving at least this many ERG; 0 = off
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
