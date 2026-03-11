"""Origin breakdown horizontal bar chart."""
from collections import Counter
from textual.widgets import Static
from rich.text import Text

from ergo_mempool_tui.api.models import Transaction
from ergo_mempool_tui.config import PLATFORM_COLORS


class OriginChart(Static):
    """Shows percentage breakdown of TX origins."""

    _transactions: list[Transaction] = []

    def update_txs(self, txs: list[Transaction]) -> None:
        self._transactions = txs
        self.refresh()

    def render(self) -> Text:
        result = Text()
        result.append("  ORIGIN BREAKDOWN\n")
        if not self._transactions:
            result.append("  No transactions")
            return result

        counts = Counter(tx.origin for tx in self._transactions)
        total = len(self._transactions)

        # Sort by count descending
        sorted_origins = counts.most_common()
        max_count = sorted_origins[0][1] if sorted_origins else 1

        for origin, count in sorted_origins:
            pct = count / total * 100
            bar_len = int(count / max_count * 8)
            bar = "=" * max(bar_len, 1)
            color = PLATFORM_COLORS.get(origin, "#556655")
            result.append(f"  {origin:<12} ")
            result.append(f"{bar:<8}", style=color)
            result.append(f" {pct:>3.0f}%\n")

        return result
