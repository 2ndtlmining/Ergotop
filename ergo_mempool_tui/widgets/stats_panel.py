"""Left column: block utilization bar and mempool summary."""
import time
from rich.text import Text
from textual.widgets import Static
from textual.app import ComposeResult
from textual.containers import Vertical
from textual.reactive import reactive

from ergo_mempool_tui.api.models import Transaction
from ergo_mempool_tui.config import BLOCK_SIZE, COLORS


class UtilizationBar(Static):
    """Horizontal utilization bar showing block fullness."""

    utilization = reactive(0.0)

    def render(self) -> Text:
        pct = self.utilization * 100
        bar_width = 22
        filled = int(pct / 100 * bar_width)

        if pct >= 95:
            bar_color = COLORS["error"]
            label = " RED"
        elif pct >= 80:
            bar_color = COLORS["warning"]
            label = " WARN"
        else:
            bar_color = COLORS["primary"]
            label = ""

        total_bytes = self.utilization * BLOCK_SIZE
        if total_bytes >= 1024 * 1024:
            size_str = f"{total_bytes / 1024 / 1024:.1f} MB"
        elif total_bytes >= 1024:
            size_str = f"{total_bytes / 1024:.0f} KB"
        else:
            size_str = f"{total_bytes:.0f} B"

        result = Text()
        result.append("  BLOCK UTILIZATION\n")
        result.append("  (")
        result.append("=" * filled, style=bar_color)
        result.append("-" * (bar_width - filled), style=COLORS["dim"])
        result.append(f") {pct:.0f}%")
        if label:
            result.append(label, style=bar_color)
        result.append(f"\n  {size_str} / 2.0 MB")
        return result


class MempoolSummary(Static):
    """Aggregate mempool stats: total fees, avg fee, avg size, largest TX, oldest TX."""

    _txs: list[Transaction] = []

    def update_txs(self, txs: list[Transaction]) -> None:
        self._txs = txs
        self.refresh()

    def render(self) -> str:
        lines = ["  MEMPOOL SUMMARY"]
        if not self._txs:
            lines.append("  No transactions")
            return "\n".join(lines)

        total_fees = sum(t.fee for t in self._txs)
        avg_fee = total_fees / len(self._txs)

        avg_size = sum(t.size for t in self._txs) / len(self._txs)
        largest = max(t.size for t in self._txs)

        if largest >= 1024:
            largest_str = f"{largest / 1024:.1f} KB"
        else:
            largest_str = f"{largest} B"

        now = time.time()
        oldest_ts = min(t.timestamp for t in self._txs)
        oldest_age = now - oldest_ts
        if oldest_age >= 3600:
            age_str = f"{oldest_age / 3600:.1f}h"
        elif oldest_age >= 60:
            mins = int(oldest_age // 60)
            secs = int(oldest_age % 60)
            age_str = f"{mins}m {secs:02d}s"
        else:
            age_str = f"{oldest_age:.0f}s"

        lines.append(f"  Total Fees  {total_fees:>10.4f} ERG")
        lines.append(f"  Avg Fee     {avg_fee:>10.4f} ERG")
        lines.append(f"  Avg Size    {avg_size:>10.0f} B")
        lines.append(f"  Largest TX  {largest_str:>10}")
        lines.append(f"  Oldest TX   {age_str:>10}")

        return "\n".join(lines)


class StatsPanel(Vertical):
    """Left column with utilization bar and mempool summary."""

    def compose(self) -> ComposeResult:
        yield UtilizationBar(id="utilization-bar")
        yield MempoolSummary(id="mempool-summary")
