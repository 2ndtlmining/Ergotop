# Ergotop (Rust) — Design Spec

Date: 2026-10-03
Status: Draft for review
Repo: `2ndtlmining/Ergotop` (Rust replaces Python on `main`; Python preserved as `python-legacy`)

## 1. Goal

An accurate, very fast, fun-to-watch terminal visualizer for the Ergo mempool, in its own repo, that:

- Uses the user's own LAN nodes as the source of truth, with public explorers as fallback and as a cross-check.
- Classifies transactions using the ergexplorer address book plus local overrides.
- Makes block packing (ported from Ergomempool v1) the visual centerpiece.
- Ships as a single binary for Windows, Linux and macOS.

### Non-goals (v1)

- Wallet features, signing or submitting transactions.
- Load balancing across nodes (Ergotop does its own health-checked failover; a proxy URL may still be configured as an ordinary node).
- Mempool rain, whale effects and fee heatmap animations (the animation module must allow adding these later without restructuring).
- Testnet support.

### Problems in the Python version this design fixes

| Problem | Fix |
|---|---|
| Default node `213.239.193.208:9053` is dead; every network poll stalls ~10s | No hardcoded remote node; per-source tasks with 3s timeouts never block the UI |
| Sequential 12s fallback chain | Sources run concurrently and independently |
| Full mempool (`limit=10000`) re-downloaded every 5s | Node: poll `transactionIds`, fetch only new txs |
| Fee defaults to fake `0.001` if fee output not found | Fee = actual value sent to the fee contract, else 0 |
| "Value" sums all outputs including change | Value = ERG sent to addresses not among the inputs |
| Age = first time this client saw the tx | First-seen across all sources (+ explorer creation timestamp where present) |
| Block size hardcoded as 2 MB | `maxBlockSize` from node `/info` → `parameters` |
| Block mined → tx removal guessed (v1) | Remove exactly the txs in the mined block; others leaving are "dropped" |
| ~30 platforms hardcoded in `origins.py` | ergexplorer address book + local `addresses.toml` + rules file |
| Visualizer ticks at 5 fps | 30 fps render loop (configurable) |

## 2. Architecture

Cargo workspace, two crates:

- **`ergotop-core`** (library, no UI, no terminal deps): models, config, data sources, reconciler, classifier, packing algorithm. Fully unit-testable offline.
- **`ergotop`** (binary): ratatui + crossterm TUI. Owns all state in one task; renders views.

Runtime: tokio. HTTP: reqwest (rustls). Serialization: serde / serde_json / toml.

```
 node task(s) ─┐
 explorer tasks ├─ SourceEvent ─▶ mpsc ─▶ UI task (state owner) ──▶ ratatui frame
 addressbook  ──┤                            ▲      ▲
 price        ──┘                 key events ┘      └ 30 fps tick (only while animating/dirty)
```

The UI task waits (`tokio::select!`) on: terminal input, source events, and an animation tick. It redraws only when state is dirty or an animation is running. Network I/O never happens on the UI task.

### 2.1 Repository layout

```
Ergotop/
  Cargo.toml                    workspace
  crates/ergotop-core/src/
    lib.rs
    model.rs                    Tx, TxIo, Token, Block, NodeInfo, Classification, SourceId
    config.rs                   ergotop.toml, addresses.toml, env vars, defaults
    sources/
      mod.rs                    MempoolSource trait, SourceEvent, SourceStatus
      node.rs                   Ergo node REST client + poll loop
      explorer.rs               Explorer API client (public + p2p) + poll loop
      addressbook.rs            ergexplorer fetch, disk cache, embedded snapshot
      price.rs                  ERG/USD oracle
    reconcile.rs                canonical mempool, per-source views, mined vs dropped
    classify/
      mod.rs                    Classifier (lookup order), tx-level classification
      ergotree.rs               address <-> ErgoTree (base58 + checksum), no ergo-lib
      book.rs                   address book entries -> ErgoTree map
      rules.rs                  contract-template rules (Spectrum, SkyHarbor, ...)
    packing.rs                  gravity packing + ERG hexagon mask
  crates/ergotop/src/
    main.rs                     CLI args, config load, terminal setup, panic hook
    app.rs                      state owner, event loop, key handling
    views/
      dashboard.rs
      packing.rs
      sources.rs
    widgets/                    tx table, tx detail, blocks, network, origins, header, status bar
    anim/                       Animation trait, falling-sand drop, mined sweep, header flash
    theme.rs                    4 themes ported from Python
  assets/
    addressbook-snapshot.json   refreshed at release time
    rules.toml                  contract-template rules migrated from origins.py
  docs/superpowers/specs/
```

