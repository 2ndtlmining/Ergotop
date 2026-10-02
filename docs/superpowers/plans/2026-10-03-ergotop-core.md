# Ergotop Core (Plan 1 of 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `ergotop-core` (data sources, classifier, reconciler, packing) plus an `ergotop --headless` mode that prints the reconciled live mempool, so the data layer can be verified against the user's LAN nodes before any TUI exists.

**Architecture:** Cargo workspace with `crates/ergotop-core` (library, no terminal deps) and `crates/ergotop` (binary). Per-source tokio tasks send `SourceEvent`s over an mpsc channel; a pure, synchronous `Reconciler` folds them into one canonical mempool and emits `Update`s. Classification matches mainnet address strings (node ErgoTrees are converted to addresses at ingest).

**Tech Stack:** Rust stable ≥ 1.80 (edition 2021), tokio, reqwest (rustls), serde/serde_json/toml, bs58, blake2, hex, dirs, thiserror, clap; tests use wiremock and proptest.

**Spec:** `docs/superpowers/specs/2026-10-03-ergotop-rust-design.md`

**Later plans:** Plan 2 = TUI (views, packing animation, keys, themes, benchmark). Plan 3 = CI, release, `python-legacy` migration, README.

## Global Constraints

- Mainnet only. All ERG amounts are `u64` nanoERG (1 ERG = 1_000_000_000).
- No `ergo-lib` / sigma-rust dependency.
- HTTP request timeout 3s; failure backoff 1s doubling to a 30s cap.
- Cadences: node mempool ids 1s, node `/info` 10s, node headers 5s; explorer mempool 5s, explorer blocks 10s; address book refresh 24h; price 5 min.
- Fee contract address: `2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1bT2Fk4FkFLCfdPpe`.
- Default node when none configured: `http://127.0.0.1:9053` (id `local`). Explorers: `p2p` = `https://api-p2p.ergoplatform.com`, `public` = `https://api.ergoplatform.com`.
- Config dir `dirs::config_dir()/ergotop` (`ergotop.toml`, `addresses.toml`); cache dir `dirs::cache_dir()/ergotop` (`addressbook.json`).
- Env overrides: `ERGO_NODE_URL` replaces the node list; `ERGO_API_URL` replaces the explorer list.
- A source failure must never block the UI/headless loop or other sources.
- Every commit message ends with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (shown as `<trailer>` in commit steps below; write the full line).
- Work happens on branch `rust-rewrite`. The Python package stays in the repo until Plan 3.

## Review Focus

1. A node whose `POST /transactions/unconfirmed/byTransactionIds` fails or returns an unexpected shape (the OpenAPI spec documents the response as strings) must still deliver txs via per-id GETs — pinned in Task 6.
2. The active node going down mid-run must fail over to an explorer with a single `Resynced`, not a burst of drops/adds — pinned in Task 9.
3. A block arriving before *or* after its txs leave the mempool must yield `Mined`, never `Dropped` — pinned in Task 9 (both orders).
4. An explorer returning an empty mempool once (API glitch) must not wipe the pool — pinned in Task 9.
5. First run with no network and no cache must still classify using the embedded address-book snapshot — pinned in Task 8.

---

## File Structure

```
Cargo.toml                                  workspace
.gitignore                                  + /target
assets/
  builtin-addresses.toml                    generated (Task 5) from Python origins
  rules.toml                                address-prefix rules (Task 5)
  addressbook-snapshot.json                 captured ergexplorer response (Task 8)
scripts/migrate_origins.py                  one-off generator (Task 5)
crates/ergotop-core/
  Cargo.toml
  src/lib.rs
  src/model.rs                              Tx, BoxData, Block, NodeInfo, SourceId/Kind/Status, test_util
  src/ergotree.rs                           tree <-> address
  src/metrics.rs                            fee/value/approx
  src/config.rs                             Config, AddressesFile, SourceSpec, load_from_dir
  src/classify/mod.rs                       Kind, Rgb, Classification, TxClass, BookEntry, Classifier
  src/classify/builtin.rs                   Builtin (embedded assets)
  src/sources/mod.rs                        SourceError, http helpers, SourceEvent
  src/sources/node.rs                       NodeClient
  src/sources/explorer.rs                   ExplorerClient
  src/sources/addressbook.rs                fetch/parse/cache/snapshot
  src/sources/price.rs                      oracle price
  src/sources/runtime.rs                    poll loops, backoff, spawn_all
  src/reconcile.rs                          Reconciler
  src/packing.rs                            gravity packing
  tests/fixtures/node/*.json
  tests/fixtures/explorer/*.json
crates/ergotop/
  Cargo.toml
  src/main.rs                               CLI
  src/headless.rs                           --headless printer
```

---

### Task 1: Toolchain, workspace scaffold, core model

**Files:**
- Create: `Cargo.toml`, `crates/ergotop-core/Cargo.toml`, `crates/ergotop-core/src/lib.rs`, `crates/ergotop-core/src/model.rs`, `crates/ergotop/Cargo.toml`, `crates/ergotop/src/main.rs`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: nothing.
- Produces (in `ergotop_core::model`):
  - `pub const NANOERG_PER_ERG: u64`, `pub const MINER_REWARD_PREFIX: &str`
  - `pub type TxId = String`
  - `pub struct SourceId(pub String)` (Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)
  - `pub enum SourceKind { Node, Explorer }` (Copy)
  - `pub enum SourceStatus { Unknown, Up, Degraded(String), Down(String) }` with `fn usable(&self) -> bool`
  - `pub struct Token { token_id: String, amount: u64 }`
  - `pub struct BoxData { box_id: String, value: u64, address: String, tokens: Vec<Token> }`
  - `pub struct Input { box_id: String, resolved: Option<BoxData> }`
  - `pub struct Tx { id: TxId, size: u32, inputs: Vec<Input>, outputs: Vec<BoxData>, creation_ts_ms: Option<u64> }`
  - `pub struct BlockRef { id: String, height: u32, timestamp_ms: u64 }`
  - `pub struct Block { id, height: u32, timestamp_ms: u64, size: u32, tx_ids: Vec<TxId>, miner_address: Option<String>, miner_reward: u64 }`
  - `pub struct NodeInfo { full_height: u32, headers_height: u32, peers: u32, app_version: String, max_block_size: u32, indexed_height: Option<u32> }` (Default)
  - `pub struct TokenMeta { token_id: String, name: Option<String>, decimals: u32 }`
  - `pub fn nano_to_erg(n: u64) -> f64`
  - `pub fn find_miner_reward(outputs: &[BoxData]) -> Option<&BoxData>`
  - `#[cfg(test)] pub(crate) mod test_util { pub fn bx(address: &str, value: u64) -> BoxData; pub fn tx(id: &str, size: u32, inputs: Vec<BoxData>, outputs: Vec<BoxData>) -> Tx }`

- [ ] **Step 1: Install the Rust toolchain (Windows)**

Ask the user before installing (system change). Then run in PowerShell:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup -e
```

Open a new shell, then verify:

Run: `cargo --version`
Expected: `cargo 1.8x.x ...` (≥ 1.80)

- [ ] **Step 2: Create the workspace files**

`Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/ergotop-core", "crates/ergotop"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
rust-version = "1.80"

[workspace.dependencies]
anyhow = "1"
blake2 = "0.10"
bs58 = "0.5"
clap = { version = "4", features = ["derive"] }
dirs = "5"
hex = "0.4"
proptest = "1"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tokio = { version = "1", features = ["full"] }
toml = "0.8"
tracing = "0.1"
wiremock = "0.6"
```

`crates/ergotop-core/Cargo.toml`:

```toml
[package]
name = "ergotop-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
blake2.workspace = true
bs58.workspace = true
dirs.workspace = true
hex.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
toml.workspace = true
tracing.workspace = true

[dev-dependencies]
proptest.workspace = true
wiremock.workspace = true
```

`crates/ergotop/Cargo.toml`:

```toml
[package]
name = "ergotop"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
ergotop-core = { path = "../ergotop-core" }
anyhow.workspace = true
clap.workspace = true
tokio.workspace = true
```

`crates/ergotop/src/main.rs`:

```rust
fn main() {
    println!("ergotop {}", env!("CARGO_PKG_VERSION"));
}
```

`crates/ergotop-core/src/lib.rs`:

```rust
//! Data layer for Ergotop: sources, classification, reconciliation, packing.
pub mod model;
```

Append to `.gitignore`:

```
/target
```

- [ ] **Step 3: Write the failing model test**

`crates/ergotop-core/src/model.rs`:

```rust
//! Core data types. All ERG amounts are nanoERG.

pub const NANOERG_PER_ERG: u64 = 1_000_000_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_nanoerg_to_erg() {
        assert_eq!(nano_to_erg(1_500_000_000), 1.5);
        assert_eq!(nano_to_erg(0), 0.0);
    }

    #[test]
    fn finds_miner_reward_output() {
        let outs = vec![
            test_util::bx("2Z4YBkDsDvQj8B", 1_170_924_000_000_000),
            test_util::bx("88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY", 12_000_000_000),
        ];
        let reward = find_miner_reward(&outs).expect("reward output");
        assert_eq!(reward.value, 12_000_000_000);
        assert!(find_miner_reward(&outs[..1]).is_none());
    }

    #[test]
    fn usable_statuses() {
        assert!(SourceStatus::Up.usable());
        assert!(SourceStatus::Degraded("lag".into()).usable());
        assert!(!SourceStatus::Down("x".into()).usable());
        assert!(!SourceStatus::Unknown.usable());
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p ergotop-core model`
Expected: FAIL to compile — `cannot find function nano_to_erg`, `find_miner_reward`, `SourceStatus`, `test_util`.

- [ ] **Step 5: Implement the model**

Insert above the `#[cfg(test)] mod tests` block in `model.rs`:

```rust
/// Miner reward boxes are locked by a P2S contract whose address starts with this.
pub const MINER_REWARD_PREFIX: &str = "88dhgzEuTXa";

pub type TxId = String;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub String);

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Node,
    Explorer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Unknown,
    Up,
    Degraded(String),
    Down(String),
}

impl SourceStatus {
    pub fn usable(&self) -> bool {
        matches!(self, SourceStatus::Up | SourceStatus::Degraded(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub token_id: String,
    pub amount: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxData {
    pub box_id: String,
    pub value: u64,
    pub address: String,
    pub tokens: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub box_id: String,
    pub resolved: Option<BoxData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    pub id: TxId,
    pub size: u32,
    pub inputs: Vec<Input>,
    pub outputs: Vec<BoxData>,
    pub creation_ts_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRef {
    pub id: String,
    pub height: u32,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: String,
    pub height: u32,
    pub timestamp_ms: u64,
    pub size: u32,
    pub tx_ids: Vec<TxId>,
    pub miner_address: Option<String>,
    pub miner_reward: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeInfo {
    pub full_height: u32,
    pub headers_height: u32,
    pub peers: u32,
    pub app_version: String,
    pub max_block_size: u32,
    pub indexed_height: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenMeta {
    pub token_id: String,
    pub name: Option<String>,
    pub decimals: u32,
}

pub fn nano_to_erg(n: u64) -> f64 {
    n as f64 / NANOERG_PER_ERG as f64
}

/// The miner reward box among a block's first (emission) transaction outputs.
pub fn find_miner_reward(outputs: &[BoxData]) -> Option<&BoxData> {
    outputs.iter().find(|o| o.address.starts_with(MINER_REWARD_PREFIX))
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub fn bx(address: &str, value: u64) -> BoxData {
        BoxData {
            box_id: format!("box-{address}-{value}"),
            value,
            address: address.to_string(),
            tokens: vec![],
        }
    }

    /// A tx whose inputs are all resolved to the given boxes.
    pub fn tx(id: &str, size: u32, inputs: Vec<BoxData>, outputs: Vec<BoxData>) -> Tx {
        Tx {
            id: id.to_string(),
            size,
            inputs: inputs
                .into_iter()
                .map(|b| Input { box_id: b.box_id.clone(), resolved: Some(b) })
                .collect(),
            outputs,
            creation_ts_ms: None,
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p ergotop-core model` then `cargo run -p ergotop`
Expected: 3 tests PASS; binary prints `ergotop 0.1.0`.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml .gitignore crates/
git commit -m "feat(core): scaffold Rust workspace and core model

<trailer>"
```

---

### Task 2: ErgoTree ↔ address conversion

**Files:**
- Create: `crates/ergotop-core/src/ergotree.rs`
- Modify: `crates/ergotop-core/src/lib.rs` (add `pub mod ergotree;`)

**Interfaces:**
- Consumes: nothing.
- Produces (`ergotop_core::ergotree`):
  - `pub enum AddressError { Hex, Base58, TooShort, Checksum, Unsupported(u8) }` (Debug, PartialEq, thiserror)
  - `pub fn tree_bytes_to_address(tree: &[u8]) -> String`
  - `pub fn tree_to_address(tree_hex: &str) -> Result<String, AddressError>`
  - `pub fn address_to_tree(address: &str) -> Result<String, AddressError>`

Test vectors (captured from api.ergoplatform.com, 2026-10-03):

| Type | Address | ErgoTree |
|---|---|---|
| P2PK | `9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq` | `0008cd033a8238d69857709e47016aa51b06d0c95d67b8855c23e24c0d8fb26667424e76` |
| P2S (fee) | fee address (Global Constraints) | `1005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304` |
| P2S (short) | `4MQyMKvMbnCJG3aJ` | `10010100d17300` |
| P2S (2Miners reward) | `88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY` | `100204a00b08cd0274e729bb6615cbda94d9d176a2f1525068f12b330e38bbbf387232797dfd891fea02d192a39a8cc7a70173007301` |

- [ ] **Step 1: Write the failing tests**

`crates/ergotop-core/src/ergotree.rs`:

```rust
//! Ergo mainnet ErgoTree <-> address conversion (P2PK and P2S).
//!
//! Address bytes = [network|type] ++ content ++ checksum, where checksum is the
//! first 4 bytes of blake2b256([network|type] ++ content).

#[cfg(test)]
mod tests {
    use super::*;

    const P2PK_ADDR: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const P2PK_TREE: &str = "0008cd033a8238d69857709e47016aa51b06d0c95d67b8855c23e24c0d8fb26667424e76";
    const FEE_ADDR: &str = "2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1bT2Fk4FkFLCfdPpe";
    const FEE_TREE: &str = "1005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304";
    const SHORT_ADDR: &str = "4MQyMKvMbnCJG3aJ";
    const SHORT_TREE: &str = "10010100d17300";
    const POOL_ADDR: &str = "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY";
    const POOL_TREE: &str = "100204a00b08cd0274e729bb6615cbda94d9d176a2f1525068f12b330e38bbbf387232797dfd891fea02d192a39a8cc7a70173007301";

    #[test]
    fn tree_to_address_matches_vectors() {
        assert_eq!(tree_to_address(P2PK_TREE).unwrap(), P2PK_ADDR);
        assert_eq!(tree_to_address(FEE_TREE).unwrap(), FEE_ADDR);
        assert_eq!(tree_to_address(SHORT_TREE).unwrap(), SHORT_ADDR);
        assert_eq!(tree_to_address(POOL_TREE).unwrap(), POOL_ADDR);
    }

    #[test]
    fn address_to_tree_matches_vectors() {
        assert_eq!(address_to_tree(P2PK_ADDR).unwrap(), P2PK_TREE);
        assert_eq!(address_to_tree(FEE_ADDR).unwrap(), FEE_TREE);
        assert_eq!(address_to_tree(SHORT_ADDR).unwrap(), SHORT_TREE);
        assert_eq!(address_to_tree(POOL_ADDR).unwrap(), POOL_TREE);
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut bad = P2PK_ADDR.to_string();
        bad.pop();
        bad.push('r');
        assert_eq!(address_to_tree(&bad), Err(AddressError::Checksum));
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(address_to_tree("0OIl"), Err(AddressError::Base58));
        assert_eq!(address_to_tree("1"), Err(AddressError::TooShort));
        assert_eq!(tree_to_address("zz"), Err(AddressError::Hex));
    }

    #[test]
    fn rejects_p2sh() {
        let mut body = vec![0x02u8];
        body.extend_from_slice(&[0u8; 24]);
        let cs = checksum(&body);
        body.extend_from_slice(&cs);
        let addr = bs58::encode(body).into_string();
        assert_eq!(address_to_tree(&addr), Err(AddressError::Unsupported(0x02)));
    }
}
```

Add `pub mod ergotree;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop-core ergotree`
Expected: FAIL to compile — `tree_to_address`, `address_to_tree`, `AddressError`, `checksum` not found.

- [ ] **Step 3: Implement**

Insert above the tests module:

```rust
use blake2::{digest::consts::U32, Blake2b, Digest};

type Blake2b256 = Blake2b<U32>;

const MAINNET_P2PK: u8 = 0x01;
const MAINNET_P2S: u8 = 0x03;
const P2PK_TREE_PREFIX: [u8; 3] = [0x00, 0x08, 0xcd];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("invalid hex")]
    Hex,
    #[error("invalid base58")]
    Base58,
    #[error("address too short")]
    TooShort,
    #[error("checksum mismatch")]
    Checksum,
    #[error("unsupported address type byte {0:#04x}")]
    Unsupported(u8),
}

fn checksum(body: &[u8]) -> [u8; 4] {
    let digest = Blake2b256::digest(body);
    [digest[0], digest[1], digest[2], digest[3]]
}

pub fn tree_bytes_to_address(tree: &[u8]) -> String {
    let mut body = Vec::with_capacity(tree.len() + 5);
    if tree.len() == 36 && tree[..3] == P2PK_TREE_PREFIX {
        body.push(MAINNET_P2PK);
        body.extend_from_slice(&tree[3..]);
    } else {
        body.push(MAINNET_P2S);
        body.extend_from_slice(tree);
    }
    let cs = checksum(&body);
    body.extend_from_slice(&cs);
    bs58::encode(body).into_string()
}

pub fn tree_to_address(tree_hex: &str) -> Result<String, AddressError> {
    let tree = hex::decode(tree_hex).map_err(|_| AddressError::Hex)?;
    Ok(tree_bytes_to_address(&tree))
}

