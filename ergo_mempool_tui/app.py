"""Main Textual App - Ergotop Dashboard."""
import inspect
import time
import webbrowser

from textual.app import App, ComposeResult
from textual.containers import Horizontal, Vertical
from textual.widgets import Static, Input, Footer, DataTable
from textual.binding import Binding
from textual.screen import ModalScreen
from rich.text import Text as RichText

def _footer_text(sort_mode: str = "", poll: int = 5) -> RichText:
    """Build footer with underlined keybinding chars."""
    t = RichText("  ")
    items = [
        ("Q", "uit"), ("S", f"ort:{sort_mode}" if sort_mode else "ort"),
        ("/", "Filter"), ("Enter", " Detail"),
        ("C", "opy"), ("E", "xplorer"), ("C-t", " Theme"), ("?", "Help"),
    ]
    for key, label in items:
        t.append(key, style="bold underline")
        t.append(label + " ")
    t.append(f"   Poll: {poll}s")
    return t


from ergo_mempool_tui.api.client import MempoolClient
from ergo_mempool_tui.api.models import Transaction
from ergo_mempool_tui.config import (
    BLOCK_SIZE, POLL_MEMPOOL, POLL_NETWORK, POLL_BLOCKS, POLL_PRICE,
    THEMES, set_theme, COLORS,
)
from ergo_mempool_tui.styles.theme_css import generate_css
from ergo_mempool_tui.widgets.header_bar import HeaderBar
from ergo_mempool_tui.widgets.tx_table import TxTable
from ergo_mempool_tui.widgets.tx_detail import TxDetail
from ergo_mempool_tui.widgets.stats_panel import StatsPanel, UtilizationBar, MempoolSummary
from ergo_mempool_tui.widgets.blocks_panel import BlocksPanel
from ergo_mempool_tui.widgets.network_panel import NetworkPanel
from ergo_mempool_tui.widgets.origin_chart import OriginChart
from ergo_mempool_tui.widgets.tx_visualizer import TxVisualizer


class HelpModal(ModalScreen[None]):
    """Help screen showing keyboard shortcuts."""

    BINDINGS = [
        Binding("escape", "dismiss", "Close"),
        Binding("question_mark", "dismiss", "Close"),
    ]

    def compose(self) -> ComposeResult:
        yield Static(
            "  KEYBOARD SHORTCUTS\n"
            "\n"
            "  Up/Down     Navigate TX list\n"
            "  PgUp/PgDn   Scroll by page\n"
            "  Enter       Show TX detail\n"
            "  s           Cycle sort mode\n"
            "  /           Open filter bar\n"
            "  Escape      Clear filter\n"
            "  c           Copy TX ID\n"
            "  e           Open in explorer\n"
            "  Ctrl+T      Cycle color theme\n"
            "  ?           This help\n"
            "  q           Quit\n"
            "\n"
            "  Sort modes: fee > value > size > age > origin\n"
            "  Filter: type name, >N or <N for value\n"
            "\n"
            "  Press Escape or ? to close",
            id="help-modal",
        )


