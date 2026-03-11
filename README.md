# Ergotop

A real-time terminal dashboard for the Ergo blockchain mempool -- like `htop` for Ergo. Built with Python and [Textual](https://textual.textualize.io/), it provides a three-column layout with live transaction tables, animated block visualizations, network stats, and platform origin detection.

## Features

- **Live mempool monitoring** -- polls unconfirmed transactions every 5 seconds with differential updates
- **Transaction table** -- sortable by fee, value, size, age, or origin; filterable by platform name or value range
- **Animated block visualizer** -- alternates between a block-packing grid and an ERG text logo, both filled with origin-colored TX segments; plays a drain animation when a new block is mined
- **Block utilization bar** -- color-coded percentage bar (green < 80%, amber 80-95%, red > 95%)
- **Mempool summary** -- total fees, average fee, average size, largest TX, oldest TX
- **Recent blocks** -- last 4 blocks with mining pool name, TX count, and time since mined
- **Network info** -- block height, node version, peer count, hashrate, mempool TX count
- **Origin breakdown** -- horizontal bar chart of transaction origin distribution
- **Platform detection** -- identifies transactions from 30+ platforms: Spectrum, Duckpools, SkyHarbor, SigUSD Bank, DexyUSD, Mew Finance, Crux Finance, Rosen Bridge, Ergomixer, CEX wallets (Kucoin, NonKyc, Gate, MEXC, CoinEx, and more), TokenJay, ErgoPad, mining pools, and others
- **TX detail view** -- inspect inputs, outputs, token transfers, and metadata for any transaction
- **New block celebration** -- header flash + toast showing miner name, TX count, and reward
- **Open in explorer** -- launch Ergo Explorer for any transaction directly from the TUI
- **Copy TX ID** -- copy to clipboard (requires pyperclip)
- **4 color themes** -- neon-green, amber-terminal, blue-ice, high-contrast; cycle with `Ctrl+T`
- **Differential caching** -- TX cache avoids re-parsing; widget updates batched and skipped when idle

## Quick Start

```bash
# Clone the repo
git clone https://github.com/2ndtlmining/Ergotop.git
cd Ergotop

# Install dependencies
pip install -r requirements.txt

# Run
python -m ergo_mempool_tui
```

### Optional

```bash
pip install pyperclip  # clipboard support for Copy TX ID
```

## Requirements

- Python 3.11+
- `textual` >= 0.47.0
- `aiohttp` >= 3.9.0
- `pyperclip` (optional)

No API keys or node setup required -- uses public Ergo APIs by default.

## Configuration

| Environment Variable | Default | Description |
|---|---|---|
| `ERGO_NODE_URL` | `http://213.239.193.208:9053` | Ergo node for network data (peer count, node version). Falls back to public API if unreachable. |
| `ERGO_API_URL` | `https://api-p2p.ergoplatform.com` | Primary API endpoint. Automatically falls back to `https://api.ergoplatform.com` on failure. |

```bash
# Example: use your own local node
ERGO_NODE_URL=http://localhost:9053 python -m ergo_mempool_tui
```

## Keyboard Shortcuts

| Key | Action |
|---|---|
| `Up` / `Down` | Navigate transaction list |
| `PgUp` / `PgDn` | Scroll by page |
| `Enter` | Show full TX detail (lazy-loaded from API) |
| `s` | Cycle sort: fee -> value -> size -> age -> origin |
| `/` | Open/close filter bar |
| `Escape` | Clear filter |
| `c` | Copy TX ID to clipboard |
| `e` | Open TX in Ergo Explorer |
| `Ctrl+T` | Cycle color theme |
| `?` | Help |
| `q` | Quit |

### Filtering

Press `/` then type:

- **Platform name** -- e.g. `spectrum`, `skyharbor` (also matches partial TX IDs)
- **Min value** -- `>100` shows TXs >= 100 ERG
- **Max value** -- `<1` shows TXs <= 1 ERG

Filtering is live as you type. `Escape` clears and closes.

## Color Themes

| Theme | Style |
|---|---|
| `neon-green` | Dark with bright green text and cyan accents (default) |
| `amber-terminal` | Retro amber/gold CRT look |
| `blue-ice` | Cool blue tones |
| `high-contrast` | Pure black/white for accessibility |

## Layout

```
+============================================================================+
| ERGOTOP  Block #1,738,799  ERG $0.33  51 TX  UTIL [====      ] 12%        |
+==========================+========================+========================+
|  BLOCK UTILIZATION       | NEXT BLOCK (43% full)  |   NETWORK INFO         |
|  [========----------] 42%| [####  ####  ##  ####] |  Height  1,738,799     |
|  856 KB / 2.0 MB         | [##  ####            ] |  Node         6.0.2    |
|                          |                        |  Peers   30            |
|  MEMPOOL SUMMARY         |                        |  Mempool TXs  51       |
|  Total Fees  0.0234 ERG  |   TRANSACTION LIST     +------------------------+
|  Avg Fee     0.0021 ERG  |   (scrollable table)   |  ORIGIN BREAKDOWN      |
|  Avg Size       412 B    |                        |  Spectrum  ====  35%   |
|  Largest TX    2.1 KB    | ID   Size Fee Val Orig |  P2P       ===   26%   |
|  Oldest TX     4m 12s    | a3f8 1.2K .002 145 Spc |  Contract  ==    17%   |
|                          | b7e2 0.4K .001  23 P2P |  Mew Fin   =      5%  |
+==========================+ c1d9 2.1K .005  89 Ctr +------------------------+
|  RECENT BLOCKS           | ...                    |  TX DETAIL             |
| #1738799 Herominers 12tx |                        |  ID: a3f8c2...e91b     |
| #1738798 LeafPool    8tx |                        |  Size: 1.2 KB          |
| #1738797 Nanopool    3tx |                        |  Fee: 0.0020 ERG       |
| #1738796 2Miners    18tx |                        |  Value: 145.00 ERG     |
+==========================+========================+========================+
| [Q]uit [S]ort [/]Filter [Enter]Detail [C]opy [E]xplorer [Ctrl+T]Theme [?] |
+============================================================================+
```

### Left Column
- **Block Utilization** -- mempool bytes vs. 2 MB block limit
- **Mempool Summary** -- aggregate stats (total fees, averages, extremes)
- **Recent Blocks** -- last 4 mined blocks with pool identification

### Center Column
- **TX Visualizer** -- animated block packing grid with ERG text logo transitions and drain effect
- **Transaction Table** -- sortable, filterable DataTable with differential row updates

### Right Column
- **Network Info** -- block height, node version, peers, hashrate, mempool count
- **Origin Breakdown** -- horizontal bar chart of TX platform distribution
- **TX Detail** -- inputs, outputs, and token transfers for the selected TX

## Origin Detection

Transactions are classified using a priority chain:

1. **Exact address match** -- known contract addresses for Spectrum, Duckpools, Mew Finance, Crux Finance, Rosen Bridge, SigUSD Bank, DexyUSD, Ergomixer, oracles, CEX wallets (Kucoin, NonKyc, Gate, MEXC, Huobi, CoinEx, TradeOgre, Xeggex, Probit, FluxSwap), TokenJay, ErgoRaffle, ErgoPad, Padeia, HODL ERG, Lilium, Gridbot, Pulse
2. **Address prefix match** -- script addresses matched by prefix (Spectrum, SkyHarbor, Rosen Bridge)
3. **P2P heuristic** -- all non-fee outputs to P2PK addresses (start with `9`, 51 chars)
4. **Contract heuristic** -- any non-fee output to a script address
5. **Unknown**

### Mining Pool Identification

Blocks show the pool name when the miner address matches: 2Miners, 2Miners Solo, Herominers, Kryptex, Nanopool, K1Pool, DXPool, SigPool, WoolyPooly, JJPool, Unmineable. Unknown miners show as "Other".

## API Endpoints

| Endpoint | Source | Interval |
|---|---|---|
| `/transactions/unconfirmed?limit=10000` | Ergo Platform API | 5s |
| `/api/v1/blocks?limit=4` | Ergo Platform API | 30s |
| `/info` | Ergo node | 10s |
| `/api/v1/networkState` | Ergo Platform API (fallback) | 10s |
| `/frontendData` | ERG/USD Oracle | 5m |
| `/api/v1/transactions/{id}` | Ergo Platform API | On demand |

The client tries `api-p2p.ergoplatform.com` first, falling back to `api.ergoplatform.com`. Timeout is 12 seconds.

## Performance

- **Differential updates** -- only changed table rows are added/removed, preserving scroll position
- **Incremental polling** -- diffs against an in-memory TX cache; only new TXs parsed
- **Batch rendering** -- all widget updates in a single `batch_update()` pass
- **Early skip** -- no redraws when nothing changed between polls
- **Selective parsing** -- cached TXs reused without re-running origin detection
- **Empty-mempool guard** -- prevents cache wipe on transient API glitches
- **Block-age check** -- celebrations only fire for blocks mined within 90 seconds

## Architecture

```
ergo_mempool_tui/
    __init__.py
    __main__.py          Entry point (python -m ergo_mempool_tui)
    app.py               Main Textual App, layout, keybindings, polling loops
    config.py            API URLs, poll intervals, themes, mining pools, platform colors

    api/
        client.py        Async HTTP with p2p-first fallback, TX cache, diff-based updates
        models.py        Dataclasses: Transaction, Block, NodeInfo, TxIO, TokenInfo, MempoolDiff

    detection/
        origins.py       Platform detection: contract addresses, prefix matching, heuristics

    styles/
        theme_css.py     Dynamic Textual CSS generation from color palettes

    widgets/
        header_bar.py    Top bar: block height, ERG price, TX count, utilization
        tx_table.py      Sortable/filterable DataTable with differential row updates
        tx_detail.py     TX detail: inputs, outputs, tokens
        stats_panel.py   UtilizationBar + MempoolSummary
        tx_visualizer.py Animated visualizer: block grid, ERG logo, drain effect
        blocks_panel.py  Recent blocks with miner name and TX count
        network_panel.py Node info: height, version, peers, hashrate
        origin_chart.py  Horizontal bar chart of TX origins
```

## License

MIT