pub fn address_to_tree(address: &str) -> Result<String, AddressError> {
    let bytes = bs58::decode(address)
        .into_vec()
        .map_err(|_| AddressError::Base58)?;
    if bytes.len() < 5 {
        return Err(AddressError::TooShort);
    }
    let (body, cs) = bytes.split_at(bytes.len() - 4);
    if checksum(body)[..] != *cs {
        return Err(AddressError::Checksum);
    }
    match body[0] {
        MAINNET_P2PK if body.len() == 34 => Ok(format!("0008cd{}", hex::encode(&body[1..]))),
        MAINNET_P2S => Ok(hex::encode(&body[1..])),
        t => Err(AddressError::Unsupported(t)),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop-core ergotree`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop-core/src/ergotree.rs crates/ergotop-core/src/lib.rs
git commit -m "feat(core): ErgoTree <-> address conversion

<trailer>"
```

---

### Task 3: Tx metrics (fee, value, approx)

**Files:**
- Create: `crates/ergotop-core/src/metrics.rs`
- Modify: `crates/ergotop-core/src/lib.rs` (add `pub mod metrics;`)

**Interfaces:**
- Consumes: `model::{Tx, BoxData, Input}`, `model::test_util::{bx, tx}`.
- Produces (`ergotop_core::metrics`):
  - `pub const FEE_ADDRESS: &str`
  - `pub struct TxMetrics { pub fee: u64, pub value: u64, pub approx: bool }` (Clone, Copy, Debug, PartialEq, Eq, Default)
  - `pub fn tx_metrics(tx: &Tx) -> TxMetrics`

Rules (spec §3.3): fee = sum of outputs to `FEE_ADDRESS` (0 if none). If every input is resolved, value = sum of non-fee outputs whose address is not among input addresses, `approx = false`. Otherwise value = sum of all non-fee outputs, `approx = true`.

- [ ] **Step 1: Write the failing tests**

`crates/ergotop-core/src/metrics.rs`:

```rust
//! Per-transaction fee and transferred value.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::test_util::{bx, tx};
    use crate::model::Input;

    const ALICE: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const BOB: &str = "4MQyMKvMbnCJG3aJ";

    #[test]
    fn fee_is_sum_of_fee_outputs() {
        let t = tx("a", 300, vec![bx(ALICE, 10)], vec![bx(BOB, 5), bx(FEE_ADDRESS, 2), bx(FEE_ADDRESS, 1)]);
        assert_eq!(tx_metrics(&t).fee, 3);
    }

    #[test]
    fn no_fee_output_means_zero_fee() {
        let t = tx("a", 300, vec![bx(ALICE, 10)], vec![bx(BOB, 10)]);
        assert_eq!(tx_metrics(&t).fee, 0);
    }

    #[test]
    fn value_excludes_change_and_fee() {
        let t = tx(
            "a",
            300,
            vec![bx(ALICE, 100)],
            vec![bx(BOB, 40), bx(ALICE, 59), bx(FEE_ADDRESS, 1)],
        );
        assert_eq!(tx_metrics(&t), TxMetrics { fee: 1, value: 40, approx: false });
    }

    #[test]
    fn unresolved_inputs_give_approx_value() {
        let mut t = tx("a", 300, vec![], vec![bx(BOB, 40), bx(ALICE, 59), bx(FEE_ADDRESS, 1)]);
        t.inputs.push(Input { box_id: "x".into(), resolved: None });
        assert_eq!(tx_metrics(&t), TxMetrics { fee: 1, value: 99, approx: true });
    }
}
```

Add `pub mod metrics;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop-core metrics`
Expected: FAIL to compile — `FEE_ADDRESS`, `tx_metrics`, `TxMetrics` not found.

- [ ] **Step 3: Implement**

Insert above the tests module:

```rust
use std::collections::HashSet;

use crate::model::Tx;

pub const FEE_ADDRESS: &str = "2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1bT2Fk4FkFLCfdPpe";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TxMetrics {
    pub fee: u64,
    pub value: u64,
    /// True when some inputs are unresolved, so change could not be excluded.
    pub approx: bool,
}

pub fn tx_metrics(tx: &Tx) -> TxMetrics {
    let fee = tx
        .outputs
        .iter()
        .filter(|o| o.address == FEE_ADDRESS)
        .map(|o| o.value)
        .sum();
    let input_addrs: Option<HashSet<&str>> = tx
        .inputs
        .iter()
        .map(|i| i.resolved.as_ref().map(|b| b.address.as_str()))
        .collect();
    let non_fee = tx.outputs.iter().filter(|o| o.address != FEE_ADDRESS);
    match input_addrs {
        Some(ins) => TxMetrics {
            fee,
            value: non_fee
                .filter(|o| !ins.contains(o.address.as_str()))
                .map(|o| o.value)
                .sum(),
            approx: false,
        },
        None => TxMetrics {
            fee,
            value: non_fee.map(|o| o.value).sum(),
            approx: true,
        },
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop-core metrics`
Expected: 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop-core/src/metrics.rs crates/ergotop-core/src/lib.rs
git commit -m "feat(core): accurate fee and value metrics

<trailer>"
```

---

### Task 4: Configuration

**Files:**
- Create: `crates/ergotop-core/src/config.rs`
- Modify: `crates/ergotop-core/src/lib.rs` (add `pub mod config;`)

**Interfaces:**
- Consumes: `model::{SourceId, SourceKind}`.
- Produces (`ergotop_core::config`):
  - consts `DEFAULT_NODE_URL`, `PUBLIC_EXPLORER`, `P2P_EXPLORER`
  - `pub struct Config { pub node: Vec<NodeConfig>, pub explorers: ExplorersConfig, pub ui: UiConfig }` (Default, Deserialize)
  - `pub struct NodeConfig { pub url: String, pub name: Option<String> }`
  - `pub struct ExplorersConfig { pub enabled: Vec<String> }` (default `["p2p","public"]`)
  - `pub struct UiConfig { pub theme: String, pub fps: u32, pub start_view: String }` (default `"neon-green"`, `30`, `"packing"`)
  - `pub struct LocalAddress { pub address: String, pub name: String, pub kind: Option<String>, pub color: Option<String> }`
  - `pub struct AddressesFile { pub address: Vec<LocalAddress> }` (Default)
  - `pub struct SourceSpec { pub id: SourceId, pub kind: SourceKind, pub url: String }`
  - `impl Config { pub fn apply_env(&mut self, node_url: Option<String>, api_url: Option<String>); pub fn sources(&self) -> Vec<SourceSpec> }`
  - `pub fn config_dir() -> Option<PathBuf>`, `pub fn cache_dir() -> Option<PathBuf>`
  - `pub fn load_from_dir(dir: &Path) -> (Config, AddressesFile, Vec<String>)`

- [ ] **Step 1: Write the failing tests**

`crates/ergotop-core/src/config.rs`:

```rust
//! ergotop.toml / addresses.toml loading and source list resolution.

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ergotop-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_full_config() {
        let cfg: Config = toml::from_str(
            r#"
            [[node]]
            url = "http://192.168.1.50:9053/"
            name = "node-a"
            [[node]]
            url = "http://192.168.1.51:9053"
            [explorers]
            enabled = ["public"]
            [ui]
            theme = "amber-terminal"
            fps = 60
            start_view = "dashboard"
            "#,
        )
        .unwrap();
        let ids: Vec<String> = cfg.sources().into_iter().map(|s| s.id.0).collect();
        assert_eq!(ids, vec!["node-a", "http://192.168.1.51:9053", "public"]);
        assert_eq!(cfg.sources()[0].url, "http://192.168.1.50:9053");
        assert_eq!(cfg.ui.fps, 60);
    }

    #[test]
    fn defaults_use_local_node_and_both_explorers() {
        let specs = Config::default().sources();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].id, SourceId("local".into()));
        assert_eq!(specs[0].kind, SourceKind::Node);
        assert_eq!(specs[0].url, DEFAULT_NODE_URL);
        assert_eq!(specs[1].url, P2P_EXPLORER);
        assert_eq!(specs[2].url, PUBLIC_EXPLORER);
        assert_eq!(Config::default().ui.theme, "neon-green");
    }

    #[test]
    fn env_overrides_replace_lists() {
        let mut cfg = Config::default();
        cfg.apply_env(Some("http://10.0.0.5:9053".into()), Some("https://my-explorer.example".into()));
        let specs = cfg.sources();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].url, "http://10.0.0.5:9053");
        assert_eq!(specs[1].kind, SourceKind::Explorer);
        assert_eq!(specs[1].url, "https://my-explorer.example");
    }

    #[test]
    fn load_from_dir_reads_both_files() {
        let dir = temp_dir("both");
        std::fs::write(dir.join("ergotop.toml"), "[[node]]\nurl = \"http://n:9053\"\n").unwrap();
        std::fs::write(
            dir.join("addresses.toml"),
            "[[address]]\naddress = \"9f\"\nname = \"Mine\"\n",
        )
        .unwrap();
        let (cfg, addrs, warnings) = load_from_dir(&dir);
        assert!(warnings.is_empty());
        assert_eq!(cfg.node[0].url, "http://n:9053");
        assert_eq!(addrs.address[0].name, "Mine");
        assert_eq!(addrs.address[0].kind, None);
    }

    #[test]
    fn missing_files_give_defaults_and_bad_file_gives_warning() {
        let dir = temp_dir("bad");
        let (cfg, addrs, warnings) = load_from_dir(&dir);
        assert_eq!(cfg, Config::default());
        assert!(addrs.address.is_empty());
        assert!(warnings.is_empty());

        std::fs::write(dir.join("ergotop.toml"), "this is = = not toml").unwrap();
        let (cfg, _, warnings) = load_from_dir(&dir);
        assert_eq!(cfg, Config::default());
        assert_eq!(warnings.len(), 1);
    }
}
```

Add `pub mod config;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop-core config`
Expected: FAIL to compile — `Config`, `load_from_dir`, etc. not found.

- [ ] **Step 3: Implement**

Insert above the tests module:

```rust
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::model::{SourceId, SourceKind};