## 3. Data layer (`ergotop-core`)

### 3.1 Sources

All sources are optional. Each runs in its own tokio task, uses a 3s request timeout, backs off exponentially on failure (max 30s), and emits `SourceEvent`s. A failing source is marked `Down` and never stalls other sources or the UI.

**Node (primary).** Zero or more nodes from config. Endpoints (verified against the node OpenAPI spec):

| Purpose | Endpoint | Cadence |
|---|---|---|
| Health, height, `maxBlockSize`, peers, version | `GET /info` (`parameters.maxBlockSize`) | 10s (health check every 30s when Down) |
| Mempool ids | `GET /transactions/unconfirmed/transactionIds` | 1s |
| New txs only | `POST /transactions/unconfirmed/byTransactionIds` | when ids diff is non-empty |
| Resolve input boxes (addresses/values) | `POST /utxo/withPool/byIds` | for new txs, batched |
| Recent blocks | `GET /blocks/lastHeaders/{n}` | 5s |
| Exact mined tx ids | `GET /blocks/{headerId}/transactions` | on new header |

Node tx JSON has `size`; outputs carry `ergoTree` (not address); inputs carry only `boxId`, hence the batched input resolution.

Risk: `/utxo/withPool/byIds` needs a node with UTXO state. If it errors (e.g. digest-mode node), the node source still supplies the mempool, and input addresses/values for those txs are taken from an explorer when available, otherwise left unresolved (value computed from outputs only, flagged in detail view).

Node selection: all healthy nodes are polled; the **active node** (shown in the status bar) is the first healthy node in config order. The canonical mempool follows the active node. Others feed the consistency view.

**Explorers (backup + cross-check).** `public` = `https://api.ergoplatform.com`, `p2p` = `https://api-p2p.ergoplatform.com`. Poll `GET /transactions/unconfirmed?limit=10000` every 5s; blocks via `GET /api/v1/blocks?limit=n` every 10s. If no node is healthy, the first healthy explorer becomes the active source ("explorer fallback" in the status bar).