class MempoolApp(App):
    """Ergotop - Real-time Ergo mempool visualization."""

    TITLE = "Ergotop"
    CSS = generate_css(THEMES["neon-green"])

    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("s", "sort", "Sort", show=True),
        Binding("slash", "filter", "Filter", show=True),
        Binding("escape", "clear_filter", "Clear", show=False),
        Binding("c", "copy_tx", "Copy ID", show=True),
        Binding("e", "explorer", "Explorer", show=True),
        Binding("ctrl+t", "cycle_theme", "Theme", show=True),
        Binding("question_mark", "help", "Help", show=True),
    ]

    _theme_names = list(THEMES.keys())
    _theme_index: int = 0

    def __init__(self, node_url: str | None = None):
        super().__init__()
        self.client = MempoolClient(node_url=node_url)
        self._transactions: list[Transaction] = []
        self._filter_visible = False
        self._current_sort = "fee"
        self._last_known_height: int = 0

    def compose(self) -> ComposeResult:
        yield HeaderBar(id="header-bar")
        with Horizontal(id="main-container"):
            with Vertical(id="left-column"):
                yield StatsPanel()
                yield BlocksPanel(id="blocks-panel")
            with Vertical(id="center-column"):
                yield Input(placeholder="Filter: name, >N or <N", id="filter-bar")
                yield TxVisualizer(id="tx-visualizer")
                yield TxTable(id="tx-table")
            with Vertical(id="right-column"):
                yield NetworkPanel(id="network-panel")
                yield OriginChart(id="origin-chart")
                yield TxDetail(id="tx-detail")
        yield Static(_footer_text(poll=POLL_MEMPOOL), id="footer-bar")

    async def on_mount(self) -> None:
        # Start polling loops
        self.set_interval(POLL_MEMPOOL, self._poll_mempool)
        self.set_interval(POLL_NETWORK, self._poll_network)
        self.set_interval(POLL_BLOCKS, self._poll_blocks)
        self.set_interval(POLL_PRICE, self._poll_price)

        # Initial fetch
        await self._poll_mempool()
        await self._poll_blocks()
        await self._poll_network()
        await self._poll_price()

    async def _poll_mempool(self) -> None:
        try:
            diff = await self.client.fetch_mempool()

            if not diff.changed and self._transactions:
                # Still update ages in table even when no diff
                table = self.query_one("#tx-table", TxTable)
                table.update_transactions(self._transactions)
                return

            txs = diff.transactions
            self._transactions = txs

            with self.batch_update():
                # Update header
                header = self.query_one("#header-bar", HeaderBar)
                header.tx_count = len(txs)
                total_bytes = sum(t.size for t in txs)
                header.utilization = min(total_bytes / BLOCK_SIZE, 1.0)

                # Update table
                table = self.query_one("#tx-table", TxTable)
                table.update_transactions(txs)

                # Update stats
                util_bar = self.query_one("#utilization-bar", UtilizationBar)
                util_bar.utilization = min(total_bytes / BLOCK_SIZE, 1.0)

                # Update origin chart
                origin_chart = self.query_one("#origin-chart", OriginChart)
                origin_chart.update_txs(txs)

                # Update TX visualizer
                visualizer = self.query_one("#tx-visualizer", TxVisualizer)
                visualizer.update_txs(txs)

                # Update mempool summary
                summary = self.query_one("#mempool-summary", MempoolSummary)
                summary.update_txs(txs)

                # Push mempool count to network panel
                network = self.query_one("#network-panel", NetworkPanel)
                network.mempool_count = len(txs)

                # Update footer with sort mode
                footer = self.query_one("#footer-bar", Static)
                footer.update(_footer_text(sort_mode=self._current_sort, poll=POLL_MEMPOOL))

        except Exception as e:
            self.notify(f"Mempool poll error: {e}", severity="error")

    async def _poll_blocks(self) -> None:
        try:
            blocks = await self.client.fetch_blocks()
            blocks_panel = self.query_one("#blocks-panel", BlocksPanel)
            blocks_panel.update_blocks(blocks)

            if blocks:
                header = self.query_one("#header-bar", HeaderBar)
                header.height_val = blocks[0].height

                # Celebration effect on new block
                new_height = blocks[0].height
                if self._last_known_height and new_height > self._last_known_height:
                    # Only celebrate if the block was mined recently (< 90s ago)
                    # This prevents stale notifications when the API catches up
                    block_age_s = (time.time() * 1000 - blocks[0].timestamp) / 1000
                    if block_age_s < 90:
                        miner = blocks[0].miner
                        tx_count = blocks[0].tx_count
                        reward = blocks[0].miner_reward
                        self.notify(
                            f"Block #{new_height:,} mined by {miner}\n"
                            f"{tx_count} TXs | {reward:.2f} ERG reward",
                            title="-- NEW BLOCK MINED --",
                            timeout=8,
                        )
                        self.bell()
                        # Flash header bar
                        self._flash_header()
                        # Trigger visualizer drain animation
                        visualizer = self.query_one("#tx-visualizer", TxVisualizer)
                        visualizer.on_block_mined()
                self._last_known_height = new_height
        except Exception:
            pass

    def _flash_header(self) -> None:
        """Briefly flash the header bar to celebrate a new block."""
        header = self.query_one("#header-bar", HeaderBar)
        header.styles.background = "#2a4a0a"
        header.styles.color = COLORS["warning"]
        self.set_timer(0.5, lambda: self._reset_header_flash())

    def _reset_header_flash(self) -> None:
        """Reset header bar after flash."""
        header = self.query_one("#header-bar", HeaderBar)
        header.styles.background = COLORS["panel_bg"]
        header.styles.color = COLORS["accent"]

    async def _poll_network(self) -> None:
        try:
            info = await self.client.fetch_network_state()
            network = self.query_one("#network-panel", NetworkPanel)
            network.update_info(info)

            # TX count is now set from mempool poll for consistency
        except Exception:
            pass

    async def _poll_price(self) -> None:
        try:
            price = await self.client.fetch_price()
            header = self.query_one("#header-bar", HeaderBar)
            header.price = price
        except Exception:
            pass

    # -- Actions --

    def action_sort(self) -> None:
        table = self.query_one("#tx-table", TxTable)
        self._current_sort = table.cycle_sort()
        self.notify(f"Sort: {self._current_sort}", timeout=2)

    def action_filter(self) -> None:
        filter_bar = self.query_one("#filter-bar", Input)
        if not self._filter_visible:
            filter_bar.add_class("visible")
            self._filter_visible = True
            filter_bar.focus()
        else:
            filter_bar.remove_class("visible")
            self._filter_visible = False

    def action_clear_filter(self) -> None:
        filter_bar = self.query_one("#filter-bar", Input)
        filter_bar.value = ""
        filter_bar.remove_class("visible")
        self._filter_visible = False
        table = self.query_one("#tx-table", TxTable)
        table.set_filter("")
        self.query_one("#tx-table", TxTable).focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id == "filter-bar":
            table = self.query_one("#tx-table", TxTable)
            table.set_filter(event.value)
            self.query_one("#tx-table", TxTable).focus()

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id == "filter-bar":
            table = self.query_one("#tx-table", TxTable)
            table.set_filter(event.value)

    async def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        """Handle Enter on a table row - show TX detail."""
        table = self.query_one("#tx-table", TxTable)
        # Look up TX by row key
        row_key_val = event.row_key.value
        tx = None
        for t in table._sorted_txs:
            if t.id == row_key_val:
                tx = t
                break
        if tx:
            detail_tx = await self.client.fetch_tx_detail(tx.id)
            detail = self.query_one("#tx-detail", TxDetail)
            detail.set_tx(detail_tx or tx)

    def action_copy_tx(self) -> None:
        table = self.query_one("#tx-table", TxTable)
        tx = table.get_selected_tx()
        if tx:
            try:
                import pyperclip
                pyperclip.copy(tx.id)
                self.notify(f"Copied: {tx.id[:12]}...", timeout=2)
            except ImportError:
                self.notify("pyperclip not installed", severity="warning", timeout=2)
            except Exception:
                self.notify(f"ID: {tx.id}", timeout=4)

    def action_explorer(self) -> None:
        table = self.query_one("#tx-table", TxTable)
        tx = table.get_selected_tx()
        if tx:
            url = f"https://explorer.ergoplatform.com/en/transactions/{tx.id}"
            webbrowser.open(url)
            self.notify(f"Opened explorer for {tx.id[:8]}...", timeout=2)

    def action_help(self) -> None:
        self.push_screen(HelpModal())

    def action_cycle_theme(self) -> None:
        self._theme_index = (self._theme_index + 1) % len(self._theme_names)
        theme_name = self._theme_names[self._theme_index]
        colors = set_theme(theme_name)
        new_css = generate_css(colors)
        # Update the CSS class var and refresh
        MempoolApp.CSS = new_css
        try:
            app_path = inspect.getfile(self.__class__)
        except (TypeError, OSError):
            app_path = ""
        read_from = (app_path, f"{self.__class__.__name__}.CSS")
        self.stylesheet.add_source(new_css, read_from=read_from, is_default_css=False)
        self.stylesheet.reparse()
        self.stylesheet.update(self)
        self.notify(f"Theme: {theme_name}", timeout=2)

    async def on_unmount(self) -> None:
        await self.client.close()