pub const DEFAULT_NODE_URL: &str = "http://127.0.0.1:9053";
pub const PUBLIC_EXPLORER: &str = "https://api.ergoplatform.com";
pub const P2P_EXPLORER: &str = "https://api-p2p.ergoplatform.com";

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub node: Vec<NodeConfig>,
    pub explorers: ExplorersConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NodeConfig {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExplorersConfig {
    pub enabled: Vec<String>,
}

impl Default for ExplorersConfig {
    fn default() -> Self {
        Self { enabled: vec!["p2p".into(), "public".into()] }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub fps: u32,
    pub start_view: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { theme: "neon-green".into(), fps: 30, start_view: "packing".into() }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LocalAddress {
    pub address: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AddressesFile {
    pub address: Vec<LocalAddress>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceSpec {
    pub id: SourceId,
    pub kind: SourceKind,
    pub url: String,
}

impl Config {
    pub fn apply_env(&mut self, node_url: Option<String>, api_url: Option<String>) {
        if let Some(url) = node_url {
            self.node = vec![NodeConfig { url, name: None }];
        }
        if let Some(url) = api_url {
            self.explorers.enabled = vec![url];
        }
    }

    /// All sources in priority order: nodes (config order), then explorers.
    pub fn sources(&self) -> Vec<SourceSpec> {
        let nodes = if self.node.is_empty() {
            vec![NodeConfig { url: DEFAULT_NODE_URL.into(), name: Some("local".into()) }]
        } else {
            self.node.clone()
        };
        let mut out: Vec<SourceSpec> = nodes
            .into_iter()
            .map(|n| {
                let url = n.url.trim_end_matches('/').to_string();
                SourceSpec {
                    id: SourceId(n.name.unwrap_or_else(|| url.clone())),
                    kind: SourceKind::Node,
                    url,
                }
            })
            .collect();
        for e in &self.explorers.enabled {
            let (id, url) = match e.as_str() {
                "p2p" => ("p2p", P2P_EXPLORER),
                "public" => ("public", PUBLIC_EXPLORER),
                other => (other, other),
            };
            out.push(SourceSpec {
                id: SourceId(id.to_string()),
                kind: SourceKind::Explorer,
                url: url.trim_end_matches('/').to_string(),
            });
        }
        out
    }
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ergotop"))
}

pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("ergotop"))
}

/// Loads `ergotop.toml` and `addresses.toml` from `dir`.
/// Missing files give defaults; unparsable files give defaults plus a warning.
pub fn load_from_dir(dir: &Path) -> (Config, AddressesFile, Vec<String>) {
    let mut warnings = Vec::new();
    let config = read_toml(&dir.join("ergotop.toml"), &mut warnings);
    let addresses = read_toml(&dir.join("addresses.toml"), &mut warnings);
    (config, addresses, warnings)
}

fn read_toml<T: DeserializeOwned + Default>(path: &Path, warnings: &mut Vec<String>) -> T {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            warnings.push(format!("{}: {e}", path.display()));
            T::default()
        }),
        Err(_) => T::default(),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop-core config`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop-core/src/config.rs crates/ergotop-core/src/lib.rs
git commit -m "feat(core): config files, env overrides, source list

<trailer>"
```

---

### Task 5: Classifier with built-in addresses and rules

**Files:**
- Create: `scripts/migrate_origins.py`, `assets/builtin-addresses.toml` (generated), `assets/rules.toml`, `crates/ergotop-core/src/classify/mod.rs`, `crates/ergotop-core/src/classify/builtin.rs`
- Modify: `crates/ergotop-core/src/lib.rs` (add `pub mod classify;`)

**Interfaces:**
- Consumes: `config::LocalAddress`, `metrics::FEE_ADDRESS`, `model::{Tx, BoxData}`, `model::test_util`.
- Produces (`ergotop_core::classify`):
  - `pub enum Kind { Exchange, Service, MiningPool, Meme, Local, P2P, Contract, Unknown }` (Copy, Debug, PartialEq, Eq, Hash) with `pub fn parse(s: &str) -> Kind` and `pub fn label(self) -> &'static str`
  - `pub struct Rgb(pub u8, pub u8, pub u8)` (Copy, Debug, PartialEq, Eq) with `pub fn parse_hex(s: &str) -> Option<Rgb>`
  - `pub struct Classification { pub name: String, pub kind: Kind, pub color: Rgb }` (Clone, Debug, PartialEq)
  - `pub struct TxClass { pub class: Classification, pub from: Option<String> }` (Clone, Debug, PartialEq)
  - `pub struct BookEntry { pub address: String, pub name: String, pub kind: Kind }` (Clone, Debug, PartialEq)
  - `pub struct Classifier` with `pub fn new(builtin: &Builtin, book: &[BookEntry], local: &[LocalAddress]) -> Self`, `pub fn lookup(&self, address: &str) -> Option<Classification>`, `pub fn classify_tx(&self, tx: &Tx) -> TxClass`
  - `pub use builtin::{Builtin, Rule};`
  - `builtin::Builtin { pub addresses: Vec<LocalAddress>, pub colors: HashMap<String, String>, pub rules: Vec<Rule> }` (Default) with `pub fn load() -> Builtin`
  - `builtin::Rule { pub name: String, pub kind: String, pub address_prefix: String }`

Lookup order (spec §3.4): local → address book → built-in → prefix rules. Implemented by inserting built-in, then book, then local into one map (later overwrite earlier), then rules on a miss. Color: entry color (local) → `colors` override by name → kind color shaded by a stable FNV-1a hash of the name.

- [ ] **Step 1: Generate built-in addresses from the Python tables**

`scripts/migrate_origins.py`:

```python
"""One-off: export the Python Ergotop origin tables to assets/builtin-addresses.toml.

Run from the repo root: python scripts/migrate_origins.py
"""
import sys

sys.path.insert(0, ".")
from ergo_mempool_tui.config import MINING_POOLS  # noqa: E402
from ergo_mempool_tui.detection.origins import CONTRACT_ADDRESSES, PLATFORMS  # noqa: E402

EXCHANGES = {"KUCOIN", "NONKYC", "GATE", "MEXC", "HUOBI", "COINEX", "TRADEOGRE", "XEGGEX", "PROBIT"}

lines = ["# Generated by scripts/migrate_origins.py from the Python Ergotop origin tables.", ""]
colors = {}
for addr, key in sorted(CONTRACT_ADDRESSES.items(), key=lambda kv: (kv[1], kv[0])):
    name, color = PLATFORMS[key]
    kind = "Exchange" if key in EXCHANGES else ("MiningPool" if key == "MININGPOOL" else "Service")
    lines += ["[[address]]", f'address = "{addr}"', f'name = "{name}"', f'kind = "{kind}"', ""]
    colors[name] = color
for addr, name in sorted(MINING_POOLS.items(), key=lambda kv: kv[1]):
    lines += ["[[address]]", f'address = "{addr}"', f'name = "{name}"', 'kind = "MiningPool"', ""]
lines.append("[colors]")
for name, color in sorted(colors.items()):
    lines.append(f'"{name}" = "{color}"')
with open("assets/builtin-addresses.toml", "w", newline="\n", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
print(len(CONTRACT_ADDRESSES) + len(MINING_POOLS), "addresses written")
```

Run: `mkdir -p assets && python scripts/migrate_origins.py`
Expected: `155 addresses written` (144 contract + 11 pool) and `assets/builtin-addresses.toml` exists, starting with `[[address]]` blocks and ending with a `[colors]` table.

- [ ] **Step 2: Write the rules file**

`assets/rules.toml` (prefixes copied verbatim from `ADDRESS_PREFIXES` in `ergo_mempool_tui/detection/origins.py`):

```toml
# Address-prefix rules for contracts that share a template but have no fixed address.

[[rule]]
name = "Spectrum"
kind = "Service"
address_prefix = "MUbV38YgqHy7XbsoXWF5z7EZm524Ybdwe5p9WDerbecviBcYLtRiXcqtyf55992zu5jbRFPqSobanaHpUpEFBEGio"

[[rule]]
name = "SkyHarbor"
kind = "Service"
address_prefix = "K9LerzPNHBdMqEjMiWwBMfbxQmxL8gDVDXTBZJJCDCVFGXCGCyEer"

[[rule]]
name = "Rosen Bridge"
kind = "Service"
address_prefix = "4L1ktFnzjG"
```

- [ ] **Step 3: Write the failing tests**

`crates/ergotop-core/src/classify/builtin.rs`:

```rust
//! Built-in classification data embedded at compile time.
use std::collections::HashMap;

use serde::Deserialize;

use crate::config::LocalAddress;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Rule {
    pub name: String,
    pub kind: String,
    pub address_prefix: String,
}

#[derive(Debug, Clone, Default)]
pub struct Builtin {
    pub addresses: Vec<LocalAddress>,
    pub colors: HashMap<String, String>,
    pub rules: Vec<Rule>,
}

#[derive(Deserialize)]
struct AddressesToml {
    #[serde(default)]
    address: Vec<LocalAddress>,
    #[serde(default)]
    colors: HashMap<String, String>,
}

#[derive(Deserialize)]
struct RulesToml {
    #[serde(default)]
    rule: Vec<Rule>,
}

const ADDRESSES_TOML: &str = include_str!("../../../../assets/builtin-addresses.toml");
const RULES_TOML: &str = include_str!("../../../../assets/rules.toml");

impl Builtin {
    /// Parses the embedded assets. Panics only if the shipped assets are malformed (covered by tests).
    pub fn load() -> Builtin {
        let a: AddressesToml = toml::from_str(ADDRESSES_TOML).expect("assets/builtin-addresses.toml");
        let r: RulesToml = toml::from_str(RULES_TOML).expect("assets/rules.toml");
        Builtin { addresses: a.address, colors: a.colors, rules: r.rule }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_parse() {
        let b = Builtin::load();
        assert!(b.addresses.len() >= 150, "got {}", b.addresses.len());
        assert_eq!(b.rules.len(), 3);
        assert_eq!(b.colors.get("Spectrum").map(String::as_str), Some("#3498db"));
    }
}
```

`crates/ergotop-core/src/classify/mod.rs` (tests first; implementation in Step 5):

```rust
//! Address and transaction classification.
mod builtin;

pub use builtin::{Builtin, Rule};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LocalAddress;
    use crate::metrics::FEE_ADDRESS;
    use crate::model::test_util::{bx, tx};

    const WALLET_A: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";
    const WALLET_B: &str = "9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1";
    const CONTRACT: &str = "4MQyMKvMbnCJG3aJ";

    fn local(address: &str, name: &str) -> LocalAddress {
        LocalAddress { address: address.into(), name: name.into(), kind: None, color: None }
    }

    fn book(address: &str, name: &str, kind: Kind) -> BookEntry {
        BookEntry { address: address.into(), name: name.into(), kind }
    }

    #[test]
    fn local_beats_book_beats_builtin() {
        let builtin = Builtin { addresses: vec![local(WALLET_A, "Builtin")], ..Default::default() };
        let c = Classifier::new(&builtin, &[], &[]);
        assert_eq!(c.lookup(WALLET_A).unwrap().name, "Builtin");

        let books = [book(WALLET_A, "Book", Kind::Exchange)];
        let c = Classifier::new(&builtin, &books, &[]);
        let hit = c.lookup(WALLET_A).unwrap();
        assert_eq!((hit.name.as_str(), hit.kind), ("Book", Kind::Exchange));

        let c = Classifier::new(&builtin, &books, &[local(WALLET_A, "Mine")]);
        let hit = c.lookup(WALLET_A).unwrap();
        assert_eq!((hit.name.as_str(), hit.kind), ("Mine", Kind::Local));
    }

    #[test]
    fn prefix_rules_match_after_exact_miss() {
        let builtin = Builtin {
            rules: vec![Rule { name: "Spectrum".into(), kind: "Service".into(), address_prefix: "4MQy".into() }],
            ..Default::default()
        };
        let c = Classifier::new(&builtin, &[], &[]);
        assert_eq!(c.lookup(CONTRACT).unwrap().name, "Spectrum");
        assert!(c.lookup(WALLET_A).is_none());
    }

    #[test]
    fn tx_uses_output_match_and_reports_input_side() {
        let books = [book(WALLET_A, "Kucoin", Kind::Exchange), book(CONTRACT, "Spectrum", Kind::Service)];
        let c = Classifier::new(&Builtin::default(), &books, &[]);
        let t = tx("t", 300, vec![bx(WALLET_A, 10)], vec![bx(CONTRACT, 9), bx(FEE_ADDRESS, 1)]);
        let tc = c.classify_tx(&t);
        assert_eq!(tc.class.name, "Spectrum");
        assert_eq!(tc.from.as_deref(), Some("Kucoin"));
    }

    #[test]
    fn tx_falls_back_to_input_match() {
        let books = [book(WALLET_A, "Kucoin", Kind::Exchange)];
        let c = Classifier::new(&Builtin::default(), &books, &[]);
        let t = tx("t", 300, vec![bx(WALLET_A, 10)], vec![bx(WALLET_B, 9), bx(FEE_ADDRESS, 1)]);
        let tc = c.classify_tx(&t);
        assert_eq!(tc.class.name, "Kucoin");
        assert_eq!(tc.from, None);
    }

    #[test]
    fn heuristics_p2p_contract_unknown() {
        let c = Classifier::new(&Builtin::default(), &[], &[]);
        let p2p = tx("t", 1, vec![], vec![bx(WALLET_B, 9), bx(FEE_ADDRESS, 1)]);
        assert_eq!(c.classify_tx(&p2p).class.kind, Kind::P2P);
        let contract = tx("t", 1, vec![], vec![bx(WALLET_B, 9), bx(CONTRACT, 1)]);
        assert_eq!(c.classify_tx(&contract).class.kind, Kind::Contract);
        let only_fee = tx("t", 1, vec![], vec![bx(FEE_ADDRESS, 1)]);
        assert_eq!(c.classify_tx(&only_fee).class.kind, Kind::Unknown);
    }

    #[test]
    fn colors_prefer_entry_then_override_then_kind_shade() {
        let mut builtin = Builtin::default();
        builtin.colors.insert("Spectrum".into(), "#3498db".into());
        let mut mine = local(WALLET_B, "Mine");
        mine.color = Some("#ff00ff".into());
        let books = [book(CONTRACT, "Spectrum", Kind::Service), book(WALLET_A, "Other", Kind::Service)];
        let c = Classifier::new(&builtin, &books, &[mine]);
        assert_eq!(c.lookup(WALLET_B).unwrap().color, Rgb(0xff, 0x00, 0xff));
        assert_eq!(c.lookup(CONTRACT).unwrap().color, Rgb(0x34, 0x98, 0xdb));
        let shade = c.lookup(WALLET_A).unwrap().color;
        assert_eq!(shade, c.lookup(WALLET_A).unwrap().color, "stable across calls");
    }

    #[test]
    fn parses_kinds_and_hex() {
        assert_eq!(Kind::parse("Mining pool"), Kind::MiningPool);
        assert_eq!(Kind::parse("Exchange"), Kind::Exchange);
        assert_eq!(Kind::parse("Meme"), Kind::Meme);
        assert_eq!(Kind::parse("something new"), Kind::Service);
        assert_eq!(Rgb::parse_hex("#0a0B0c"), Some(Rgb(10, 11, 12)));
        assert_eq!(Rgb::parse_hex("nope"), None);
    }
}
```

Add `pub mod classify;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p ergotop-core classify`
Expected: FAIL to compile — `Kind`, `Rgb`, `Classifier`, `BookEntry` not found.

- [ ] **Step 5: Implement the classifier**

Insert in `classify/mod.rs` after `pub use builtin::{Builtin, Rule};`:

```rust
use std::collections::HashMap;

use crate::config::LocalAddress;
use crate::metrics::FEE_ADDRESS;
use crate::model::{BoxData, Tx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Exchange,
    Service,
    MiningPool,
    Meme,
    Local,
    P2P,
    Contract,
    Unknown,
}

impl Kind {
    /// Parses ergexplorer `type` values and our own TOML `kind` values.
    pub fn parse(s: &str) -> Kind {
        match s.trim().to_ascii_lowercase().replace(' ', "").as_str() {
            "exchange" => Kind::Exchange,
            "miningpool" => Kind::MiningPool,
            "meme" => Kind::Meme,
            "local" => Kind::Local,
            _ => Kind::Service,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Exchange => "Exchange",
            Kind::Service => "Service",
            Kind::MiningPool => "Mining pool",
            Kind::Meme => "Meme",
            Kind::Local => "Local",
            Kind::P2P => "P2P",
            Kind::Contract => "Contract",
            Kind::Unknown => "Unknown",
        }
    }

    fn base_color(self) -> Rgb {
        match self {
            Kind::Exchange => Rgb(0xf3, 0x9c, 0x12),
            Kind::Service => Rgb(0x34, 0x98, 0xdb),
            Kind::MiningPool => Rgb(0x8b, 0x45, 0x13),
            Kind::Meme => Rgb(0xe9, 0x1e, 0x63),
            Kind::Local => Rgb(0xf1, 0xc4, 0x0f),
            Kind::P2P => Rgb(0x7a, 0xb8, 0x7a),
            Kind::Contract => Rgb(0x8a, 0x6a, 0x9a),
            Kind::Unknown => Rgb(0x55, 0x66, 0x55),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse_hex(s: &str) -> Option<Rgb> {
        let h = s.strip_prefix('#')?;
        if h.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    fn shift(self, delta: i16) -> Rgb {
        let f = |c: u8| (c as i16 + delta).clamp(0, 255) as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Classification {
    pub name: String,
    pub kind: Kind,
    pub color: Rgb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TxClass {
    pub class: Classification,
    /// Input-side name when it differs from the output-side match (`from → class`).
    pub from: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BookEntry {
    pub address: String,
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    kind: Kind,
    color: Option<Rgb>,
}

pub struct Classifier {
    exact: HashMap<String, Entry>,
    rules: Vec<(String, Entry)>,
    colors: HashMap<String, Rgb>,
}

fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

fn local_entry(a: &LocalAddress, default_kind: Kind) -> Entry {
    Entry {
        name: a.name.clone(),
        kind: a.kind.as_deref().map(Kind::parse).unwrap_or(default_kind),
        color: a.color.as_deref().and_then(Rgb::parse_hex),
    }
}

fn is_p2pk(address: &str) -> bool {
    address.starts_with('9') && address.len() == 51
}

impl Classifier {
    pub fn new(builtin: &Builtin, book: &[BookEntry], local: &[LocalAddress]) -> Self {
        let mut exact = HashMap::new();
        for a in &builtin.addresses {
            exact.insert(a.address.clone(), local_entry(a, Kind::Service));
        }
        for b in book {
            exact.insert(b.address.clone(), Entry { name: b.name.clone(), kind: b.kind, color: None });
        }
        for a in local {
            exact.insert(a.address.clone(), local_entry(a, Kind::Local));
        }
        let rules = builtin
            .rules
            .iter()
            .map(|r| {
                (r.address_prefix.clone(), Entry { name: r.name.clone(), kind: Kind::parse(&r.kind), color: None })
            })
            .collect();
        let colors = builtin
            .colors
            .iter()
            .filter_map(|(name, hex)| Rgb::parse_hex(hex).map(|c| (name.clone(), c)))
            .collect();
        Classifier { exact, rules, colors }
    }

    fn finish(&self, e: &Entry) -> Classification {
        let color = e
            .color
            .or_else(|| self.colors.get(&e.name).copied())
            .unwrap_or_else(|| e.kind.base_color().shift((fnv1a(&e.name) % 81) as i16 - 40));
        Classification { name: e.name.clone(), kind: e.kind, color }
    }

    pub fn lookup(&self, address: &str) -> Option<Classification> {
        if let Some(e) = self.exact.get(address) {
            return Some(self.finish(e));
        }
        self.rules
            .iter()
            .find(|(prefix, _)| address.starts_with(prefix.as_str()))
            .map(|(_, e)| self.finish(e))
    }

    pub fn classify_tx(&self, tx: &Tx) -> TxClass {
        let outs: Vec<&BoxData> = tx.outputs.iter().filter(|o| o.address != FEE_ADDRESS).collect();
        let out_named = outs.iter().find_map(|o| self.lookup(&o.address));
        let in_named = tx
            .inputs
            .iter()
            .filter_map(|i| i.resolved.as_ref())
            .find_map(|b| self.lookup(&b.address));
        match (out_named, in_named) {
            (Some(o), Some(i)) if o.name != i.name => TxClass { class: o, from: Some(i.name) },
            (Some(o), _) => TxClass { class: o, from: None },
            (None, Some(i)) => TxClass { class: i, from: None },
            (None, None) => TxClass { class: heuristic(&outs), from: None },
        }
    }
}

fn heuristic(outs: &[&BoxData]) -> Classification {
    let kind = if outs.is_empty() {
        Kind::Unknown
    } else if outs.iter().all(|o| is_p2pk(&o.address)) {
        Kind::P2P
    } else {
        Kind::Contract
    };
    Classification { name: kind.label().to_string(), kind, color: kind.base_color() }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p ergotop-core classify`
Expected: 8 tests PASS (7 in `classify::tests`, 1 in `classify::builtin::tests`).

- [ ] **Step 7: Commit**

```bash
git add scripts/migrate_origins.py assets/ crates/ergotop-core/src/classify crates/ergotop-core/src/lib.rs
git commit -m "feat(core): classifier with local/book/builtin/rule precedence

<trailer>"
```

---

### Task 6: Node client

**Files:**
- Create: `crates/ergotop-core/src/sources/mod.rs`, `crates/ergotop-core/src/sources/node.rs`, fixtures under `crates/ergotop-core/tests/fixtures/node/`
- Modify: `crates/ergotop-core/src/lib.rs` (add `pub mod sources;`)

**Interfaces:**
- Consumes: `ergotree::tree_to_address`, `model::*`.
- Produces:
  - `sources::SourceError { Http(reqwest::Error), Status(u16), Parse(String) }`, `sources::Result<T>`
  - `sources::http_client() -> reqwest::Client` (3s timeout)
  - `pub(crate) async fn get_json<T: DeserializeOwned>(http: &Client, url: &str) -> Result<T>`
  - `pub(crate) async fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(http: &Client, url: &str, body: &B) -> Result<T>`
  - `sources::node::NodeClient` with:
    - `pub fn new(http: Client, base: &str) -> Self`
    - `pub async fn info(&self) -> Result<NodeInfo>`
    - `pub async fn mempool_ids(&self) -> Result<Vec<TxId>>`
    - `pub async fn mempool_txs(&self, ids: &[TxId]) -> Result<Vec<Tx>>`
    - `pub async fn last_headers(&self, n: u32) -> Result<Vec<BlockRef>>`
    - `pub async fn block(&self, header: &BlockRef) -> Result<Block>`
    - `pub async fn token(&self, token_id: &str) -> Result<TokenMeta>`

Endpoints (spec §3.1): `GET /info`, `GET /blockchain/indexedHeight` (optional), `GET /transactions/unconfirmed/transactionIds`, `POST /transactions/unconfirmed/byTransactionIds` (fallback: `GET /transactions/unconfirmed/byTransactionId/{id}` per id), `POST /utxo/withPool/byIds` (chunks of 100), `GET /blocks/lastHeaders/{n}`, `GET /blocks/{headerId}/transactions`, `GET /blockchain/token/byId/{id}`.

- [ ] **Step 1: Create node fixtures**

`crates/ergotop-core/tests/fixtures/node/info.json`:

```json
{"name":"node-a","appVersion":"6.0.1","fullHeight":1886101,"headersHeight":1886101,"maxPeerHeight":1886101,"stateType":"utxo","peersCount":31,"unconfirmedCount":2,"parameters":{"height":1886101,"blockVersion":4,"storageFeeFactor":1250000,"minValuePerByte":360,"maxBlockSize":1271009,"maxBlockCost":8001091,"tokenAccessCost":100,"inputCost":2407,"dataInputCost":100,"outputCost":197}}
```

`.../node/indexed_height.json`:

```json
{"indexedHeight":1886100,"fullHeight":1886101}
```

`.../node/mempool_ids.json`:

```json
["tx-a","tx-b"]
```

`.../node/mempool_txs.json`:

```json
[
  {"id":"tx-a","inputs":[{"boxId":"in-a","spendingProof":{"proofBytes":"","extension":{}}}],"dataInputs":[],
   "outputs":[
     {"boxId":"out-a1","value":11887500000,"ergoTree":"0008cd033a8238d69857709e47016aa51b06d0c95d67b8855c23e24c0d8fb26667424e76","creationHeight":1886100,"assets":[{"tokenId":"tok-1","amount":5}],"additionalRegisters":{},"transactionId":"tx-a","index":0},
     {"boxId":"out-a2","value":1500000,"ergoTree":"1005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304","creationHeight":1886100,"assets":[],"additionalRegisters":{},"transactionId":"tx-a","index":1}
   ],"size":412},
  {"id":"tx-b","inputs":[{"boxId":"in-b","spendingProof":{"proofBytes":"","extension":{}}}],"dataInputs":[],
   "outputs":[
     {"boxId":"out-b1","value":5000000000,"ergoTree":"10010100d17300","creationHeight":1886100,"assets":[],"additionalRegisters":{},"transactionId":"tx-b","index":0}
   ],"size":300}
]
```

`.../node/tx_a.json`: the first element of the array above as a standalone object (copy the `{"id":"tx-a", ... "size":412}` object exactly).

`.../node/boxes.json`:

```json
[
  {"boxId":"in-a","value":11889000000,"ergoTree":"0008cd021111111111111111111111111111111111111111111111111111111111111111","creationHeight":1886000,"assets":[],"additionalRegisters":{},"transactionId":"prev-a","index":0},
  {"boxId":"in-b","value":5001000000,"ergoTree":"0008cd032222222222222222222222222222222222222222222222222222222222222222","creationHeight":1886000,"assets":[],"additionalRegisters":{},"transactionId":"prev-b","index":0}
]
```

`.../node/last_headers.json`:

```json
[
  {"id":"hdr-0","height":1886100,"timestamp":1790977963213,"version":4,"parentId":"hdr-x","size":220},
  {"id":"hdr-1","height":1886101,"timestamp":1790978083213,"version":4,"parentId":"hdr-0","size":221}
]
```

`.../node/block_txs.json`:

```json
{"headerId":"hdr-1","size":187236,"transactions":[
  {"id":"cb-1","inputs":[{"boxId":"emission","spendingProof":{"proofBytes":"","extension":{}}}],"dataInputs":[],
   "outputs":[
     {"boxId":"em-2","value":1170924000000000,"ergoTree":"10010100d17300","creationHeight":1886101,"assets":[],"additionalRegisters":{},"transactionId":"cb-1","index":0},
     {"boxId":"rw-1","value":12000000000,"ergoTree":"100204a00b08cd0274e729bb6615cbda94d9d176a2f1525068f12b330e38bbbf387232797dfd891fea02d192a39a8cc7a70173007301","creationHeight":1886101,"assets":[],"additionalRegisters":{},"transactionId":"cb-1","index":1}
   ],"size":200},
  {"id":"tx-a","inputs":[],"dataInputs":[],"outputs":[],"size":412}
]}
```

`.../node/token.json`:

```json
{"id":"tok-1","boxId":"tb","emissionAmount":100000000,"name":"SigUSD","description":"","decimals":2}
```

- [ ] **Step 2: Write `sources/mod.rs`**

```rust
//! Data sources: node, explorers, address book, price, and their poll loops.
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod node;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("http status {0}")]
    Status(u16),
    #[error("parse: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, SourceError>;

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent(concat!("ergotop/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

async fn read_json<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    if !resp.status().is_success() {
        return Err(SourceError::Status(resp.status().as_u16()));
    }
    let text = resp.text().await?;
    serde_json::from_str(&text).map_err(|e| SourceError::Parse(e.to_string()))
}

pub(crate) async fn get_json<T: DeserializeOwned>(http: &reqwest::Client, url: &str) -> Result<T> {
    read_json(http.get(url).send().await?).await
}

pub(crate) async fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    body: &B,
) -> Result<T> {
    read_json(http.post(url).json(body).send().await?).await
}
```

Add `pub mod sources;` to `lib.rs`.

- [ ] **Step 3: Write the failing tests**

`crates/ergotop-core/src/sources/node.rs`:

```rust
//! Ergo node REST client.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ergotree::tree_to_address;
    use crate::sources::http_client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const INFO: &str = include_str!("../../tests/fixtures/node/info.json");
    const INDEXED: &str = include_str!("../../tests/fixtures/node/indexed_height.json");
    const IDS: &str = include_str!("../../tests/fixtures/node/mempool_ids.json");
    const TXS: &str = include_str!("../../tests/fixtures/node/mempool_txs.json");
    const TX_A: &str = include_str!("../../tests/fixtures/node/tx_a.json");
    const BOXES: &str = include_str!("../../tests/fixtures/node/boxes.json");
    const HEADERS: &str = include_str!("../../tests/fixtures/node/last_headers.json");
    const BLOCK: &str = include_str!("../../tests/fixtures/node/block_txs.json");
    const TOKEN: &str = include_str!("../../tests/fixtures/node/token.json");

    async fn mock(server: &MockServer, verb: &str, p: &str, status: u16, body: &str) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn info_includes_indexed_height_and_block_size() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/info", 200, INFO).await;
        mock(&s, "GET", "/blockchain/indexedHeight", 200, INDEXED).await;
        let info = NodeClient::new(http_client(), &s.uri()).info().await.unwrap();
        assert_eq!(info.full_height, 1886101);
        assert_eq!(info.max_block_size, 1271009);
        assert_eq!(info.peers, 31);
        assert_eq!(info.app_version, "6.0.1");
        assert_eq!(info.indexed_height, Some(1886100));
    }

    #[tokio::test]
    async fn info_without_index_is_not_indexed() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/info", 200, INFO).await;
        mock(&s, "GET", "/blockchain/indexedHeight", 404, "{}").await;
        let info = NodeClient::new(http_client(), &s.uri()).info().await.unwrap();
        assert_eq!(info.indexed_height, None);
    }

    #[tokio::test]
    async fn mempool_txs_resolve_inputs_and_addresses() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/transactions/unconfirmed/transactionIds", 200, IDS).await;
        mock(&s, "POST", "/transactions/unconfirmed/byTransactionIds", 200, TXS).await;
        mock(&s, "POST", "/utxo/withPool/byIds", 200, BOXES).await;
        let c = NodeClient::new(http_client(), &s.uri());
        let ids = c.mempool_ids().await.unwrap();
        assert_eq!(ids, vec!["tx-a", "tx-b"]);
        let txs = c.mempool_txs(&ids).await.unwrap();
        assert_eq!(txs.len(), 2);
        let a = &txs[0];
        assert_eq!(a.size, 412);
        assert_eq!(a.outputs[0].address, "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq");
        assert_eq!(a.outputs[1].address, crate::metrics::FEE_ADDRESS);
        assert_eq!(a.outputs[0].tokens[0].token_id, "tok-1");
        let resolved = a.inputs[0].resolved.as_ref().expect("input resolved");
        assert_eq!(resolved.value, 11889000000);
        assert_eq!(
            resolved.address,
            tree_to_address("0008cd021111111111111111111111111111111111111111111111111111111111111111").unwrap()
        );
        assert_eq!(txs[1].outputs[0].address, "4MQyMKvMbnCJG3aJ");
    }

    #[tokio::test]
    async fn mempool_txs_fall_back_to_single_gets() {
        let s = MockServer::start().await;
        mock(&s, "POST", "/transactions/unconfirmed/byTransactionIds", 500, "").await;
        mock(&s, "GET", "/transactions/unconfirmed/byTransactionId/tx-a", 200, TX_A).await;
        mock(&s, "POST", "/utxo/withPool/byIds", 404, "{}").await;
        let c = NodeClient::new(http_client(), &s.uri());
        let txs = c.mempool_txs(&["tx-a".to_string()]).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id, "tx-a");
        assert!(txs[0].inputs[0].resolved.is_none(), "box lookup failed, input left unresolved");
    }

    #[tokio::test]
    async fn empty_id_list_makes_no_requests() {
        let c = NodeClient::new(http_client(), "http://127.0.0.1:9");
        assert!(c.mempool_txs(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn block_has_tx_ids_and_miner() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/blocks/lastHeaders/2", 200, HEADERS).await;
        mock(&s, "GET", "/blocks/hdr-1/transactions", 200, BLOCK).await;
        let c = NodeClient::new(http_client(), &s.uri());
        let headers = c.last_headers(2).await.unwrap();
        assert_eq!(headers[1], BlockRef { id: "hdr-1".into(), height: 1886101, timestamp_ms: 1790978083213 });
        let b = c.block(&headers[1]).await.unwrap();
        assert_eq!(b.height, 1886101);
        assert_eq!(b.size, 187236);
        assert_eq!(b.tx_ids, vec!["cb-1", "tx-a"]);
        assert_eq!(
            b.miner_address.as_deref(),
            Some("88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY")
        );
        assert_eq!(b.miner_reward, 12000000000);
    }

    #[tokio::test]
    async fn token_meta() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/blockchain/token/byId/tok-1", 200, TOKEN).await;
        let t = NodeClient::new(http_client(), &s.uri()).token("tok-1").await.unwrap();
        assert_eq!(t, TokenMeta { token_id: "tok-1".into(), name: Some("SigUSD".into()), decimals: 2 });
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p ergotop-core sources::node`
Expected: FAIL to compile — `NodeClient` not found.

- [ ] **Step 5: Implement `NodeClient`**

Insert above the tests module in `node.rs`:

```rust
use std::collections::HashMap;

use serde::Deserialize;

use super::{get_json, post_json, Result};
use crate::ergotree::tree_to_address;
use crate::model::{find_miner_reward, Block, BlockRef, BoxData, Input, NodeInfo, Token, TokenMeta, Tx, TxId};

const BOX_CHUNK: usize = 100;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfoJson {
    #[serde(default)]
    full_height: Option<u32>,
    #[serde(default)]
    headers_height: Option<u32>,
    #[serde(default)]
    peers_count: Option<u32>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    parameters: Option<ParamsJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParamsJson {
    max_block_size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexedHeightJson {
    indexed_height: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutputJson {
    box_id: String,
    value: u64,
    ergo_tree: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetJson {
    token_id: String,
    amount: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputJson {
    box_id: String,
}

#[derive(Deserialize)]
struct TxJson {
    id: String,
    inputs: Vec<InputJson>,
    outputs: Vec<OutputJson>,
    #[serde(default)]
    size: Option<u32>,
}

#[derive(Deserialize)]
struct HeaderJson {
    id: String,
    height: u32,
    timestamp: u64,
}

#[derive(Deserialize)]
struct BlockTxsJson {
    transactions: Vec<TxJson>,
    #[serde(default)]
    size: u32,
}

#[derive(Deserialize)]
struct TokenJson {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    decimals: Option<u32>,
}

fn to_box(o: OutputJson) -> BoxData {
    BoxData {
        address: tree_to_address(&o.ergo_tree).unwrap_or_default(),
        box_id: o.box_id,
        value: o.value,
        tokens: o
            .assets
            .into_iter()
            .map(|a| Token { token_id: a.token_id, amount: a.amount })
            .collect(),
    }
}

fn to_tx(t: TxJson, resolved: &HashMap<String, BoxData>) -> Tx {
    Tx {
        id: t.id,
        size: t.size.unwrap_or(0),
        inputs: t
            .inputs
            .into_iter()
            .map(|i| Input { resolved: resolved.get(&i.box_id).cloned(), box_id: i.box_id })
            .collect(),
        outputs: t.outputs.into_iter().map(to_box).collect(),
        creation_ts_ms: None,
    }
}

pub struct NodeClient {
    http: reqwest::Client,
    base: String,
}

impl NodeClient {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').to_string() }
    }

    fn url(&self, p: &str) -> String {
        format!("{}{}", self.base, p)
    }

    pub async fn info(&self) -> Result<NodeInfo> {
        let i: InfoJson = get_json(&self.http, &self.url("/info")).await?;
        let indexed_height = get_json::<IndexedHeightJson>(&self.http, &self.url("/blockchain/indexedHeight"))
            .await
            .ok()
            .map(|h| h.indexed_height);
        Ok(NodeInfo {
            full_height: i.full_height.unwrap_or(0),
            headers_height: i.headers_height.unwrap_or(0),
            peers: i.peers_count.unwrap_or(0),
            app_version: i.app_version.unwrap_or_default(),
            max_block_size: i.parameters.map(|p| p.max_block_size).unwrap_or(0),
            indexed_height,
        })
    }

    pub async fn mempool_ids(&self) -> Result<Vec<TxId>> {
        get_json(&self.http, &self.url("/transactions/unconfirmed/transactionIds")).await
    }

    pub async fn mempool_txs(&self, ids: &[TxId]) -> Result<Vec<Tx>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let raw: Vec<TxJson> =
            match post_json(&self.http, &self.url("/transactions/unconfirmed/byTransactionIds"), ids).await {
                Ok(v) => v,
                Err(_) => {
                    let mut v = Vec::new();
                    for id in ids {
                        let url = self.url(&format!("/transactions/unconfirmed/byTransactionId/{id}"));
                        if let Ok(t) = get_json::<TxJson>(&self.http, &url).await {
                            v.push(t);
                        }
                    }
                    v
                }
            };
        let box_ids: Vec<String> = raw.iter().flat_map(|t| t.inputs.iter().map(|i| i.box_id.clone())).collect();
        let resolved = self.boxes(&box_ids).await;
        Ok(raw.into_iter().map(|t| to_tx(t, &resolved)).collect())
    }

    /// Input boxes from UTXO set + mempool. Failures leave inputs unresolved.
    async fn boxes(&self, ids: &[String]) -> HashMap<String, BoxData> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(BOX_CHUNK) {
            if let Ok(found) = post_json::<_, Vec<OutputJson>>(&self.http, &self.url("/utxo/withPool/byIds"), chunk).await {
                for o in found {
                    let b = to_box(o);
                    out.insert(b.box_id.clone(), b);
                }
            }
        }
        out
    }

    pub async fn last_headers(&self, n: u32) -> Result<Vec<BlockRef>> {
        let hs: Vec<HeaderJson> = get_json(&self.http, &self.url(&format!("/blocks/lastHeaders/{n}"))).await?;
        Ok(hs
            .into_iter()
            .map(|h| BlockRef { id: h.id, height: h.height, timestamp_ms: h.timestamp })
            .collect())
    }

    pub async fn block(&self, header: &BlockRef) -> Result<Block> {
        let b: BlockTxsJson = get_json(&self.http, &self.url(&format!("/blocks/{}/transactions", header.id))).await?;
        let empty = HashMap::new();
        let txs: Vec<Tx> = b.transactions.into_iter().map(|t| to_tx(t, &empty)).collect();
        let reward = txs.first().and_then(|t| find_miner_reward(&t.outputs));
        Ok(Block {
            id: header.id.clone(),
            height: header.height,
            timestamp_ms: header.timestamp_ms,
            size: b.size,
            miner_address: reward.map(|r| r.address.clone()),
            miner_reward: reward.map(|r| r.value).unwrap_or(0),
            tx_ids: txs.iter().map(|t| t.id.clone()).collect(),
        })
    }

    pub async fn token(&self, token_id: &str) -> Result<TokenMeta> {
        let t: TokenJson = get_json(&self.http, &self.url(&format!("/blockchain/token/byId/{token_id}"))).await?;
        Ok(TokenMeta { token_id: t.id, name: t.name, decimals: t.decimals.unwrap_or(0) })
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p ergotop-core sources::node`
Expected: 7 tests PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/ergotop-core/src/sources crates/ergotop-core/src/lib.rs crates/ergotop-core/tests/fixtures/node
git commit -m "feat(core): node client with id-diff fetch, input resolution, blocks, tokens

<trailer>"
```

---

### Task 7: Explorer client

**Files:**
- Create: `crates/ergotop-core/src/sources/explorer.rs`, fixtures under `crates/ergotop-core/tests/fixtures/explorer/`
- Modify: `crates/ergotop-core/src/sources/mod.rs` (add `pub mod explorer;`)

**Interfaces:**
- Consumes: `sources::{get_json, Result}`, `model::*`.
- Produces: `sources::explorer::ExplorerClient` with
  - `pub fn new(http: Client, base: &str) -> Self`
  - `pub async fn mempool(&self) -> Result<Vec<Tx>>` — `GET /transactions/unconfirmed?limit=10000&offset=0`
  - `pub async fn latest_blocks(&self, n: u32) -> Result<Vec<BlockRef>>` — `GET /api/v1/blocks?limit={n}`
  - `pub async fn block(&self, id: &str) -> Result<Block>` — `GET /api/v1/blocks/{id}`

Explorer shapes (captured 2026-10-03): inputs `{id, value, address}`; outputs `{id, value, ergoTree, address, assets:[{tokenId, amount, name, decimals}]}`; unconfirmed tx adds `creationTimestamp`, `size`; block detail is `{block:{header:{id,height,timestamp,size}, blockTransactions:[tx...]}}`.

- [ ] **Step 1: Create explorer fixtures**

`crates/ergotop-core/tests/fixtures/explorer/unconfirmed.json`:

```json
{"items":[
  {"id":"e1","inputs":[{"id":"in-1","transactionId":"e1","value":11889000000,"index":0,"address":"9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1"}],
   "dataInputs":[],
   "outputs":[
     {"id":"out-1","txId":"e1","value":11887500000,"index":0,"creationHeight":1886100,"ergoTree":"10010100d17300","address":"4MQyMKvMbnCJG3aJ","assets":[{"tokenId":"tok-1","index":0,"amount":5,"name":"SigUSD","decimals":2,"type":"EIP-004"}],"additionalRegisters":{}},
     {"id":"out-2","txId":"e1","value":1500000,"index":1,"creationHeight":1886100,"ergoTree":"1005040004000e36100204a00b08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304","address":"2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1bT2Fk4FkFLCfdPpe","assets":[],"additionalRegisters":{}}
   ],
   "creationTimestamp":1790978000000,"size":412}
],"total":1}
```

`.../explorer/blocks.json`:

```json
{"items":[
  {"id":"blk-2","height":1886101,"timestamp":1790978083213,"transactionsCount":2,"miner":{"address":"88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY","name":"2TH22DBY"},"size":187236,"difficulty":1,"minerReward":3000000000},
  {"id":"blk-1","height":1886100,"timestamp":1790977963213,"transactionsCount":1,"miner":{"address":"88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY","name":"2TH22DBY"},"size":1000,"difficulty":1,"minerReward":3000000000}
],"total":2}
```

`.../explorer/block.json`:

```json
{"block":{
  "header":{"id":"blk-2","height":1886101,"timestamp":1790978083213,"size":187236,"version":4},
  "blockTransactions":[
    {"id":"cb-1","headerId":"blk-2","inclusionHeight":1886101,"timestamp":1790978083213,"index":0,"confirmationsCount":1,
     "inputs":[{"id":"emission","value":1170936000000000,"address":"2Z4YBkDsDvQj8BX7xiySFewjitqp2ge9c99jfes2whbtKitZTxdBYqbrVZUvZvKv6aqn9by4kp3LE1c26LCyosFnVnm6b6U1JYvWpYmL2ZnixJbXLjWAWuBThV1D6dLpqZJYQHYDznJCk49g5TUiS4q8khpag2aNmHwREV7JSsypHdHLgJT7MGaw51aJfNubyzSKxZ4AJXFS27EfXwyCLzW1K6GVqwkJtCoPvrcLqmqwacAWJPkmh78nke9H4oT88XmSbRt2n9aWZjosiZCafZ4osUDxmZcc5QVEeTWn8drSraY3eFKe8Mu9MSCcVU"}],
     "dataInputs":[],
     "outputs":[
       {"id":"em-2","txId":"cb-1","value":1170924000000000,"index":0,"creationHeight":1886101,"ergoTree":"10010100d17300","address":"4MQyMKvMbnCJG3aJ","assets":[],"additionalRegisters":{}},
       {"id":"rw-1","txId":"cb-1","value":12000000000,"index":1,"creationHeight":1886101,"ergoTree":"100204a00b08cd0274e729bb6615cbda94d9d176a2f1525068f12b330e38bbbf387232797dfd891fea02d192a39a8cc7a70173007301","address":"88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY","assets":[],"additionalRegisters":{}}
     ]},
    {"id":"e1","headerId":"blk-2","inclusionHeight":1886101,"timestamp":1790978083213,"index":1,"confirmationsCount":1,"inputs":[],"dataInputs":[],"outputs":[]}
  ]},
 "references":{}}
```

- [ ] **Step 2: Write the failing tests**

`crates/ergotop-core/src/sources/explorer.rs`:

```rust
//! Ergo Explorer API client (public and p2p instances share the API).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::http_client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UNCONFIRMED: &str = include_str!("../../tests/fixtures/explorer/unconfirmed.json");
    const BLOCKS: &str = include_str!("../../tests/fixtures/explorer/blocks.json");
    const BLOCK: &str = include_str!("../../tests/fixtures/explorer/block.json");

    async fn mock(server: &MockServer, p: &str, status: u16, body: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn parses_mempool() {
        let s = MockServer::start().await;
        mock(&s, "/transactions/unconfirmed", 200, UNCONFIRMED).await;
        let txs = ExplorerClient::new(http_client(), &s.uri()).mempool().await.unwrap();
        assert_eq!(txs.len(), 1);
        let t = &txs[0];
        assert_eq!(t.id, "e1");
        assert_eq!(t.size, 412);
        assert_eq!(t.creation_ts_ms, Some(1790978000000));
        let input = t.inputs[0].resolved.as_ref().unwrap();
        assert_eq!(input.address, "9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1");
        assert_eq!(input.value, 11889000000);
        assert_eq!(t.outputs[0].address, "4MQyMKvMbnCJG3aJ");
        assert_eq!(t.outputs[0].tokens[0].amount, 5);
        let m = crate::metrics::tx_metrics(t);
        assert_eq!((m.fee, m.value, m.approx), (1500000, 11887500000, false));
    }

    #[tokio::test]
    async fn parses_blocks_and_block_detail() {
        let s = MockServer::start().await;
        mock(&s, "/api/v1/blocks", 200, BLOCKS).await;
        mock(&s, "/api/v1/blocks/blk-2", 200, BLOCK).await;
        let c = ExplorerClient::new(http_client(), &s.uri());
        let refs = c.latest_blocks(2).await.unwrap();
        assert_eq!(refs[0], BlockRef { id: "blk-2".into(), height: 1886101, timestamp_ms: 1790978083213 });
        let b = c.block("blk-2").await.unwrap();
        assert_eq!(b.tx_ids, vec!["cb-1", "e1"]);
        assert_eq!(b.size, 187236);
        assert_eq!(b.miner_reward, 12000000000);
        assert!(b.miner_address.unwrap().starts_with("88dhgz"));
    }

    #[tokio::test]
    async fn http_error_is_reported() {
        let s = MockServer::start().await;
        mock(&s, "/transactions/unconfirmed", 503, "").await;
        let err = ExplorerClient::new(http_client(), &s.uri()).mempool().await.unwrap_err();
        assert!(matches!(err, crate::sources::SourceError::Status(503)));
    }
}
```

Add `pub mod explorer;` to `sources/mod.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p ergotop-core sources::explorer`
Expected: FAIL to compile — `ExplorerClient` not found.

- [ ] **Step 4: Implement**

Insert above the tests module:

```rust
use serde::Deserialize;

use super::{get_json, Result};
use crate::model::{find_miner_reward, Block, BlockRef, BoxData, Input, Token, Tx};

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetJson {
    token_id: String,
    amount: u64,
}

#[derive(Deserialize)]
struct InputJson {
    id: String,
    #[serde(default)]
    value: Option<u64>,
    #[serde(default)]
    address: Option<String>,
}

#[derive(Deserialize)]
struct OutputJson {
    id: String,
    value: u64,
    address: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TxJson {
    id: String,
    #[serde(default)]
    inputs: Vec<InputJson>,
    #[serde(default)]
    outputs: Vec<OutputJson>,
    #[serde(default)]
    creation_timestamp: Option<u64>,
    #[serde(default)]
    size: Option<u32>,
}

#[derive(Deserialize)]
struct BlockSummaryJson {
    id: String,
    height: u32,
    timestamp: u64,
}

#[derive(Deserialize)]
struct HeaderJson {
    id: String,
    height: u32,
    timestamp: u64,
    #[serde(default)]
    size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlockBodyJson {
    header: HeaderJson,
    block_transactions: Vec<TxJson>,
}

#[derive(Deserialize)]
struct BlockJson {
    block: BlockBodyJson,
}

fn to_tx(t: TxJson) -> Tx {
    Tx {
        id: t.id,
        size: t.size.unwrap_or(0),
        inputs: t
            .inputs
            .into_iter()
            .map(|i| {
                let resolved = match (i.value, i.address) {
                    (Some(value), Some(address)) => {
                        Some(BoxData { box_id: i.id.clone(), value, address, tokens: vec![] })
                    }
                    _ => None,
                };
                Input { box_id: i.id, resolved }
            })
            .collect(),
        outputs: t
            .outputs
            .into_iter()
            .map(|o| BoxData {
                box_id: o.id,
                value: o.value,
                address: o.address,
                tokens: o
                    .assets
                    .into_iter()
                    .map(|a| Token { token_id: a.token_id, amount: a.amount })
                    .collect(),
            })
            .collect(),
        creation_ts_ms: t.creation_timestamp,
    }
}

pub struct ExplorerClient {
    http: reqwest::Client,
    base: String,
}

impl ExplorerClient {
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').to_string() }
    }

    pub async fn mempool(&self) -> Result<Vec<Tx>> {
        let url = format!("{}/transactions/unconfirmed?limit=10000&offset=0", self.base);
        let page: Page<TxJson> = get_json(&self.http, &url).await?;
        Ok(page.items.into_iter().map(to_tx).collect())
    }

    pub async fn latest_blocks(&self, n: u32) -> Result<Vec<BlockRef>> {
        let url = format!("{}/api/v1/blocks?limit={n}", self.base);
        let page: Page<BlockSummaryJson> = get_json(&self.http, &url).await?;
        Ok(page
            .items
            .into_iter()
            .map(|b| BlockRef { id: b.id, height: b.height, timestamp_ms: b.timestamp })
            .collect())
    }

    pub async fn block(&self, id: &str) -> Result<Block> {
        let b: BlockJson = get_json(&self.http, &format!("{}/api/v1/blocks/{id}", self.base)).await?;
        let txs: Vec<Tx> = b.block.block_transactions.into_iter().map(to_tx).collect();
        let reward = txs.first().and_then(|t| find_miner_reward(&t.outputs));
        let h = b.block.header;
        Ok(Block {
            id: h.id,
            height: h.height,
            timestamp_ms: h.timestamp,
            size: h.size,
            miner_address: reward.map(|r| r.address.clone()),
            miner_reward: reward.map(|r| r.value).unwrap_or(0),
            tx_ids: txs.iter().map(|t| t.id.clone()).collect(),
        })
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ergotop-core sources::explorer`
Expected: 3 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ergotop-core/src/sources crates/ergotop-core/tests/fixtures/explorer
git commit -m "feat(core): explorer client for mempool and blocks

<trailer>"
```

---

### Task 8: Address book (fetch, cache, snapshot) and price

**Files:**
- Create: `crates/ergotop-core/src/sources/addressbook.rs`, `crates/ergotop-core/src/sources/price.rs`, `assets/addressbook-snapshot.json`
- Modify: `crates/ergotop-core/src/sources/mod.rs` (add `pub mod addressbook; pub mod price;`)

**Interfaces:**
- Consumes: `classify::{BookEntry, Kind}`, `sources::{SourceError, Result}`.
- Produces:
  - `addressbook::BOOK_API: &str = "https://api.ergexplorer.com"`
  - `addressbook::parse(json: &str) -> Result<Vec<BookEntry>>`
  - `addressbook::snapshot() -> Vec<BookEntry>`
  - `addressbook::is_stale(modified: SystemTime, now: SystemTime) -> bool` (> 24h)
  - `addressbook::load_cache(path: &Path) -> Option<(Vec<BookEntry>, SystemTime)>`
  - `addressbook::save_cache(path: &Path, raw: &str) -> std::io::Result<()>`
  - `addressbook::initial(cache: Option<&Path>) -> (Vec<BookEntry>, bool /* needs refresh */)`
  - `addressbook::fetch(http: &Client, base: &str) -> Result<(String /* raw */, Vec<BookEntry>)>`
  - `price::PRICE_URL: &str`, `price::parse_price(text: &str) -> Option<f64>`, `price::fetch_price(http: &Client, url: &str) -> Result<f64>`

- [ ] **Step 1: Capture the address book snapshot**

Run (repo root):

```bash
curl -s -m 30 "https://api.ergexplorer.com/addressbook/getAddresses?offset=0&limit=5000&type=all&order=nameAsc&query=&testnet=0" -o assets/addressbook-snapshot.json
python -c "import json;d=json.load(open('assets/addressbook-snapshot.json',encoding='utf-8'));print(len(d['items']))"
```

Expected: a count ≥ 300 (362 on 2026-10-03).

- [ ] **Step 2: Write the failing tests**

`crates/ergotop-core/src/sources/addressbook.rs`:

```rust
//! ergexplorer.com address book: fetch, disk cache, embedded snapshot.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Kind;
    use crate::sources::http_client;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE: &str = r#"{"items":[
        {"address":"88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY","name":"2miners","url":"https://2miners.com","type":"Mining pool","urltype":"","addressmd5":"x"},
        {"address":"9fyeEQBXvJzRYpRmrNy2eaB2kDqQGDk3KoSQGUB62db3tVDw2Z1","name":"$BASS","url":"","type":"Meme","urltype":"Pond","addressmd5":"y"}
    ],"total":2,"tokens":[]}"#;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ergotop-book-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("addressbook.json")
    }

    #[test]
    fn parses_entries_and_kinds() {
        let e = parse(SAMPLE).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "2miners");
        assert_eq!(e[0].kind, Kind::MiningPool);
        assert_eq!(e[1].kind, Kind::Meme);
    }

    #[test]
    fn embedded_snapshot_is_usable_offline() {
        let e = snapshot();
        assert!(e.len() >= 300, "got {}", e.len());
        assert!(e.iter().any(|b| b.kind == Kind::MiningPool));
    }

    #[test]
    fn staleness_is_24h() {
        let now = SystemTime::now();
        assert!(!is_stale(now - Duration::from_secs(23 * 3600), now));
        assert!(is_stale(now - Duration::from_secs(25 * 3600), now));
    }

    #[test]
    fn initial_uses_snapshot_without_cache_and_cache_when_present() {
        let path = temp_file("initial");
        let (entries, refresh) = initial(Some(path.as_path()));
        assert!(entries.len() >= 300);
        assert!(refresh, "no cache -> refresh needed");

        save_cache(&path, SAMPLE).unwrap();
        let (entries, refresh) = initial(Some(path.as_path()));
        assert_eq!(entries.len(), 2);
        assert!(!refresh, "fresh cache -> no refresh");

        let (entries, refresh) = initial(None);
        assert!(entries.len() >= 300);
        assert!(refresh);
    }

    #[tokio::test]
    async fn fetches_from_api() {
        let s = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/addressbook/getAddresses"))
            .and(query_param("type", "all"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&s)
            .await;
        let (raw, entries) = fetch(&http_client(), &s.uri()).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(raw, SAMPLE);
    }
}
```

`crates/ergotop-core/src/sources/price.rs`:

```rust
//! ERG/USD price from the SigmaUSD oracle frontend.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_json_string() {
        let text = r#""{\"title\":\"Erg-USD\",\"latest_price\":0.3227559441177931}""#;
        assert_eq!(parse_price(text), Some(0.3227559441177931));
    }

    #[test]
    fn parses_plain_json_and_rejects_garbage() {
        assert_eq!(parse_price(r#"{"latest_price":1.5}"#), Some(1.5));
        assert_eq!(parse_price("<html>"), None);
    }
}
```

Add `pub mod addressbook;` and `pub mod price;` to `sources/mod.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p ergotop-core sources::addressbook sources::price`
Expected: FAIL to compile — `parse`, `snapshot`, `parse_price`, … not found.

(Cargo accepts one filter; if the two-filter form errors, run `cargo test -p ergotop-core sources::` instead.)

- [ ] **Step 4: Implement the address book**

Insert above the tests module in `addressbook.rs`:

```rust
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use super::{Result, SourceError};
use crate::classify::{BookEntry, Kind};

pub const BOOK_API: &str = "https://api.ergexplorer.com";
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);
const SNAPSHOT: &str = include_str!("../../../../assets/addressbook-snapshot.json");

#[derive(Deserialize)]
struct BookPage {
    items: Vec<BookItem>,
}

#[derive(Deserialize)]
struct BookItem {
    address: String,
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
}

pub fn parse(json: &str) -> Result<Vec<BookEntry>> {
    let page: BookPage = serde_json::from_str(json).map_err(|e| SourceError::Parse(e.to_string()))?;
    Ok(page
        .items
        .into_iter()
        .map(|i| BookEntry { address: i.address, name: i.name, kind: Kind::parse(&i.kind) })
        .collect())
}

pub fn snapshot() -> Vec<BookEntry> {
    parse(SNAPSHOT).expect("assets/addressbook-snapshot.json")
}

pub fn is_stale(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified).map(|age| age > MAX_AGE).unwrap_or(false)
}

pub fn load_cache(path: &Path) -> Option<(Vec<BookEntry>, SystemTime)> {
    let text = std::fs::read_to_string(path).ok()?;
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some((parse(&text).ok()?, modified))
}

pub fn save_cache(path: &Path, raw: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, raw)
}

/// Entries to start with (cache, else embedded snapshot) and whether a network refresh is due.
pub fn initial(cache: Option<&Path>) -> (Vec<BookEntry>, bool) {
    match cache.and_then(load_cache) {
        Some((entries, modified)) => (entries, is_stale(modified, SystemTime::now())),
        None => (snapshot(), true),
    }
}

pub async fn fetch(http: &reqwest::Client, base: &str) -> Result<(String, Vec<BookEntry>)> {
    let url = format!(
        "{}/addressbook/getAddresses?offset=0&limit=5000&type=all&order=nameAsc&query=&testnet=0",
        base.trim_end_matches('/')
    );
    let resp = http.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(SourceError::Status(resp.status().as_u16()));
    }
    let raw = resp.text().await?;
    let entries = parse(&raw)?;
    Ok((raw, entries))
}

/// Default cache file location.
pub fn default_cache_path(cache_dir: Option<PathBuf>) -> Option<PathBuf> {
    cache_dir.map(|d| d.join("addressbook.json"))
}
```

- [ ] **Step 5: Implement price**

Insert above the tests module in `price.rs`:

```rust
use super::{Result, SourceError};

pub const PRICE_URL: &str = "https://erg-oracle-ergusd.spirepools.com/frontendData";

/// The oracle returns a JSON object encoded as a JSON string; accept both forms.
pub fn parse_price(text: &str) -> Option<f64> {
    let text = text.trim();
    let inner: String = if text.starts_with('"') {
        serde_json::from_str::<String>(text).ok()?
    } else {
        text.to_string()
    };
    let v: serde_json::Value = serde_json::from_str(&inner).ok()?;
    v.get("latest_price")?.as_f64()
}

pub async fn fetch_price(http: &reqwest::Client, url: &str) -> Result<f64> {
    let text = http.get(url).send().await?.text().await?;
    parse_price(&text).ok_or_else(|| SourceError::Parse("latest_price missing".into()))
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p ergotop-core sources::`
Expected: all `sources::` tests PASS (5 address book + 2 price + earlier node/explorer).

- [ ] **Step 7: Commit**

```bash
git add assets/addressbook-snapshot.json crates/ergotop-core/src/sources
git commit -m "feat(core): ergexplorer address book with cache/snapshot, oracle price

<trailer>"
```

---

### Task 9: Reconciler

**Files:**
- Create: `crates/ergotop-core/src/reconcile.rs`
- Modify: `crates/ergotop-core/src/sources/mod.rs` (add `SourceEvent`), `crates/ergotop-core/src/lib.rs` (add `pub mod reconcile;`)

**Interfaces:**
- Consumes: `classify::{Classifier, Builtin, TxClass, BookEntry}`, `metrics::{tx_metrics, TxMetrics}`, `model::*`.
- Produces:
  - `sources::SourceEvent` (Clone, Debug):
    ```rust
    pub enum SourceEvent {
        Status { source: SourceId, status: SourceStatus },
        Mempool { source: SourceId, ids: Vec<TxId>, new_txs: Vec<Tx>, latency_ms: u64 },
        Block { source: SourceId, block: Block },
        Info { source: SourceId, info: NodeInfo },
        Price(f64),
        AddressBook(Vec<BookEntry>),
        TokenMeta(TokenMeta),
    }
    ```
    `Mempool.ids` is the source's full current id set; `new_txs` are bodies the source has not sent before.
  - `reconcile::DROP_GRACE_POLLS: u8 = 2`
  - `reconcile::TxEntry { pub tx: Tx, pub first_seen_ms: u64, pub seen_by: BTreeSet<SourceId>, pub class: TxClass, pub metrics: TxMetrics }`
  - `reconcile::SourceView { pub id, pub kind, pub status, pub ids: HashSet<TxId>, pub last_update_ms: Option<u64>, pub latency_ms: Option<u64>, pub info: Option<NodeInfo> }`
  - `reconcile::Update { Added(Vec<TxId>), Mined { height: u32, tx_ids: Vec<TxId> }, Dropped(Vec<TxId>), Resynced, BlockAdded(u32), SourcesChanged }` (Clone, Debug, PartialEq, Eq) — id vectors sorted ascending.
  - `reconcile::Reconciler` with `new(sources: Vec<(SourceId, SourceKind)>)`, `apply(&mut self, ev: SourceEvent, now_ms: u64, cls: &Classifier) -> Vec<Update>`, `active() -> Option<&SourceId>`, `pool() -> &HashMap<TxId, TxEntry>`, `views() -> &[SourceView]`, `recent_blocks() -> Vec<&Block>` (height desc), `only_in(&SourceId) -> Vec<TxId>` (sorted), `reclassify(&mut self, cls: &Classifier)`.

Rules (spec §3.2): sources are in priority order; active = first source whose status is usable and that has delivered a mempool. Active change → pool rebuilt from the new active view → single `Resynced`. A `Mempool` from a non-usable source marks it `Up`. Removed txs → `Mined` if in a recent block, else pending; pending txs found in a later block → `Mined`; pending for `DROP_GRACE_POLLS` further active polls → `Dropped`. Empty active snapshot while pool > 5 is ignored once.

- [ ] **Step 1: Add `SourceEvent` to `sources/mod.rs`**

Append:

```rust
use crate::classify::BookEntry;
use crate::model::{Block, NodeInfo, SourceId, SourceStatus, TokenMeta, Tx, TxId};

#[derive(Clone, Debug)]
pub enum SourceEvent {
    Status { source: SourceId, status: SourceStatus },
    /// `ids` is the full current mempool of `source`; `new_txs` are bodies not sent before.
    Mempool { source: SourceId, ids: Vec<TxId>, new_txs: Vec<Tx>, latency_ms: u64 },
    Block { source: SourceId, block: Block },
    Info { source: SourceId, info: NodeInfo },
    Price(f64),
    AddressBook(Vec<BookEntry>),
    TokenMeta(TokenMeta),
}
```

- [ ] **Step 2: Write the failing tests**

`crates/ergotop-core/src/reconcile.rs`:

```rust
//! Folds per-source events into one canonical mempool.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Builtin;
    use crate::model::test_util::{bx, tx};

    const W: &str = "9guaDYhHCxtfAdRTKr8xXaDuXtdB8gdGB7WwnB5zTBw93Ym3Rsq";

    fn node() -> SourceId {
        SourceId("node".into())
    }
    fn expl() -> SourceId {
        SourceId("p2p".into())
    }
    fn cls() -> Classifier {
        Classifier::new(&Builtin::default(), &[], &[])
    }
    fn rec() -> Reconciler {
        Reconciler::new(vec![(node(), SourceKind::Node), (expl(), SourceKind::Explorer)])
    }
    fn t(id: &str) -> Tx {
        tx(id, 100, vec![bx(W, 10)], vec![bx(W, 9)])
    }
    fn mempool(source: SourceId, ids: &[&str], new: &[&str]) -> SourceEvent {
        SourceEvent::Mempool {
            source,
            ids: ids.iter().map(|s| s.to_string()).collect(),
            new_txs: new.iter().map(|s| t(s)).collect(),
            latency_ms: 10,
        }
    }
    fn block(id: &str, height: u32, txs: &[&str]) -> SourceEvent {
        SourceEvent::Block {
            source: node(),
            block: Block {
                id: id.into(),
                height,
                timestamp_ms: 0,
                size: 0,
                tx_ids: txs.iter().map(|s| s.to_string()).collect(),
                miner_address: None,
                miner_reward: 0,
            },
        }
    }
    fn ids(v: &[&str]) -> Vec<TxId> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn first_snapshot_resyncs_then_new_tx_is_added() {
        let (mut r, c) = (rec(), cls());
        let u = r.apply(mempool(node(), &["a"], &["a"]), 1, &c);
        assert_eq!(u, vec![Update::SourcesChanged, Update::Resynced]);
        assert_eq!(r.active(), Some(&node()));
        assert_eq!(r.pool().len(), 1);
        let u = r.apply(mempool(node(), &["a", "b"], &["b"]), 2, &c);
        assert_eq!(u, vec![Update::Added(ids(&["b"]))]);
        assert_eq!(r.pool()["b"].first_seen_ms, 2);
    }

    #[test]
    fn removal_after_block_is_mined() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        assert_eq!(r.apply(block("h1", 100, &["a"]), 2, &c), vec![Update::BlockAdded(100)]);
        let u = r.apply(mempool(node(), &["b"], &[]), 3, &c);
        assert_eq!(u, vec![Update::Mined { height: 100, tx_ids: ids(&["a"]) }]);
    }

    #[test]
    fn block_after_removal_is_still_mined() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 2, &c), vec![]);
        let u = r.apply(block("h1", 100, &["a"]), 3, &c);
        assert_eq!(u, vec![Update::BlockAdded(100), Update::Mined { height: 100, tx_ids: ids(&["a"]) }]);
    }

    #[test]
    fn removal_without_block_is_dropped_after_grace() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 2, &c), vec![]);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 3, &c), vec![]);
        assert_eq!(r.apply(mempool(node(), &["b"], &[]), 4, &c), vec![Update::Dropped(ids(&["a"]))]);
    }

    #[test]
    fn duplicate_block_from_second_source_is_ignored() {
        let (mut r, c) = (rec(), cls());
        r.apply(block("h1", 100, &[]), 1, &c);
        assert_eq!(r.apply(block("h1", 100, &[]), 2, &c), vec![]);
        assert_eq!(r.recent_blocks().len(), 1);
    }

    #[test]
    fn single_empty_snapshot_is_ignored() {
        let (mut r, c) = (rec(), cls());
        let all = ["a", "b", "c", "d", "e", "f"];
        r.apply(mempool(node(), &all, &all), 1, &c);
        assert_eq!(r.apply(mempool(node(), &[], &[]), 2, &c), vec![]);
        assert_eq!(r.pool().len(), 6);
        r.apply(mempool(node(), &[], &[]), 3, &c);
        assert_eq!(r.pool().len(), 0, "second empty snapshot is believed");
    }

    #[test]
    fn failover_to_explorer_is_one_resync() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        r.apply(mempool(expl(), &["b", "c"], &["b", "c"]), 2, &c);
        assert_eq!(r.active(), Some(&node()));
        let u = r.apply(SourceEvent::Status { source: node(), status: SourceStatus::Down("timeout".into()) }, 3, &c);
        assert_eq!(u, vec![Update::SourcesChanged, Update::Resynced]);
        assert_eq!(r.active(), Some(&expl()));
        let mut pool: Vec<&String> = r.pool().keys().collect();
        pool.sort();
        assert_eq!(pool, vec!["b", "c"]);
    }

    #[test]
    fn node_takes_over_from_explorer_when_it_reports() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(expl(), &["a"], &["a"]), 1, &c);
        assert_eq!(r.active(), Some(&expl()));
        let u = r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 2, &c);
        assert_eq!(u, vec![Update::SourcesChanged, Update::Resynced]);
        assert_eq!(r.active(), Some(&node()));
        assert_eq!(r.pool().len(), 2);
    }

    #[test]
    fn tracks_seen_by_and_only_in() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a", "b"], &["a", "b"]), 1, &c);
        r.apply(mempool(expl(), &["b", "c"], &["b", "c"]), 2, &c);
        assert_eq!(r.pool()["b"].seen_by.len(), 2);
        assert_eq!(r.pool()["a"].seen_by.len(), 1);
        assert_eq!(r.only_in(&node()), ids(&["a"]));
        assert_eq!(r.only_in(&expl()), ids(&["c"]));
    }

    #[test]
    fn reclassify_updates_entries() {
        let (mut r, c) = (rec(), cls());
        r.apply(mempool(node(), &["a"], &["a"]), 1, &c);
        assert_eq!(r.pool()["a"].class.class.name, "P2P");
        let book = [crate::classify::BookEntry { address: W.into(), name: "Kucoin".into(), kind: crate::classify::Kind::Exchange }];
        r.reclassify(&Classifier::new(&Builtin::default(), &book, &[]));
        assert_eq!(r.pool()["a"].class.class.name, "Kucoin");
    }
}
```

Add `pub mod reconcile;` to `lib.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p ergotop-core reconcile`
Expected: FAIL to compile — `Reconciler`, `Update` not found.

- [ ] **Step 4: Implement**

Insert above the tests module in `reconcile.rs`:

```rust
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::classify::{Classifier, TxClass};
use crate::metrics::{tx_metrics, TxMetrics};
use crate::model::{Block, NodeInfo, SourceId, SourceKind, SourceStatus, Tx, TxId};
use crate::sources::SourceEvent;

/// Active polls a removed tx waits for its block before it is declared dropped.
pub const DROP_GRACE_POLLS: u8 = 2;
const RECENT_BLOCKS: usize = 10;
const EMPTY_GUARD_MIN: usize = 5;

#[derive(Clone, Debug)]
pub struct TxEntry {
    pub tx: Tx,
    pub first_seen_ms: u64,
    pub seen_by: BTreeSet<SourceId>,
    pub class: TxClass,
    pub metrics: TxMetrics,
}

#[derive(Clone, Debug)]
pub struct SourceView {
    pub id: SourceId,
    pub kind: SourceKind,
    pub status: SourceStatus,
    pub ids: HashSet<TxId>,
    pub last_update_ms: Option<u64>,
    pub latency_ms: Option<u64>,
    pub info: Option<NodeInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    Added(Vec<TxId>),
    Mined { height: u32, tx_ids: Vec<TxId> },
    Dropped(Vec<TxId>),
    /// The pool was rebuilt (startup or failover); redraw without per-tx animation.
    Resynced,
    BlockAdded(u32),
    SourcesChanged,
}

pub struct Reconciler {
    views: Vec<SourceView>,
    active: Option<SourceId>,
    pool: HashMap<TxId, TxEntry>,
    bodies: HashMap<TxId, Tx>,
    first_seen: HashMap<TxId, u64>,
    pending: HashMap<TxId, u8>,
    blocks: VecDeque<Block>,
    empty_strike: bool,
}

impl Reconciler {
    pub fn new(sources: Vec<(SourceId, SourceKind)>) -> Self {
        let views = sources
            .into_iter()
            .map(|(id, kind)| SourceView {
                id,
                kind,
                status: SourceStatus::Unknown,
                ids: HashSet::new(),
                last_update_ms: None,
                latency_ms: None,
                info: None,
            })
            .collect();
        Reconciler {
            views,
            active: None,
            pool: HashMap::new(),
            bodies: HashMap::new(),
            first_seen: HashMap::new(),
            pending: HashMap::new(),
            blocks: VecDeque::new(),
            empty_strike: false,
        }
    }

    pub fn active(&self) -> Option<&SourceId> {
        self.active.as_ref()
    }

    pub fn pool(&self) -> &HashMap<TxId, TxEntry> {
        &self.pool
    }

    pub fn views(&self) -> &[SourceView] {
        &self.views
    }

    pub fn recent_blocks(&self) -> Vec<&Block> {
        let mut v: Vec<&Block> = self.blocks.iter().collect();
        v.sort_by(|a, b| b.height.cmp(&a.height));
        v
    }

    /// Ids that `source` has but no other source has.
    pub fn only_in(&self, source: &SourceId) -> Vec<TxId> {
        let Some(view) = self.views.iter().find(|v| &v.id == source) else { return vec![] };
        let mut out: Vec<TxId> = view
            .ids
            .iter()
            .filter(|id| self.views.iter().all(|o| &o.id == source || !o.ids.contains(*id)))
            .cloned()
            .collect();
        out.sort();
        out
    }

    pub fn reclassify(&mut self, cls: &Classifier) {
        for e in self.pool.values_mut() {
            e.class = cls.classify_tx(&e.tx);
        }
    }

    pub fn apply(&mut self, ev: SourceEvent, now_ms: u64, cls: &Classifier) -> Vec<Update> {
        match ev {
            SourceEvent::Status { source, status } => {
                let Some(v) = self.views.iter_mut().find(|v| v.id == source) else { return vec![] };
                if v.status == status {
                    return vec![];
                }
                v.status = status;
                let mut out = vec![Update::SourcesChanged];
                out.extend(self.reselect_active(cls));
                out
            }
            SourceEvent::Info { source, info } => {
                match self.views.iter_mut().find(|v| v.id == source) {
                    Some(v) => {
                        v.info = Some(info);
                        vec![Update::SourcesChanged]
                    }
                    None => vec![],
                }
            }
            SourceEvent::Mempool { source, ids, new_txs, latency_ms } => {
                self.on_mempool(source, ids, new_txs, latency_ms, now_ms, cls)
            }
            SourceEvent::Block { block, .. } => self.on_block(block),
            SourceEvent::Price(_) | SourceEvent::AddressBook(_) | SourceEvent::TokenMeta(_) => vec![],
        }
    }

    fn on_mempool(
        &mut self,
        source: SourceId,
        ids: Vec<TxId>,
        new_txs: Vec<Tx>,
        latency_ms: u64,
        now_ms: u64,
        cls: &Classifier,
    ) -> Vec<Update> {
        let Some(idx) = self.views.iter().position(|v| v.id == source) else { return vec![] };
        for id in &ids {
            self.first_seen.entry(id.clone()).or_insert(now_ms);
        }
        for tx in new_txs {
            self.first_seen.entry(tx.id.clone()).or_insert(now_ms);
            self.bodies.entry(tx.id.clone()).or_insert(tx);
        }

        let is_active = self.active.as_ref() == Some(&source);
        if is_active && ids.is_empty() && self.pool.len() > EMPTY_GUARD_MIN && !self.empty_strike {
            self.empty_strike = true;
            return vec![];
        }
        if is_active {
            self.empty_strike = false;
        }

        let mut out = Vec::new();
        {
            let v = &mut self.views[idx];
            v.ids = ids.into_iter().collect();
            v.last_update_ms = Some(now_ms);
            v.latency_ms = Some(latency_ms);
            if !v.status.usable() {
                v.status = SourceStatus::Up;
                out.push(Update::SourcesChanged);
            }
        }
        let view_ids = &self.views[idx].ids;
        for e in self.pool.values_mut() {
            if view_ids.contains(&e.tx.id) {
                e.seen_by.insert(source.clone());
            } else {
                e.seen_by.remove(&source);
            }
        }

        let reselected = self.reselect_active(cls);
        if !reselected.is_empty() {
            out.extend(reselected);
        } else if self.active.as_ref() == Some(&source) {
            out.extend(self.diff_active(idx, cls));
        }
        self.prune();
        out
    }

    fn reselect_active(&mut self, cls: &Classifier) -> Vec<Update> {
        let next = self
            .views
            .iter()
            .find(|v| v.status.usable() && v.last_update_ms.is_some())
            .map(|v| v.id.clone());
        if next == self.active {
            return vec![];
        }
        self.active = next;
        self.resync(cls);
        vec![Update::Resynced]
    }

    fn resync(&mut self, cls: &Classifier) {
        self.pending.clear();
        self.empty_strike = false;
        let Some(idx) = self.active_idx() else {
            self.pool.clear();
            return;
        };
        let ids: Vec<TxId> = self.views[idx].ids.iter().cloned().collect();
        let mut pool = HashMap::new();
        for id in ids {
            if let Some(tx) = self.bodies.get(&id).cloned() {
                pool.insert(id, self.make_entry(tx, cls));
            }
        }
        self.pool = pool;
    }

    fn diff_active(&mut self, idx: usize, cls: &Classifier) -> Vec<Update> {
        let (mut added, removed): (Vec<TxId>, Vec<TxId>) = {
            let active_ids = &self.views[idx].ids;
            let added = active_ids
                .iter()
                .filter(|id| !self.pool.contains_key(*id) && self.bodies.contains_key(*id))
                .cloned()
                .collect();
            let removed = self.pool.keys().filter(|id| !active_ids.contains(*id)).cloned().collect();
            (added, removed)
        };
        for id in &added {
            self.pending.remove(id);
            let entry = self.make_entry(self.bodies[id].clone(), cls);
            self.pool.insert(id.clone(), entry);
        }

        let mut dropped = Vec::new();
        for (id, polls) in self.pending.iter_mut() {
            *polls += 1;
            if *polls >= DROP_GRACE_POLLS {
                dropped.push(id.clone());
            }
        }
        for id in &dropped {
            self.pending.remove(id);
        }

        let mut mined: BTreeMap<u32, Vec<TxId>> = BTreeMap::new();
        for id in removed {
            self.pool.remove(&id);
            match self.block_height_of(&id) {
                Some(h) => mined.entry(h).or_default().push(id),
                None => {
                    self.pending.insert(id, 0);
                }
            }
        }

        let mut out = Vec::new();
        if !added.is_empty() {
            added.sort();
            out.push(Update::Added(added));
        }
        for (height, mut tx_ids) in mined {
            tx_ids.sort();
            out.push(Update::Mined { height, tx_ids });
        }
        if !dropped.is_empty() {
            dropped.sort();
            out.push(Update::Dropped(dropped));
        }
        out
    }

    fn on_block(&mut self, block: Block) -> Vec<Update> {
        if self.blocks.iter().any(|b| b.id == block.id) {
            return vec![];
        }
        let height = block.height;
        let mut mined: Vec<TxId> = self
            .pending
            .keys()
            .filter(|id| block.tx_ids.contains(*id))
            .cloned()
            .collect();
        for id in &mined {
            self.pending.remove(id);
        }
        self.blocks.push_front(block);
        self.blocks.truncate(RECENT_BLOCKS);
        let mut out = vec![Update::BlockAdded(height)];
        if !mined.is_empty() {
            mined.sort();
            out.push(Update::Mined { height, tx_ids: mined });
        }
        out
    }

    fn active_idx(&self) -> Option<usize> {
        let active = self.active.as_ref()?;
        self.views.iter().position(|v| &v.id == active)
    }

    fn block_height_of(&self, id: &TxId) -> Option<u32> {
        self.blocks.iter().find(|b| b.tx_ids.contains(id)).map(|b| b.height)
    }

    fn make_entry(&self, tx: Tx, cls: &Classifier) -> TxEntry {
        let seen_by = self
            .views
            .iter()
            .filter(|v| v.ids.contains(&tx.id))
            .map(|v| v.id.clone())
            .collect();
        TxEntry {
            first_seen_ms: self.first_seen.get(&tx.id).copied().unwrap_or(0),
            seen_by,
            class: cls.classify_tx(&tx),
            metrics: tx_metrics(&tx),
            tx,
        }
    }

    /// Forget bodies and timestamps no source still reports.
    fn prune(&mut self) {
        let views = &self.views;
        let pool = &self.pool;
        let pending = &self.pending;
        let keep = |id: &TxId| pool.contains_key(id) || pending.contains_key(id) || views.iter().any(|v| v.ids.contains(id));
        self.bodies.retain(|id, _| keep(id));
        self.first_seen.retain(|id, _| keep(id));
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p ergotop-core reconcile`
Expected: 10 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ergotop-core/src/reconcile.rs crates/ergotop-core/src/sources/mod.rs crates/ergotop-core/src/lib.rs
git commit -m "feat(core): reconciler with exact mined/dropped and failover resync

<trailer>"
```

---

### Task 10: Source poll loops

**Files:**
- Create: `crates/ergotop-core/src/sources/runtime.rs`
- Modify: `crates/ergotop-core/src/sources/mod.rs` (add `pub mod runtime;`)

**Interfaces:**
- Consumes: `NodeClient`, `ExplorerClient`, `addressbook::*`, `price::*`, `config::SourceSpec`, `SourceEvent`.
- Produces (`sources::runtime`):
  - `pub struct Timing { pub node_mempool, pub node_info, pub node_headers, pub explorer_mempool, pub explorer_blocks: Duration }` with `Default` = 1s, 10s, 5s, 5s, 10s
  - `pub fn backoff(fails: u32) -> Duration` — 1s, 2s, 4s, 8s, 16s, then 30s
  - `pub fn node_status(info: &NodeInfo) -> SourceStatus` — `Degraded("index lag N")` when indexed and lag > 2, else `Up`
  - `pub async fn run_node(id: SourceId, client: NodeClient, timing: Timing, tx: mpsc::Sender<SourceEvent>)`
  - `pub async fn run_explorer(id: SourceId, client: ExplorerClient, timing: Timing, tx: mpsc::Sender<SourceEvent>)`
  - `pub async fn run_address_book(http: Client, base: String, cache: Option<PathBuf>, tx: mpsc::Sender<SourceEvent>)`
  - `pub async fn run_price(http: Client, tx: mpsc::Sender<SourceEvent>)`
  - `pub fn spawn_all(specs: &[SourceSpec], timing: Timing, cache_dir: Option<PathBuf>) -> mpsc::Receiver<SourceEvent>` (must be called inside a tokio runtime)

Every loop returns when the receiver is dropped (`tx.is_closed()`).

- [ ] **Step 1: Write the failing tests**

`crates/ergotop-core/src/sources/runtime.rs`:

```rust
//! Per-source polling tasks feeding one event channel.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::http_client;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast() -> Timing {
        let ms = Duration::from_millis(50);
        Timing { node_mempool: ms, node_info: ms, node_headers: ms, explorer_mempool: ms, explorer_blocks: ms }
    }

    async fn mock(server: &MockServer, verb: &str, p: &str, status: u16, body: &str) {
        Mock::given(method(verb))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
            .mount(server)
            .await;
    }

    /// Collects events until every wanted event kind has arrived or 10s pass.
    async fn collect(rx: &mut mpsc::Receiver<SourceEvent>, mut want: Vec<&'static str>) -> Vec<SourceEvent> {
        let mut got = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !want.is_empty() {
            let ev = tokio::time::timeout_at(deadline, rx.recv()).await.expect("timed out").expect("closed");
            let kind = match &ev {
                SourceEvent::Status { .. } => "status",
                SourceEvent::Mempool { .. } => "mempool",
                SourceEvent::Block { .. } => "block",
                SourceEvent::Info { .. } => "info",
                SourceEvent::TokenMeta(_) => "token",
                SourceEvent::AddressBook(_) => "book",
                SourceEvent::Price(_) => "price",
            };
            want.retain(|w| *w != kind);
            got.push(ev);
        }
        got
    }

    #[test]
    fn backoff_doubles_to_cap() {
        let secs: Vec<u64> = (1..=7).map(|f| backoff(f).as_secs()).collect();
        assert_eq!(secs, vec![2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(backoff(0).as_secs(), 1);
    }

    #[test]
    fn index_lag_degrades() {
        let mut info = NodeInfo { full_height: 100, indexed_height: Some(99), ..Default::default() };
        assert_eq!(node_status(&info), SourceStatus::Up);
        info.indexed_height = Some(90);
        assert_eq!(node_status(&info), SourceStatus::Degraded("index lag 10".into()));
        info.indexed_height = None;
        assert_eq!(node_status(&info), SourceStatus::Up);
    }

    #[tokio::test]
    async fn node_loop_emits_info_mempool_block_and_tokens() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/info", 200, include_str!("../../tests/fixtures/node/info.json")).await;
        mock(&s, "GET", "/blockchain/indexedHeight", 200, include_str!("../../tests/fixtures/node/indexed_height.json")).await;
        mock(&s, "GET", "/transactions/unconfirmed/transactionIds", 200, include_str!("../../tests/fixtures/node/mempool_ids.json")).await;
        mock(&s, "POST", "/transactions/unconfirmed/byTransactionIds", 200, include_str!("../../tests/fixtures/node/mempool_txs.json")).await;
        mock(&s, "POST", "/utxo/withPool/byIds", 200, include_str!("../../tests/fixtures/node/boxes.json")).await;
        mock(&s, "GET", "/blocks/lastHeaders/3", 200, include_str!("../../tests/fixtures/node/last_headers.json")).await;
        mock(&s, "GET", "/blocks/hdr-0/transactions", 200, include_str!("../../tests/fixtures/node/block_txs.json")).await;
        mock(&s, "GET", "/blocks/hdr-1/transactions", 200, include_str!("../../tests/fixtures/node/block_txs.json")).await;
        mock(&s, "GET", "/blockchain/token/byId/tok-1", 200, include_str!("../../tests/fixtures/node/token.json")).await;

        let (tx, mut rx) = mpsc::channel(64);
        let id = SourceId("node-a".into());
        let handle = tokio::spawn(run_node(id.clone(), NodeClient::new(http_client(), &s.uri()), fast(), tx));
        let events = collect(&mut rx, vec!["status", "info", "mempool", "block", "token"]).await;
        handle.abort();

        let mempool = events.iter().find_map(|e| match e {
            SourceEvent::Mempool { source, ids, new_txs, .. } => Some((source, ids, new_txs)),
            _ => None,
        });
        let (source, ids, new_txs) = mempool.unwrap();
        assert_eq!(source, &id);
        assert_eq!(ids, &vec!["tx-a".to_string(), "tx-b".to_string()]);
        assert_eq!(new_txs.len(), 2);
        assert!(events.iter().any(|e| matches!(e, SourceEvent::Status { status: SourceStatus::Up, .. })));
    }

    #[tokio::test]
    async fn node_loop_reports_down_for_unreachable_node() {
        let (tx, mut rx) = mpsc::channel(8);
        let client = NodeClient::new(http_client(), "http://127.0.0.1:9");
        let handle = tokio::spawn(run_node(SourceId("dead".into()), client, fast(), tx));
        let events = collect(&mut rx, vec!["status"]).await;
        handle.abort();
        assert!(matches!(&events[0], SourceEvent::Status { status: SourceStatus::Down(_), .. }));
    }

    #[tokio::test]
    async fn explorer_loop_emits_mempool_once_per_new_tx_and_blocks() {
        let s = MockServer::start().await;
        mock(&s, "GET", "/transactions/unconfirmed", 200, include_str!("../../tests/fixtures/explorer/unconfirmed.json")).await;
        mock(&s, "GET", "/api/v1/blocks", 200, include_str!("../../tests/fixtures/explorer/blocks.json")).await;
        mock(&s, "GET", "/api/v1/blocks/blk-1", 200, include_str!("../../tests/fixtures/explorer/block.json")).await;
        mock(&s, "GET", "/api/v1/blocks/blk-2", 200, include_str!("../../tests/fixtures/explorer/block.json")).await;

        let (tx, mut rx) = mpsc::channel(64);
        let handle = tokio::spawn(run_explorer(SourceId("p2p".into()), ExplorerClient::new(http_client(), &s.uri()), fast(), tx));
        let first = collect(&mut rx, vec!["mempool", "block"]).await;
        let second = collect(&mut rx, vec!["mempool"]).await;
        handle.abort();
        let new_counts: Vec<usize> = first
            .iter()
            .chain(second.iter())
            .filter_map(|e| match e {
                SourceEvent::Mempool { new_txs, .. } => Some(new_txs.len()),
                _ => None,
            })
            .collect();
        assert_eq!(new_counts[0], 1);
        assert_eq!(*new_counts.last().unwrap(), 0, "already-sent bodies are not resent");
    }

    #[tokio::test]
    async fn address_book_loop_sends_snapshot_then_fetched_entries() {
        let s = MockServer::start().await;
        let body = r#"{"items":[{"address":"9f","name":"X","type":"Exchange"}],"total":1}"#;
        mock(&s, "GET", "/addressbook/getAddresses", 200, body).await;
        let cache = std::env::temp_dir().join(format!("ergotop-rt-{}", std::process::id())).join("addressbook.json");
        let _ = std::fs::remove_file(&cache);
        let (tx, mut rx) = mpsc::channel(8);
        let handle = tokio::spawn(run_address_book(http_client(), s.uri(), Some(cache.clone()), tx));
        let first = collect(&mut rx, vec!["book"]).await;
        let second = collect(&mut rx, vec!["book"]).await;
        handle.abort();
        assert!(matches!(&first[0], SourceEvent::AddressBook(e) if e.len() >= 300));
        assert!(matches!(&second[0], SourceEvent::AddressBook(e) if e.len() == 1));
        assert!(cache.exists(), "fetched book is cached");
    }
}
```

Add `pub mod runtime;` to `sources/mod.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop-core sources::runtime`
Expected: FAIL to compile — `Timing`, `backoff`, `run_node`, … not found.

- [ ] **Step 3: Implement**

Insert above the tests module:

```rust
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use super::addressbook;
use super::explorer::ExplorerClient;
use super::node::NodeClient;
use super::price;
use super::{http_client, SourceEvent};
use crate::config::SourceSpec;
use crate::model::{NodeInfo, SourceId, SourceKind, SourceStatus, TxId};

const MAX_INDEX_LAG: u32 = 2;
const TOKENS_PER_POLL: usize = 20;
const BLOCKS_PER_POLL: u32 = 3;
const BOOK_REFRESH: Duration = Duration::from_secs(24 * 3600);
const PRICE_EVERY: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub node_mempool: Duration,
    pub node_info: Duration,
    pub node_headers: Duration,
    pub explorer_mempool: Duration,
    pub explorer_blocks: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            node_mempool: Duration::from_secs(1),
            node_info: Duration::from_secs(10),
            node_headers: Duration::from_secs(5),
            explorer_mempool: Duration::from_secs(5),
            explorer_blocks: Duration::from_secs(10),
        }
    }
}

pub fn backoff(fails: u32) -> Duration {
    Duration::from_secs((1u64 << fails.min(5)).min(30))
}

pub fn node_status(info: &NodeInfo) -> SourceStatus {
    match info.indexed_height {
        Some(ih) if info.full_height.saturating_sub(ih) > MAX_INDEX_LAG => {
            SourceStatus::Degraded(format!("index lag {}", info.full_height - ih))
        }
        _ => SourceStatus::Up,
    }
}

async fn send(tx: &mpsc::Sender<SourceEvent>, ev: SourceEvent) {
    let _ = tx.send(ev).await;
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

pub async fn run_node(id: SourceId, client: NodeClient, timing: Timing, tx: mpsc::Sender<SourceEvent>) {
    let mut known: HashSet<TxId> = HashSet::new();
    let mut seen_tokens: HashSet<String> = HashSet::new();
    let mut token_queue: VecDeque<String> = VecDeque::new();
    let mut last_height: u32 = 0;
    let mut fails: u32 = 0;
    let mut indexed = false;
    let mut next_info = Instant::now();
    let mut next_headers = Instant::now();

    while !tx.is_closed() {
        if Instant::now() >= next_info {
            match client.info().await {
                Ok(info) => {
                    indexed = info.indexed_height.is_some();
                    send(&tx, SourceEvent::Status { source: id.clone(), status: node_status(&info) }).await;
                    send(&tx, SourceEvent::Info { source: id.clone(), info }).await;
                    next_info = Instant::now() + timing.node_info;
                }
                Err(e) => {
                    fails += 1;
                    send(&tx, SourceEvent::Status { source: id.clone(), status: SourceStatus::Down(e.to_string()) }).await;
                    sleep(backoff(fails)).await;
                    continue;
                }
            }
        }

        let started = Instant::now();
        let polled = match client.mempool_ids().await {
            Ok(ids) => {
                let new_ids: Vec<TxId> = ids.iter().filter(|i| !known.contains(*i)).cloned().collect();
                client.mempool_txs(&new_ids).await.map(|new_txs| (ids, new_ids, new_txs))
            }
            Err(e) => Err(e),
        };
        match polled {
            Ok((ids, new_ids, new_txs)) => {
                fails = 0;
                let returned: HashSet<&TxId> = new_txs.iter().map(|t| &t.id).collect();
                let missing: HashSet<&TxId> = new_ids.iter().filter(|i| !returned.contains(i)).collect();
                known = ids.iter().filter(|i| !missing.contains(i)).cloned().collect();
                if indexed {
                    for t in &new_txs {
                        for tok in t.outputs.iter().flat_map(|o| o.tokens.iter()) {
                            if seen_tokens.insert(tok.token_id.clone()) {
                                token_queue.push_back(tok.token_id.clone());
                            }
                        }
                    }
                }
                let latency_ms = ms(started.elapsed());
                send(&tx, SourceEvent::Mempool { source: id.clone(), ids, new_txs, latency_ms }).await;
            }
            Err(e) => {
                fails += 1;
                send(&tx, SourceEvent::Status { source: id.clone(), status: SourceStatus::Down(e.to_string()) }).await;
                next_info = Instant::now();
                sleep(backoff(fails)).await;
                continue;
            }
        }

        for _ in 0..TOKENS_PER_POLL {
            let Some(token_id) = token_queue.pop_front() else { break };
            if let Ok(meta) = client.token(&token_id).await {
                send(&tx, SourceEvent::TokenMeta(meta)).await;
            }
        }

        if Instant::now() >= next_headers {
            if let Ok(mut headers) = client.last_headers(BLOCKS_PER_POLL).await {
                headers.retain(|h| h.height > last_height);
                headers.sort_by_key(|h| h.height);
                for h in headers {
                    if let Ok(block) = client.block(&h).await {
                        last_height = h.height;
                        send(&tx, SourceEvent::Block { source: id.clone(), block }).await;
                    }
                }
            }
            next_headers = Instant::now() + timing.node_headers;
        }

        sleep(timing.node_mempool).await;
    }
}

pub async fn run_explorer(id: SourceId, client: ExplorerClient, timing: Timing, tx: mpsc::Sender<SourceEvent>) {
    let mut known: HashSet<TxId> = HashSet::new();
    let mut last_height: u32 = 0;
    let mut fails: u32 = 0;
    let mut next_blocks = Instant::now();

    while !tx.is_closed() {
        let started = Instant::now();
        match client.mempool().await {
            Ok(txs) => {
                fails = 0;
                let ids: Vec<TxId> = txs.iter().map(|t| t.id.clone()).collect();
                let new_txs = txs.into_iter().filter(|t| !known.contains(&t.id)).collect();
                known = ids.iter().cloned().collect();
                let latency_ms = ms(started.elapsed());
                send(&tx, SourceEvent::Mempool { source: id.clone(), ids, new_txs, latency_ms }).await;
            }
            Err(e) => {
                fails += 1;
                send(&tx, SourceEvent::Status { source: id.clone(), status: SourceStatus::Down(e.to_string()) }).await;
                sleep(backoff(fails)).await;
                continue;
            }
        }

        if Instant::now() >= next_blocks {
            if let Ok(mut refs) = client.latest_blocks(BLOCKS_PER_POLL).await {
                refs.retain(|r| r.height > last_height);
                refs.sort_by_key(|r| r.height);
                for r in refs {
                    if let Ok(block) = client.block(&r.id).await {
                        last_height = r.height;
                        send(&tx, SourceEvent::Block { source: id.clone(), block }).await;
                    }
                }
            }
            next_blocks = Instant::now() + timing.explorer_blocks;
        }

        sleep(timing.explorer_mempool).await;
    }
}

pub async fn run_address_book(
    http: reqwest::Client,
    base: String,
    cache: Option<PathBuf>,
    tx: mpsc::Sender<SourceEvent>,
) {
    let (entries, mut refresh) = addressbook::initial(cache.as_deref());
    send(&tx, SourceEvent::AddressBook(entries)).await;
    while !tx.is_closed() {
        if refresh {
            match addressbook::fetch(&http, &base).await {
                Ok((raw, entries)) => {
                    if let Some(path) = &cache {
                        let _ = addressbook::save_cache(path, &raw);
                    }
                    send(&tx, SourceEvent::AddressBook(entries)).await;
                }
                Err(e) => tracing::warn!("address book refresh failed: {e}"),
            }
        }
        sleep(BOOK_REFRESH).await;
        refresh = true;
    }
}

pub async fn run_price(http: reqwest::Client, tx: mpsc::Sender<SourceEvent>) {
    while !tx.is_closed() {
        if let Ok(p) = price::fetch_price(&http, price::PRICE_URL).await {
            send(&tx, SourceEvent::Price(p)).await;
        }
        sleep(PRICE_EVERY).await;
    }
}

/// Spawns one task per source plus address book and price. Call inside a tokio runtime.
pub fn spawn_all(specs: &[SourceSpec], timing: Timing, cache_dir: Option<PathBuf>) -> mpsc::Receiver<SourceEvent> {
    let (tx, rx) = mpsc::channel(1024);
    let http = http_client();
    for s in specs {
        match s.kind {
            SourceKind::Node => {
                tokio::spawn(run_node(s.id.clone(), NodeClient::new(http.clone(), &s.url), timing, tx.clone()));
            }
            SourceKind::Explorer => {
                tokio::spawn(run_explorer(s.id.clone(), ExplorerClient::new(http.clone(), &s.url), timing, tx.clone()));
            }
        }
    }
    let cache = addressbook::default_cache_path(cache_dir);
    tokio::spawn(run_address_book(http.clone(), addressbook::BOOK_API.to_string(), cache, tx.clone()));
    tokio::spawn(run_price(http, tx));
    rx
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop-core sources::runtime`
Expected: 6 tests PASS.

- [ ] **Step 5: Run the whole core suite**

Run: `cargo test -p ergotop-core`
Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ergotop-core/src/sources
git commit -m "feat(core): concurrent source poll loops with backoff

<trailer>"
```

---

### Task 11: Gravity packing

**Files:**
- Create: `crates/ergotop-core/src/packing.rs`
- Modify: `crates/ergotop-core/src/lib.rs` (add `pub mod packing;`)

**Interfaces:**
- Consumes: `model::TxId`.
- Produces (`ergotop_core::packing`):
  - `pub struct PackItem { pub id: TxId, pub size_bytes: u32, pub fee: u64 }`
  - `pub enum Region { Block, Overflow }`, `pub enum Shape { Rect, Hexagon }` (Copy, Debug, PartialEq, Eq)
  - `pub struct Placed { pub id: TxId, pub x: u16, pub y: u16, pub side: u16, pub region: Region }` — pixel units, `y = 0` is the bottom of the block region; overflow sits above `block_height`
  - `pub struct PackParams { pub width: u16, pub block_height: u16, pub overflow_height: u16, pub capacity_bytes: u32, pub max_side: u16, pub shape: Shape }` (Copy)
  - `pub struct PackResult { pub placed: Vec<Placed>, pub block_bytes: u64, pub block_count: usize, pub not_shown: usize }` (Default)
  - `pub fn side_for(size_bytes: u32, max_side: u16) -> u16` — v1 formula: `1 + (max_side-1) * sqrt(min(size/20000, 1))`, rounded
  - `pub fn pack(items: &[PackItem], p: &PackParams) -> PackResult`

Algorithm: choose next-block txs greedily by fee per byte (desc, ties by id) while cumulative bytes ≤ capacity; the rest are overflow. In each region place items largest-first (ties by id) using a skyline: for each x, y = max skyline over the item's columns; choose the lowest y, then leftmost x, where the square fits the region mask. Hexagon mask (block region only): a flat-topped hexagon whose left/right quarter columns are inset vertically.

- [ ] **Step 1: Write the failing tests**

`crates/ergotop-core/src/packing.rs`:

```rust
//! Gravity (skyline) packing of mempool txs, ported from Ergomempool v1 PackingAlgorithm.js.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn item(id: &str, size: u32, fee: u64) -> PackItem {
        PackItem { id: id.into(), size_bytes: size, fee }
    }

    fn params(width: u16, block_height: u16, capacity: u32, shape: Shape) -> PackParams {
        PackParams { width, block_height, overflow_height: 20, capacity_bytes: capacity, max_side: 6, shape }
    }

    #[test]
    fn side_follows_v1_curve() {
        assert_eq!(side_for(0, 6), 1);
        assert_eq!(side_for(20_000, 6), 6);
        assert_eq!(side_for(1_000_000, 6), 6);
        assert_eq!(side_for(5_000, 6), 4); // 1 + 5*sqrt(0.25) = 3.5 -> 4
    }

    #[test]
    fn selects_block_by_fee_rate() {
        let items = [item("a", 600, 6000), item("b", 600, 600), item("c", 300, 3000)];
        let r = pack(&items, &params(40, 40, 1000, Shape::Rect));
        let region = |id: &str| r.placed.iter().find(|p| p.id == id).unwrap().region;
        assert_eq!(region("a"), Region::Block);
        assert_eq!(region("c"), Region::Block);
        assert_eq!(region("b"), Region::Overflow);
        assert_eq!(r.block_bytes, 900);
        assert_eq!(r.block_count, 2);
    }

    #[test]
    fn gravity_fills_bottom_row_left_to_right() {
        let items = [item("a", 1, 1), item("b", 1, 1)];
        let p = PackParams { width: 4, block_height: 4, overflow_height: 0, capacity_bytes: 100, max_side: 2, shape: Shape::Rect };
        let r = pack(&items, &p);
        let pos: Vec<(u16, u16)> = r.placed.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(pos, vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn items_that_do_not_fit_are_counted() {
        let items: Vec<PackItem> = (0..10).map(|i| item(&format!("t{i}"), 20_000, 1)).collect();
        let p = PackParams { width: 6, block_height: 6, overflow_height: 0, capacity_bytes: u32::MAX, max_side: 6, shape: Shape::Rect };
        let r = pack(&items, &p);
        assert_eq!(r.placed.len(), 1);
        assert_eq!(r.not_shown, 9);
    }

    fn overlaps(a: &Placed, b: &Placed) -> bool {
        a.x < b.x + b.side && b.x < a.x + a.side && a.y < b.y + b.side && b.y < a.y + a.side
    }

    proptest! {
        #[test]
        fn packing_invariants(
            sizes in proptest::collection::vec((1u32..30_000, 0u64..100_000), 0..300),
            width in 8u16..120,
            block_height in 8u16..60,
            capacity in 1_000u32..2_000_000,
            hex in any::<bool>(),
        ) {
            let items: Vec<PackItem> = sizes.iter().enumerate().map(|(i, (s, f))| item(&format!("t{i}"), *s, *f)).collect();
            let shape = if hex { Shape::Hexagon } else { Shape::Rect };
            let p = PackParams { width, block_height, overflow_height: 20, capacity_bytes: capacity, max_side: 6, shape };
            let r = pack(&items, &p);
            prop_assert_eq!(r.placed.len() + r.not_shown, items.len());
            prop_assert!(r.block_bytes <= capacity as u64);
            for (i, a) in r.placed.iter().enumerate() {
                prop_assert!(a.x + a.side <= width);
                match a.region {
                    Region::Block => prop_assert!(a.y + a.side <= block_height),
                    Region::Overflow => prop_assert!(a.y >= block_height && a.y + a.side <= block_height + 20),
                }
                for b in &r.placed[i + 1..] {
                    prop_assert!(!overlaps(a, b), "{:?} overlaps {:?}", a, b);
                }
            }
        }
    }
}
```

Add `pub mod packing;` to `lib.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop-core packing`
Expected: FAIL to compile — `pack`, `PackItem`, … not found.

- [ ] **Step 3: Implement**

Insert above the tests module:

```rust
use crate::model::TxId;

const V1_NORMALIZE_BYTES: f64 = 20_000.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackItem {
    pub id: TxId,
    pub size_bytes: u32,
    pub fee: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Block,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Hexagon,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: TxId,
    pub x: u16,
    pub y: u16,
    pub side: u16,
    pub region: Region,
}

#[derive(Clone, Copy, Debug)]
pub struct PackParams {
    pub width: u16,
    pub block_height: u16,
    pub overflow_height: u16,
    pub capacity_bytes: u32,
    pub max_side: u16,
    pub shape: Shape,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackResult {
    pub placed: Vec<Placed>,
    pub block_bytes: u64,
    pub block_count: usize,
    pub not_shown: usize,
}

pub fn side_for(size_bytes: u32, max_side: u16) -> u16 {
    let norm = (size_bytes as f64 / V1_NORMALIZE_BYTES).min(1.0);
    let max = max_side.max(1) as f64;
    (1.0 + (max - 1.0) * norm.sqrt()).round() as u16
}

fn fee_rate(i: &PackItem) -> u128 {
    i.fee as u128 * 1000 / i.size_bytes.max(1) as u128
}

/// Per-column [lo, hi) vertical bounds of a region.
fn mask(width: u16, height: u16, shape: Shape) -> (Vec<u16>, Vec<u16>) {
    let w = width as usize;
    match shape {
        Shape::Rect => (vec![0; w], vec![height; w]),
        Shape::Hexagon => {
            let q = (width as f64 / 4.0).max(1.0);
            let half = height as f64 / 2.0;
            let mut lo = vec![0; w];
            let mut hi = vec![height; w];
            for x in 0..w {
                let xc = x as f64 + 0.5;
                let inset = if xc < q {
                    (q - xc) / q
                } else if xc > width as f64 - q {
                    (xc - (width as f64 - q)) / q
                } else {
                    0.0
                };
                let margin = (inset * half).ceil() as u16;
                lo[x] = margin.min(height);
                hi[x] = height.saturating_sub(margin).max(lo[x]);
            }
            (lo, hi)
        }
    }
}

/// Places (index, side) items into one region; returns (index, x, y) and the count not placed.
fn pack_region(items: &[(usize, u16)], width: u16, height: u16, shape: Shape) -> (Vec<(usize, u16, u16)>, usize) {
    let (lo, hi) = mask(width, height, shape);
    let mut sky = lo.clone();
    let mut out = Vec::with_capacity(items.len());
    let mut not_shown = 0;
    for &(idx, side) in items {
        if side == 0 || side > width {
            not_shown += 1;
            continue;
        }
        let mut best: Option<(u16, u16)> = None;
        for x in 0..=(width - side) {
            let cols = x as usize..(x + side) as usize;
            let y = cols.clone().map(|c| sky[c]).max().unwrap_or(0);
            let fits = cols.clone().all(|c| y >= lo[c] && y + side <= hi[c]);
            if fits && best.map_or(true, |(by, bx)| (y, x) < (by, bx)) {
                best = Some((y, x));
            }
        }
        match best {
            Some((y, x)) => {
                for c in x as usize..(x + side) as usize {
                    sky[c] = y + side;
                }
                out.push((idx, x, y));
            }
            None => not_shown += 1,
        }
    }
    (out, not_shown)
}

pub fn pack(items: &[PackItem], p: &PackParams) -> PackResult {
    let mut by_rate: Vec<usize> = (0..items.len()).collect();
    by_rate.sort_by(|&a, &b| fee_rate(&items[b]).cmp(&fee_rate(&items[a])).then_with(|| items[a].id.cmp(&items[b].id)));

    let mut block = Vec::new();
    let mut overflow = Vec::new();
    let mut block_bytes: u64 = 0;
    for i in by_rate {
        let size = items[i].size_bytes as u64;
        if block_bytes + size <= p.capacity_bytes as u64 {
            block_bytes += size;
            block.push(i);
        } else {
            overflow.push(i);
        }
    }

    let sized = |idxs: &[usize]| {
        let mut v: Vec<(usize, u16)> = idxs.iter().map(|&i| (i, side_for(items[i].size_bytes, p.max_side))).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| items[a.0].id.cmp(&items[b.0].id)));
        v
    };

    let mut result = PackResult { block_bytes, block_count: block.len(), ..Default::default() };
    let (placed, hidden) = pack_region(&sized(&block), p.width, p.block_height, p.shape);
    result.not_shown += hidden;
    for (i, x, y) in placed {
        result.placed.push(Placed { id: items[i].id.clone(), x, y, side: side_for(items[i].size_bytes, p.max_side), region: Region::Block });
    }
    let (placed, hidden) = pack_region(&sized(&overflow), p.width, p.overflow_height, Shape::Rect);
    result.not_shown += hidden;
    for (i, x, y) in placed {
        result.placed.push(Placed {
            id: items[i].id.clone(),
            x,
            y: y + p.block_height,
            side: side_for(items[i].size_bytes, p.max_side),
            region: Region::Overflow,
        });
    }
    result
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p ergotop-core packing`
Expected: 5 tests PASS (4 unit + 1 proptest).

- [ ] **Step 5: Commit**

```bash
git add crates/ergotop-core/src/packing.rs crates/ergotop-core/src/lib.rs
git commit -m "feat(core): fee-rate block selection and gravity packing

<trailer>"
```

---

### Task 12: Headless mode and live check

**Files:**
- Create: `crates/ergotop/src/headless.rs`
- Modify: `crates/ergotop/src/main.rs`

**Interfaces:**
- Consumes: everything in `ergotop_core` (`config::*`, `classify::*`, `reconcile::*`, `sources::{SourceEvent, runtime::{spawn_all, Timing}}`, `model::nano_to_erg`).
- Produces (binary-internal):
  - `headless::tx_line(e: &TxEntry) -> String`
  - `headless::sources_line(views: &[SourceView], active: Option<&SourceId>) -> String`
  - `headless::run(cfg: Config, addrs: AddressesFile) -> anyhow::Result<()>`
  - CLI: `ergotop [--config <dir>] [--headless]`

Output format:
- tx line: `+ {first 8 of id} {class name, padded to 16} fee {fee ERG, 4 dp} value {value ERG, 2 dp}{"~" if approx} {size} B`
- sources line: `sources: {id} {● if usable else ○}{"*" if active} {latency}ms {n}tx | ...` (`-` when unknown)

- [ ] **Step 1: Write the failing tests**

`crates/ergotop/src/headless.rs`:

```rust
//! `--headless`: print reconciled mempool activity as text lines.

#[cfg(test)]
mod tests {
    use super::*;
    use ergotop_core::classify::{Classification, Kind, Rgb, TxClass};
    use ergotop_core::metrics::TxMetrics;
    use ergotop_core::model::{SourceKind, SourceStatus, Tx};
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn formats_tx_line() {
        let e = TxEntry {
            tx: Tx { id: "abcdef0123456789".into(), size: 412, inputs: vec![], outputs: vec![], creation_ts_ms: None },
            first_seen_ms: 0,
            seen_by: BTreeSet::new(),
            class: TxClass { class: Classification { name: "Spectrum".into(), kind: Kind::Service, color: Rgb(0, 0, 0) }, from: None },
            metrics: TxMetrics { fee: 1_500_000, value: 11_880_000_000, approx: true },
        };
        assert_eq!(tx_line(&e), "+ abcdef01 Spectrum         fee 0.0015 value 11.88~ 412 B");
    }

    #[test]
    fn formats_sources_line() {
        let view = |id: &str, status: SourceStatus, latency: Option<u64>, n: usize| SourceView {
            id: SourceId(id.into()),
            kind: SourceKind::Node,
            status,
            ids: (0..n).map(|i| i.to_string()).collect::<HashSet<_>>(),
            last_update_ms: None,
            latency_ms: latency,
            info: None,
        };
        let views = vec![view("node-a", SourceStatus::Up, Some(23), 2), view("p2p", SourceStatus::Down("x".into()), None, 0)];
        assert_eq!(
            sources_line(&views, Some(&SourceId("node-a".into()))),
            "sources: node-a ●* 23ms 2tx | p2p ○ -ms 0tx"
        );
    }
}
```

Replace `crates/ergotop/src/main.rs` with:

```rust
mod headless;

use std::path::PathBuf;

use clap::Parser;
use ergotop_core::config::{config_dir, load_from_dir, AddressesFile, Config};

#[derive(Parser)]
#[command(version, about = "Real-time Ergo mempool visualizer")]
struct Args {
    /// Directory containing ergotop.toml and addresses.toml
    #[arg(long)]
    config: Option<PathBuf>,
    /// Print mempool activity as text instead of the TUI
    #[arg(long)]
    headless: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let (mut cfg, addrs, warnings) = match args.config.clone().or_else(config_dir) {
        Some(dir) => load_from_dir(&dir),
        None => (Config::default(), AddressesFile::default(), vec![]),
    };
    cfg.apply_env(std::env::var("ERGO_NODE_URL").ok(), std::env::var("ERGO_API_URL").ok());
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    if !args.headless {
        eprintln!("The TUI arrives in Plan 2; run with --headless for now.");
        return Ok(());
    }
    headless::run(cfg, addrs).await
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ergotop`
Expected: FAIL to compile — `tx_line`, `sources_line`, `run` not found.

- [ ] **Step 3: Implement**

Insert above the tests module in `headless.rs`:

```rust
use std::time::{SystemTime, UNIX_EPOCH};

use ergotop_core::classify::{BookEntry, Builtin, Classifier};
use ergotop_core::config::{cache_dir, AddressesFile, Config};
use ergotop_core::model::{nano_to_erg, SourceId};
use ergotop_core::reconcile::{Reconciler, SourceView, TxEntry, Update};
use ergotop_core::sources::runtime::{spawn_all, Timing};
use ergotop_core::sources::SourceEvent;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn tx_line(e: &TxEntry) -> String {
    let id: String = e.tx.id.chars().take(8).collect();
    format!(
        "+ {id} {:<16} fee {:.4} value {:.2}{} {} B",
        e.class.class.name,
        nano_to_erg(e.metrics.fee),
        nano_to_erg(e.metrics.value),
        if e.metrics.approx { "~" } else { "" },
        e.tx.size
    )
}

pub fn sources_line(views: &[SourceView], active: Option<&SourceId>) -> String {
    let parts: Vec<String> = views
        .iter()
        .map(|v| {
            format!(
                "{} {}{} {}ms {}tx",
                v.id,
                if v.status.usable() { "●" } else { "○" },
                if Some(&v.id) == active { "*" } else { "" },
                v.latency_ms.map(|l| l.to_string()).unwrap_or_else(|| "-".into()),
                v.ids.len()
            )
        })
        .collect();
    format!("sources: {}", parts.join(" | "))
}

fn describe(rec: &Reconciler, cls: &Classifier, u: &Update) -> Vec<String> {
    match u {
        Update::Added(ids) => ids.iter().filter_map(|id| rec.pool().get(id)).map(tx_line).collect(),
        Update::Mined { height, tx_ids } => vec![format!("⛏ block {height}: {} mempool txs mined", tx_ids.len())],
        Update::Dropped(ids) => vec![format!("- dropped {}: {}", ids.len(), ids.join(", "))],
        Update::Resynced => vec![format!(
            "= resynced on {}: {} txs",
            rec.active().map(|s| s.0.as_str()).unwrap_or("none"),
            rec.pool().len()
        )],
        Update::BlockAdded(h) => {
            let miner = rec
                .recent_blocks()
                .into_iter()
                .find(|b| b.height == *h)
                .and_then(|b| b.miner_address.as_deref())
                .and_then(|a| cls.lookup(a))
                .map(|c| c.name)
                .unwrap_or_else(|| "Other".into());
            vec![format!("# block {h} by {miner}")]
        }
        Update::SourcesChanged => vec![sources_line(rec.views(), rec.active())],
    }
}

pub async fn run(cfg: Config, addrs: AddressesFile) -> anyhow::Result<()> {
    let specs = cfg.sources();
    let mut rx = spawn_all(&specs, Timing::default(), cache_dir());
    let builtin = Builtin::load();
    let mut book: Vec<BookEntry> = Vec::new();
    let mut cls = Classifier::new(&builtin, &book, &addrs.address);
    let mut rec = Reconciler::new(specs.iter().map(|s| (s.id.clone(), s.kind)).collect());

    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    SourceEvent::AddressBook(entries) => {
                        book = entries;
                        cls = Classifier::new(&builtin, &book, &addrs.address);
                        rec.reclassify(&cls);
                        println!("address book: {} entries", book.len());
                    }
                    SourceEvent::Price(p) => println!("price: ${p:.4}"),
                    SourceEvent::TokenMeta(_) => {}
                    other => {
                        for u in rec.apply(other, now_ms(), &cls) {
                            for line in describe(&rec, &cls, &u) {
                                println!("{line}");
                            }
                        }
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: all tests PASS (core + 2 headless tests).

- [ ] **Step 5: Lint**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. Fix any clippy findings in place.

- [ ] **Step 6: Live check against explorers**

Run: `ERGO_NODE_URL=http://127.0.0.1:1 cargo run -p ergotop -- --headless` (PowerShell: `$env:ERGO_NODE_URL='http://127.0.0.1:1'; cargo run -p ergotop -- --headless`), let it run ~60s, then Ctrl+C.

Expected, in order: an `address book: N entries` line (snapshot), a `sources:` line showing `127.0.0.1:1 ○`, then `= resynced on p2p: N txs` (or `public`), `# block … by …` lines, `+ …` tx lines as new txs arrive, and a second `address book:` line after the live fetch. No panics.

- [ ] **Step 7: Live check against the user's LAN node**

Ask the user for one node URL (they keep nodes in a local config file). Write it to a temp config dir and run:

```bash
mkdir -p /tmp/ergotop-cfg && printf '[[node]]\nurl = "<NODE_URL>"\nname = "lan"\n' > /tmp/ergotop-cfg/ergotop.toml
cargo run -p ergotop -- --headless --config /tmp/ergotop-cfg
```

Expected: `sources:` line shows `lan ●*` (active) with latency well under 1000ms; `= resynced on lan: N txs`; `# block` lines name pools from the address book; when a block is mined, a `⛏ block H: K mempool txs mined` line appears and K matches the block's non-coinbase tx count shown by the explorer. Note any endpoint that fails (especially `byTransactionIds` and `/utxo/withPool/byIds`) and record actual response shapes as new fixtures if they differ.

- [ ] **Step 8: Commit**

```bash
git add crates/ergotop
git commit -m "feat: headless mode printing reconciled mempool activity

<trailer>"
```

---

## Self-Review Notes

- Spec coverage: §3.1 sources → Tasks 6–8, 10; §3.2 reconciler → Task 9; §3.3 metrics → Task 3; §3.4 classification → Tasks 2, 5; §5 configuration → Task 4; §6 error handling (sources never block; bad config → warning) → Tasks 4, 10; §7 testing → per task + proptest in Task 11; packing algorithm from §4.2 → Task 11. TUI (§4), animation, benchmark, CI/release (§8), migration (§9) are Plans 2 and 3.
- `--log` / tracing subscriber (spec §6) is deferred to Plan 2, where it matters (TUI owns the terminal). Core already emits `tracing` events.
