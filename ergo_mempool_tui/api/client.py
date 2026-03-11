"""Async HTTP client for Ergo APIs with p2p-first fallback."""
import json
import time
import asyncio
from typing import Optional

import aiohttp

from ergo_mempool_tui.config import (
    API_PRIMARY, API_FALLBACK, API_TIMEOUT, NODE_URL, ORACLE_URL, FEE_ADDRESS,
    MINING_POOLS,
)
from ergo_mempool_tui.api.models import Transaction, TxIO, TokenInfo, Block, NodeInfo, MempoolDiff
from ergo_mempool_tui.detection.origins import detect_origin


class MempoolClient:
    """Fetches mempool, blocks, network info, and price from Ergo APIs."""

    def __init__(self, node_url: Optional[str] = None):
        self.node_url = node_url or NODE_URL
        self._session: Optional[aiohttp.ClientSession] = None
        self._tx_cache: dict[str, Transaction] = {}
        self._last_fetch: float = 0
        self._min_fetch_interval = 5  # seconds

    async def _get_session(self) -> aiohttp.ClientSession:
        if self._session is None or self._session.closed:
            timeout = aiohttp.ClientTimeout(total=API_TIMEOUT)
            self._session = aiohttp.ClientSession(timeout=timeout)
        return self._session

    async def close(self):
        if self._session and not self._session.closed:
            await self._session.close()

    async def _fetch_json(self, url: str) -> dict | list | None:
        session = await self._get_session()
        try:
            async with session.get(url) as resp:
                if resp.status != 200:
                    return None
                return await resp.json()
        except Exception:
            return None

    async def _fetch_with_fallback(self, path: str) -> dict | list | None:
        result = await self._fetch_json(API_PRIMARY + path)
        if result is not None:
            return result
        return await self._fetch_json(API_FALLBACK + path)

    def _parse_tx(self, raw: dict) -> Transaction:
        outputs = raw.get("outputs") or []
        inputs = raw.get("inputs") or []

        fee_out = next((o for o in outputs if o.get("address") == FEE_ADDRESS), None)
        fee = fee_out["value"] / 1e9 if fee_out else 0.001

        total_value = sum(o.get("value", 0) for o in outputs) / 1e9

        def _parse_assets(box: dict) -> list[TokenInfo]:
            assets = []
            for a in box.get("assets", []):
                decimals = a.get("decimals", 0)
                raw_amount = a.get("amount", 0)
                amount = raw_amount / (10 ** decimals) if decimals else raw_amount
                assets.append(TokenInfo(
                    token_id=a.get("tokenId", ""),
                    name=a.get("name", ""),
                    amount=amount,
                    decimals=decimals,
                ))
            return assets

        tx = Transaction(
            id=raw.get("id", ""),
            size=raw.get("size", 0),
            fee=max(fee, 0),
            value=max(total_value, 0),
            inputs=[TxIO(address=i.get("address", ""), value=i.get("value", 0) / 1e9, assets=_parse_assets(i)) for i in inputs],
            outputs=[TxIO(address=o.get("address", ""), value=o.get("value", 0) / 1e9, assets=_parse_assets(o)) for o in outputs],
            timestamp=time.time(),
        )

        origin_name, origin_color = detect_origin(tx)
        tx.origin = origin_name
        tx.origin_color = origin_color
        return tx

    async def fetch_mempool(self) -> MempoolDiff:
        now = time.time()

        # Rate-limit fetches
        if now - self._last_fetch < self._min_fetch_interval and self._tx_cache:
            return MempoolDiff(
                transactions=list(self._tx_cache.values()),
                added=[],
                removed=[],
                changed=False,
            )

        path = "/transactions/unconfirmed?limit=10000&offset=0"
        data = await self._fetch_with_fallback(path)
        if data is None:
            return MempoolDiff(
                transactions=list(self._tx_cache.values()),
                added=[],
                removed=[],
                changed=False,
            )

        items = data if isinstance(data, list) else data.get("items") or data.get("data") or []
        live_ids = {tx["id"] for tx in items if "id" in tx}
        cached_ids = set(self._tx_cache.keys())

        # Guard: if API returns empty but we had many cached TXs,
        # treat it as a transient glitch and keep the cache.
        if not live_ids and len(cached_ids) > 5:
            return MempoolDiff(
                transactions=list(self._tx_cache.values()),
                added=[],
                removed=[],
                changed=False,
            )

        # Compute diff
        removed_ids = list(cached_ids - live_ids)
        new_ids = live_ids - cached_ids

        # Evict confirmed/dropped
        for txid in removed_ids:
            del self._tx_cache[txid]

        # Parse only new transactions
        added = []
        for raw in items:
            txid = raw.get("id", "")
            if txid and txid in new_ids:
                tx = self._parse_tx(raw)
                self._tx_cache[txid] = tx
                added.append(tx)

        self._last_fetch = now
        changed = bool(removed_ids or added)

        return MempoolDiff(
            transactions=list(self._tx_cache.values()),
            added=added,
            removed=removed_ids,
            changed=changed,
        )

    async def fetch_blocks(self) -> list[Block]:
        data = await self._fetch_with_fallback("/api/v1/blocks?limit=4")
        if data is None:
            return []

        items = data.get("items", []) if isinstance(data, dict) else []
        blocks = []
        for b in items:
            miner_info = b.get("miner") or {}
            miner_addr = miner_info.get("address", "")
            api_name = miner_info.get("name") or ""
            # API returns last 8 chars of address as "name" for unknown miners - ignore those
            if api_name and miner_addr and miner_addr.endswith(api_name):
                api_name = ""
            miner_name = api_name or MINING_POOLS.get(miner_addr) or "Other"
            blocks.append(Block(
                height=b.get("height", 0),
                id=b.get("id", ""),
                miner=miner_name,
                miner_address=miner_info.get("address", ""),
                tx_count=b.get("transactionsCount", 0),
                size=b.get("size", 0),
                miner_reward=b.get("minerReward", 0) / 1e9,
                timestamp=b.get("timestamp", 0),
            ))
        return blocks

    async def fetch_network_state(self) -> NodeInfo:
        # Try local node first for richer data
        node_data = await self._fetch_json(f"{self.node_url}/info")
        if node_data:
            return NodeInfo(
                height=node_data.get("fullHeight", 0),
                peer_count=node_data.get("peersCount", 0),
                unconfirmed=node_data.get("unconfirmedCount", 0),
                difficulty=node_data.get("difficulty", 0),
                state_type=node_data.get("stateType", ""),
                node_version=node_data.get("appVersion", ""),
            )

        # Fallback to public API
        data = await self._fetch_with_fallback("/api/v1/networkState")
        if data:
            return NodeInfo(
                height=data.get("height", 0),
                difficulty=data.get("difficulty", 0),
            )
        return NodeInfo()

    async def fetch_price(self) -> float:
        session = await self._get_session()
        try:
            async with session.get(ORACLE_URL) as resp:
                text = await resp.text()
                # Oracle returns a JSON string wrapped in quotes
                if text.startswith('"') and text.endswith('"'):
                    text = text[1:-1]
                text = text.replace('\\"', '"')
                data = json.loads(text)
                return float(data.get("latest_price", 0))
        except Exception:
            return 0.0

    async def fetch_tx_detail(self, tx_id: str) -> Optional[Transaction]:
        data = await self._fetch_with_fallback(f"/api/v1/transactions/{tx_id}")
        if data is None:
            return None
        return self._parse_tx(data)
