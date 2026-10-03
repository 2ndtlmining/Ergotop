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
