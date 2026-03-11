"""Transaction DataTable with sorting and filtering."""
import time
from rich.text import Text
from textual.widgets import DataTable
from textual.reactive import reactive

from ergo_mempool_tui.api.models import Transaction
from ergo_mempool_tui.config import PLATFORM_COLORS


class TxTable(DataTable):
    """Scrollable, sortable transaction table."""

    SORT_KEYS = ["fee", "value", "size", "age", "origin"]
    sort_index = reactive(0)
    filter_text = reactive("")

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self._transactions: list[Transaction] = []
        self._sorted_txs: list[Transaction] = []
        self._row_ids: set[str] = set()
        self._last_sort_key: str = "fee"
        self._last_filter: str = ""

    def on_mount(self) -> None:
        self.add_columns("TX ID", "Size", "Fee (ERG)", "Value (ERG)", "Origin", "Age")
        self.cursor_type = "row"
        self.zebra_stripes = True

    def update_transactions(self, txs: list[Transaction]) -> None:
        self._transactions = txs
        self._apply_sort_and_filter()

    def _apply_sort_and_filter(self) -> None:
        txs = list(self._transactions)

        # Apply filter
        if self.filter_text:
            ft = self.filter_text.lower()
            if ft.startswith(">"):
                try:
                    min_val = float(ft[1:])
                    txs = [t for t in txs if t.value >= min_val]
                except ValueError:
                    pass
            elif ft.startswith("<"):
                try:
                    max_val = float(ft[1:])
                    txs = [t for t in txs if t.value <= max_val]
                except ValueError:
                    pass
            else:
                txs = [t for t in txs if ft in t.origin.lower() or ft in t.id.lower()]

        # Apply sort
        sort_key = self.SORT_KEYS[self.sort_index % len(self.SORT_KEYS)]
        now = time.time()
        if sort_key == "fee":
            txs.sort(key=lambda t: t.fee, reverse=True)
        elif sort_key == "value":
            txs.sort(key=lambda t: t.value, reverse=True)
        elif sort_key == "size":
            txs.sort(key=lambda t: t.size, reverse=True)
        elif sort_key == "age":
            txs.sort(key=lambda t: t.timestamp)
        elif sort_key == "origin":
            txs.sort(key=lambda t: t.origin)

        self._sorted_txs = txs

        # If sort or filter changed, do full rebuild (to reorder)
        if sort_key != self._last_sort_key or self.filter_text != self._last_filter:
            self._last_sort_key = sort_key
            self._last_filter = self.filter_text
            self._rebuild_table(now)
        else:
            self._update_table_diff(now)

    def _format_age(self, age_s: float) -> str:
        if age_s < 60:
            return f"{age_s:.0f}s"
        elif age_s < 3600:
            return f"{age_s / 60:.0f}m"
        else:
            return f"{age_s / 3600:.1f}h"

    def _format_size(self, size: int) -> str:
        return f"{size / 1024:.1f}K" if size >= 1024 else f"{size}B"

    def _update_table_diff(self, now: float) -> None:
        """Differential update: add/remove only changed rows, update ages."""
        current_ids = {tx.id for tx in self._sorted_txs}

        # Remove rows no longer present
        stale = self._row_ids - current_ids
        for txid in stale:
            try:
                self.remove_row(row_key=txid)
            except Exception:
                pass
            self._row_ids.discard(txid)

        # Add new rows
        new_ids = current_ids - self._row_ids
        for tx in self._sorted_txs:
            if tx.id in new_ids:
                age_str = self._format_age(now - tx.timestamp)
                self.add_row(
                    tx.id[:8],
                    self._format_size(tx.size),
                    f"{tx.fee:.4f}",
                    f"{tx.value:.2f}",
                    Text(tx.origin, style=tx.origin_color),
                    age_str,
                    key=tx.id,
                )
                self._row_ids.add(tx.id)

        # Update age column for existing rows
        for tx in self._sorted_txs:
            if tx.id not in new_ids and tx.id in self._row_ids:
                age_str = self._format_age(now - tx.timestamp)
                try:
                    self.update_cell(tx.id, "Age", age_str)
                except Exception:
                    pass

    def _rebuild_table(self, now: float) -> None:
        self.clear()
        self._row_ids.clear()
        for tx in self._sorted_txs:
            age_str = self._format_age(now - tx.timestamp)
            self.add_row(
                tx.id[:8],
                self._format_size(tx.size),
                f"{tx.fee:.4f}",
                f"{tx.value:.2f}",
                Text(tx.origin, style=tx.origin_color),
                age_str,
                key=tx.id,
            )
            self._row_ids.add(tx.id)

    def cycle_sort(self) -> str:
        self.sort_index = (self.sort_index + 1) % len(self.SORT_KEYS)
        self._apply_sort_and_filter()
        return self.SORT_KEYS[self.sort_index]

    def set_filter(self, text: str) -> None:
        self.filter_text = text
        self._apply_sort_and_filter()

    def get_selected_tx(self) -> Transaction | None:
        if self.cursor_row is not None and 0 <= self.cursor_row < len(self._sorted_txs):
            try:
                coord = self.cursor_coordinate
                cell_key = self.coordinate_to_cell_key(coord)
                row_key_val = cell_key.row_key.value
                for tx in self._sorted_txs:
                    if tx.id == row_key_val:
                        return tx
            except Exception:
                pass
            # Fallback to index
            if self.cursor_row < len(self._sorted_txs):
                return self._sorted_txs[self.cursor_row]
        return None
