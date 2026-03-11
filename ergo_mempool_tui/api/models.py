"""Data models for mempool transactions, blocks, and network info."""
from dataclasses import dataclass, field
from typing import Optional


@dataclass
class TokenInfo:
    token_id: str = ""
    name: str = ""
    amount: float = 0.0
    decimals: int = 0


@dataclass
class TxIO:
    address: str = ""
    value: float = 0.0
    assets: list[TokenInfo] = field(default_factory=list)


@dataclass
class Transaction:
    id: str = ""
    size: int = 0
    fee: float = 0.001
    value: float = 0.0
    inputs: list[TxIO] = field(default_factory=list)
    outputs: list[TxIO] = field(default_factory=list)
    timestamp: float = 0.0
    origin: str = "Unknown"
    origin_color: str = "#556655"


@dataclass
class Block:
    height: int = 0
    id: str = ""
    miner: str = "Unknown"
    miner_address: str = ""
    tx_count: int = 0
    size: int = 0
    miner_reward: float = 0.0
    timestamp: int = 0


@dataclass
class NodeInfo:
    height: int = 0
    peer_count: int = 0
    unconfirmed: int = 0
    difficulty: float = 0.0
    state_type: str = ""
    node_version: str = ""


@dataclass
class MempoolDiff:
    transactions: list[Transaction]  # full current list
    added: list[Transaction]         # newly appeared
    removed: list[str]               # TX IDs that left
    changed: bool                    # False = nothing changed, skip redraws
