"""Recent blocks display."""
import time
from textual.widgets import Static

from ergo_mempool_tui.api.models import Block


class BlocksPanel(Static):
    """Shows last 4 blocks with miner, TX count, and time ago."""

    _blocks: list[Block] = []

    def update_blocks(self, blocks: list[Block]) -> None:
        self._blocks = blocks
        self.refresh()

    def render(self) -> str:
        lines = ["  RECENT BLOCKS"]
        if not self._blocks:
            lines.append("  Loading...")
            return "\n".join(lines)

        now_ms = time.time() * 1000
        for b in self._blocks[:4]:
            ago_s = (now_ms - b.timestamp) / 1000
            if ago_s < 60:
                ago = f"{ago_s:.0f}s"
            elif ago_s < 3600:
                ago = f"{ago_s / 60:.0f}m"
            else:
                ago = f"{ago_s / 3600:.1f}h"

            miner = b.miner[:10] if len(b.miner) > 10 else b.miner
            lines.append(f"  #{b.height} {miner:<10} {b.tx_count:>2}tx {ago:>4}")

        return "\n".join(lines)
