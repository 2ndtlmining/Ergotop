"""Top status bar with block height, price, TX count, utilization."""
from textual.widgets import Static
from textual.reactive import reactive
from rich.text import Text


class HeaderBar(Static):
    """Full-width header bar showing key mempool stats."""

    height_val = reactive(0)
    price = reactive(0.0)
    tx_count = reactive(0)
    utilization = reactive(0.0)

    def render(self) -> Text:
        util_pct = self.utilization * 100
        filled = int(util_pct / 10)
        bar = "=" * filled + " " * (10 - filled)

        if util_pct >= 95:
            bar_color = "#ff4444"
        elif util_pct >= 80:
            bar_color = "#ffb000"
        else:
            bar_color = "#39ff14"

        result = Text()
        result.append(
            f"  ERGOTOP   "
            f"Block #{self.height_val:,}   "
            f"ERG ${self.price:.2f}   "
            f"{self.tx_count} TX   "
            f"UTIL ["
        )
        result.append(bar, style=bar_color)
        result.append(f"] {util_pct:.0f}%")
        return result