**Address book.** `GET https://api.ergexplorer.com/addressbook/getAddresses?offset=&limit=&type=all&order=nameAsc&query=&testnet=0`, paged until `total` reached. Entry fields used: `address`, `name`, `type` (`Service`, `Exchange`, `Mining pool`, `Meme`), `urltype`, `url`.
- Cached at `<user cache dir>/ergotop/addressbook.json` (`%LOCALAPPDATA%\ergotop\` on Windows), refreshed in background if older than 24h.
- Load order at startup: cache → embedded `assets/addressbook-snapshot.json` (refreshed at release). Startup never waits on the network.

**Price.** ERG/USD oracle `https://erg-oracle-ergusd.spirepools.com/frontendData`, every 5 min.

### 3.2 Reconciler

Maintains:
- `canonical: HashMap<TxId, TxEntry>`, following the active source.
- Per source: set of tx ids, last update time, latency, status.
- `TxEntry { tx, first_seen, seen_by: SourceSet, classification }`.

Events produced for the UI: `TxAdded`, `TxMined { block, tx_ids }`, `TxDropped { tx_ids }`, `BlockAdded`, `SourceStatusChanged`, `NetworkInfo`, `Price`.

Rules:
- A tx leaving the active mempool is `Mined` if its id is in a block fetched from the active node (or explorer block detail when in fallback), otherwise `Dropped` after a 2-poll grace period (avoids flicker during block arrival).
- Empty-mempool guard (from Python): an empty response when >5 txs are cached is treated as a transient error once.
- Active source switch (failover) does a full resync without emitting add/drop animations for the whole set.

### 3.3 Tx metrics

- **fee**: sum of outputs whose ErgoTree equals the fee contract tree; 0 if none.
- **value**: ERG to output trees not present among input trees and not the fee tree. If inputs unresolved: sum of non-fee outputs, marked `approx`.
- **size**: from source.
- **first_seen**: earliest time any source reported the tx; explorer `creationTimestamp` shown in detail when available.

### 3.4 Classification

All matching is on ErgoTree hex. Address-book and override addresses are converted to ErgoTree once at load:
- P2PK: `0008cd` + 33-byte pubkey from the base58 payload.
- P2S: payload bytes are the tree.
- P2SH: match by the 24-byte script hash against output trees of the P2SH template.
- Checksum validated (blake2b256, first 4 bytes); invalid entries are skipped with a warning.

Lookup order per tree: local `addresses.toml` → address book → `rules.toml` contract templates (prefix/template match) → heuristics (P2PK → `P2P`, other → `Contract`) → `Unknown`.

Tx-level classification: first named match among outputs (excluding fee), then among inputs. If both sides have distinct named matches, detail shows `From → To` (e.g. `Kucoin → Spectrum`); the tx's color/category is the output-side match.

`Classification { name, kind, color }` where `kind ∈ {Exchange, Service, MiningPool, Meme, Local, P2P, Contract, Unknown}`. Color: hue per kind, stable shade per name (hash of name); built-in overrides keep the Python palette for well-known platforms.

Mining pools for blocks: the miner reward output of the block's first (emission/reward) transaction is classified with the same classifier — address-book pool entries are these `88dhgz…` reward-contract addresses (~45 today). Fallback: explorer-provided miner name (ignoring the 8-char address-suffix placeholder, as the Python version does), then `Other`.

## 4. TUI (`ergotop`)

### 4.1 Views (number keys)

1. **Dashboard** — three columns, as in the Python version:
   - Left: block utilization (vs real `maxBlockSize`), mempool summary, recent blocks with pool names.
   - Center: packing visualizer (compact) above the tx table.
   - Right: network info, origin breakdown by classification, tx detail.
2. **Packing** — full-screen packing visualizer (the default "leave it running" view).
3. **Sources** — per-source status, latency, tx count, last update; `Enter` lists txs only that source has.

Status bar: active source (`● node-a 192.168.1.50` / `○ explorer fallback`), source health dots, key hints.

### 4.2 Packing visualizer

Port of Ergomempool v1 `PackingAlgorithm.js` (gravity, bottom-up) and `ErgoPackingGrid` (hexagon mask), on a terminal cell grid:
- Half-block rendering (`▀ ▄ █` with fg/bg colors) → 2 vertical pixels per cell.
- Each tx is a square, side scaled from bytes (min 1px, max configurable, normalized like v1); color = classification color.
- Packed largest-first, bottom-up gravity, into the next-block region sized by `maxBlockSize`; txs that do not fit are drawn dimmed in an overflow zone above the capacity line.
- **New tx**: falls from the top to its packed position (falling sand).
- **Block mined**: exactly the mined txs flash, then sweep out; remaining txs settle down by gravity; overflow drops into freed space; header shows pool name and reward.
- `l` toggles ERG-hexagon mode.
- Repacking happens in core (`packing.rs`), animations interpolate between old and new positions in `anim/`.

### 4.3 Keys

| Key | Action |
|---|---|
| `1` `2` `3` | Dashboard / Packing / Sources |
| `↑↓` `PgUp/PgDn` | Navigate tx list |
| `Enter` | Tx detail |
| `s` | Cycle sort: fee → value → size → age → origin |
| `/` | Filter: name, kind (`/exchange`), partial tx id, `>100`, `<1` |
| `Esc` | Clear filter / close |
| `c` | Copy tx id |
| `e` | Open tx in explorer |
| `l` | Toggle hexagon packing mode |
| `t` | Cycle theme |
| `?` | Help |
| `q` | Quit |

Themes ported from Python: neon-green, amber-terminal, blue-ice, high-contrast.

### 4.4 Animation

`trait Animation { fn tick(&mut self, dt: Duration) -> bool /* still running */; fn draw(&self, buf: &mut Buffer, area: Rect); }`. v1 animations: tx drop, mined flash+sweep, gravity settle, header flash. Additional effects (rain, whale, heatmap) are new implementors later.

## 5. Configuration

Files live in the platform config dir (`%APPDATA%\ergotop\` on Windows, `~/.config/ergotop/` on Linux/macOS), or a path given with `--config`. All optional.

`ergotop.toml`:
```toml
[[node]]
url = "http://192.168.1.50:9053"
name = "node-a"

[[node]]
url = "http://192.168.1.51:9053"

[explorers]
enabled = ["p2p", "public"]

[ui]
theme = "neon-green"
fps = 30
start_view = "packing"
```

Defaults when no file: one node `http://127.0.0.1:9053` (probed; silently Down if absent), explorers `p2p` + `public`.

Env vars override the file: `ERGO_NODE_URL` (single node, replaces node list), `ERGO_API_URL` (replaces explorer list with one URL).

`addresses.toml`:
```toml
[[address]]
address = "9f..."
name = "My mining wallet"
kind = "Local"        # optional, defaults to Local
color = "#ff00ff"     # optional
```

## 6. Error handling

- Source errors → `SourceStatus::Down { reason, since }`, shown in status bar and Sources view. Never panics, never blocks UI.
- Malformed JSON from a source → that poll is discarded, logged, counted as a failure.
- Bad config → start with defaults and show a one-line warning in the status bar.
- Panic hook restores the terminal (leave alternate screen, disable raw mode) before printing the panic.
- Optional `--log <file>` writes `tracing` logs; nothing is written to the terminal while the TUI runs.

## 7. Testing

- **Core unit tests** with recorded JSON fixtures (captured from a real node and both explorers), no network:
  - reconciler: tx added, mined (exact ids), dropped (grace period), source disagreement, failover resync, empty-mempool guard.
  - metrics: fee, value with/without change, unresolved inputs.
  - ergotree: address → tree → address round-trip for P2PK/P2S/P2SH; invalid checksum rejected.
  - classifier: lookup order (override beats book beats rules beats heuristic).
  - addressbook: paging, cache freshness, fallback to embedded snapshot.
- **Packing property tests** (proptest): no overlaps, within bounds, capacity respected, bottom-heavy.
- **TUI snapshot tests**: each view rendered with ratatui `TestBackend` + `insta`.
- **Benchmark** (criterion): pack + render a 10,000-tx mempool in < 5 ms per frame on a typical desktop.
- **Live check** (manual, documented in README): run against a user LAN node and both explorers; Sources view counts agree within normal propagation lag. A Docker Ergo node may be used if a LAN node is unavailable (requires sync time before it has a mempool).

## 8. CI and release

GitHub Actions:
- On PR/push: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on windows-latest, ubuntu-latest, macos-latest.
- On tag `v*`: build release binaries (x86_64 Windows, x86_64/aarch64 Linux, x86_64/aarch64 macOS), refresh `addressbook-snapshot.json` before build, attach to a GitHub Release. `cargo install --git` documented.

## 9. Migration

1. Tag current `main` (`31aad86`) as `python-legacy` and create branch `python-legacy`.
2. Develop on `rust-rewrite`.
3. PR `rust-rewrite` → `main`: removes Python sources, adds the workspace, rewrites README (features, config, keys, screenshots/GIF).
4. Release `v0.1.0`.
