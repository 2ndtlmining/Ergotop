"""Animated block packing visualizer with smooth ERG logo transition.

Cycles: PACKING (5s) -> transition (2s) -> ERG (5s) -> transition (2s) -> ...
Each # cell interpolates from its packing position to its ERG letter position.
"""
from enum import Enum

from rich.text import Text
from textual.widgets import Static

from ergo_mempool_tui.api.models import Transaction
from ergo_mempool_tui.config import BLOCK_SIZE

NUM_ROWS = 7
TICKS_PER_VIEW = 25     # 5s at 5 FPS
TRANSITION_TICKS = 10   # 2s transition
DRAIN_TICKS = 10        # 2s drain


class ViewState(Enum):
    PACKING = "packing"
    TRANSITIONING = "transitioning"
    SHAPE = "shape"
    BLOCK_MINED = "mined"


def _ease_in_out(t: float) -> float:
    """Smoothstep easing for natural movement."""
    return t * t * (3.0 - 2.0 * t)


# ---------------------------------------------------------------------------
# Shape generator
# ---------------------------------------------------------------------------

def _ergo_positions(w: int) -> list[tuple[int, int]]:
    """Block letters 'ERG' centered in grid."""
    E = [
        "11111", "1    ", "1    ", "1111 ", "1    ", "1    ", "11111",
    ]
    R = [
        "1111 ", "1   1", "1   1", "1111 ", "1 1  ", "1  1 ", "1   1",
    ]
    G = [
        " 1111", "1    ", "1    ", "1  11", "1   1", "1   1", " 1111",
    ]
    letters = [E, R, G]
    letter_w = 5
    gap = 2
    total_w = len(letters) * letter_w + (len(letters) - 1) * gap  # 19
    scale = max(1, min(3, w // total_w))
    actual_w = total_w * scale
    offset = max(0, (w - actual_w) // 2)

    positions = []
    for li, letter in enumerate(letters):
        base_col = offset + li * (letter_w + gap) * scale
        for row in range(min(NUM_ROWS, len(letter))):
            for lc in range(letter_w):
                if lc < len(letter[row]) and letter[row][lc] == "1":
                    for s in range(scale):
                        col = base_col + lc * scale + s
                        if 0 <= col < w:
                            positions.append((row, col))
    return positions


SHAPE_NAME = "< ERG >"
SHAPE_FN = _ergo_positions


# ---------------------------------------------------------------------------
# Widget
# ---------------------------------------------------------------------------

class TxVisualizer(Static):
    """Animated visualizer with smooth cell transitions to ERG logo."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._txs: list[Transaction] = []
        self._state = ViewState.PACKING
        self._tick_count = 0
        self._drain_tick = 0
        self._going_to_shape = True  # direction of current transition
        self._drain_grid: list[list] | None = None  # snapshot for drain
        self._timer = None

    def on_mount(self) -> None:
        self._timer = self.set_interval(0.2, self._tick)

    # -- Tick / state machine -----------------------------------------------

    def _tick(self) -> None:
        self._tick_count += 1

        if self._state == ViewState.BLOCK_MINED:
            self._drain_tick += 1
            if self._drain_tick >= DRAIN_TICKS:
                self._state = ViewState.PACKING
                self._tick_count = 0
                self._drain_tick = 0
                self._drain_grid = None
            self.refresh()
            return

        if self._state == ViewState.TRANSITIONING:
            if self._tick_count >= TRANSITION_TICKS:
                self._tick_count = 0
                if self._going_to_shape:
                    self._state = ViewState.SHAPE
                else:
                    self._state = ViewState.PACKING
            self.refresh()
            return

        # PACKING or SHAPE -- static display
        if self._tick_count >= TICKS_PER_VIEW:
            self._tick_count = 0
            self._going_to_shape = self._state == ViewState.PACKING
            self._state = ViewState.TRANSITIONING
            self.refresh()
            return

        # Refresh once at the start of each static phase
        if self._tick_count == 1:
            self.refresh()

    def update_txs(self, txs: list[Transaction]) -> None:
        self._txs = txs
        # Don't refresh during drain -- keep the snapshot stable
        if self._state != ViewState.BLOCK_MINED:
            self.refresh()

    def on_block_mined(self) -> None:
        """Trigger the drain animation, snapshotting current grid."""
        gw = max(self.size.width - 6, 20)
        cells = self._compute_packing_cells(gw)
        self._drain_grid = self._cells_to_grid(cells, gw)
        self._state = ViewState.BLOCK_MINED
        self._drain_tick = 0
        self._tick_count = 0
        self.refresh()

    # -- Render dispatch ----------------------------------------------------

    def render(self) -> Text:
        if self._state == ViewState.BLOCK_MINED:
            return self._render_mined()
        if self._state == ViewState.PACKING:
            return self._render_packing()
        if self._state == ViewState.SHAPE:
            return self._render_shape()
        # TRANSITIONING
        progress = min(self._tick_count / TRANSITION_TICKS, 1.0)
        return self._render_transition(progress)

    # -- Cell position computation ------------------------------------------

    def _compute_packing_cells(self, grid_width: int) -> list[tuple[int, int, str]]:
        """Return (row, col, color) for each # cell in the packing layout."""
        if not self._txs:
            return []
        sorted_txs = sorted(self._txs, key=lambda tx: tx.size, reverse=True)
        total_cells = grid_width * NUM_ROWS

        segments: list[tuple[int, str]] = []
        for tx in sorted_txs:
            n = max(1, int(tx.size / BLOCK_SIZE * total_cells))
            segments.append((n, tx.origin_color))

        cells: list[tuple[int, int, str]] = []
        seg_idx = 0
        seg_used = 0

        for row in range(NUM_ROWS):
            col = 0
            while col < grid_width and seg_idx < len(segments):
                seg_n, color = segments[seg_idx]
                remaining = seg_n - seg_used
                available = grid_width - col
                use = min(remaining, available)

                for c in range(use):
                    cells.append((row, col + c, color))
                col += use
                seg_used += use

                if seg_used >= seg_n:
                    seg_idx += 1
                    seg_used = 0
                    if col < grid_width and seg_idx < len(segments):
                        col += 1  # separator space
        return cells

    # -- Static views -------------------------------------------------------

    def _packing_title(self) -> str:
        total = sum(tx.size for tx in self._txs)
        pct = min(total / BLOCK_SIZE * 100, 100) if BLOCK_SIZE else 0
        return f"  NEXT BLOCK ({pct:.0f}% full)"

    def _render_packing(self) -> Text:
        gw = max(self.size.width - 6, 20)
        result = Text()
        result.append(self._packing_title() + "\n")

        cells = self._compute_packing_cells(gw)
        grid = self._cells_to_grid(cells, gw)
        self._append_grid(result, grid, gw)
        return result

    def _render_shape(self) -> Text:
        gw = max(self.size.width - 6, 20)
        shape_pos = SHAPE_FN(gw)
        packing_cells = self._compute_packing_cells(gw)

        grid = [[None] * gw for _ in range(NUM_ROWS)]

        # Dim outline for the full shape
        for r, c in shape_pos:
            if 0 <= r < NUM_ROWS and 0 <= c < gw:
                grid[r][c] = (".", "dim")

        # Fill shape positions with TX colors
        for i, (r, c) in enumerate(shape_pos):
            if i < len(packing_cells):
                _, _, color = packing_cells[i]
                if 0 <= r < NUM_ROWS and 0 <= c < gw:
                    grid[r][c] = ("#", color)

        result = Text()
        result.append(f"  {SHAPE_NAME}\n")
        self._append_grid(result, grid, gw)
        return result

    # -- Smooth transition --------------------------------------------------

    def _render_transition(self, progress: float) -> Text:
        gw = max(self.size.width - 6, 20)
        eased = _ease_in_out(progress)

        # Reverse easing when going shape -> packing
        if not self._going_to_shape:
            eased = 1.0 - eased

        packing_cells = self._compute_packing_cells(gw)
        shape_pos = SHAPE_FN(gw)

        grid = [[None] * gw for _ in range(NUM_ROWS)]

        # Show dim outline of target shape fading in
        if eased > 0.2:
            for r, c in shape_pos:
                if 0 <= r < NUM_ROWS and 0 <= c < gw:
                    grid[r][c] = (".", "dim")

        # Interpolate each packing cell toward its shape target
        n_targets = len(shape_pos)
        for i, (src_r, src_c, color) in enumerate(packing_cells):
            if i < n_targets:
                dst_r, dst_c = shape_pos[i]
            else:
                # Overflow cells slide off the bottom
                dst_r = NUM_ROWS + 1
                dst_c = src_c

            cur_r = round(src_r + (dst_r - src_r) * eased)
            cur_c = round(src_c + (dst_c - src_c) * eased)

            if 0 <= cur_r < NUM_ROWS and 0 <= cur_c < gw:
                grid[cur_r][cur_c] = ("#", color)

        # Title crossfade at midpoint
        result = Text()
        if self._going_to_shape:
            title = self._packing_title() if progress < 0.5 else f"  {SHAPE_NAME}"
        else:
            title = f"  {SHAPE_NAME}" if progress < 0.5 else self._packing_title()
        result.append(title + "\n")

        self._append_grid(result, grid, gw)
        return result

    # -- Block mined drain --------------------------------------------------

    def _render_mined(self) -> Text:
        gw = max(self.size.width - 6, 20)

        result = Text()
        result.append("  BLOCK MINED!\n", style="bold #ffb000")

        # Use the snapshot taken when on_block_mined() was called
        if self._drain_grid is not None:
            grid = self._drain_grid
        else:
            cells = self._compute_packing_cells(gw)
            grid = self._cells_to_grid(cells, gw)

        # Rows cleared from bottom up
        cleared_from = NUM_ROWS - self._drain_tick

        for i in range(NUM_ROWS):
            result.append("  |")
            if i >= cleared_from:
                result.append(" " * gw)
            else:
                for j in range(gw):
                    cell = grid[i][j]
                    if cell is None:
                        result.append(" ")
                    elif cell[1]:
                        result.append(cell[0], style=cell[1])
                    else:
                        result.append(cell[0])
            result.append("|")
            if i < NUM_ROWS - 1:
                result.append("\n")
        return result

    # -- Grid helpers -------------------------------------------------------

    def _cells_to_grid(
        self, cells: list[tuple[int, int, str]], gw: int
    ) -> list[list]:
        """Convert cell list to 2D grid of (char, style) or None."""
        grid = [[None] * gw for _ in range(NUM_ROWS)]
        for r, c, color in cells:
            if 0 <= r < NUM_ROWS and 0 <= c < gw:
                grid[r][c] = ("#", color)
        return grid

    def _append_grid(self, result: Text, grid: list[list], gw: int) -> None:
        """Render a 2D grid into a Text object with | borders."""
        for i in range(NUM_ROWS):
            result.append("  |")
            for j in range(gw):
                cell = grid[i][j]
                if cell is None:
                    result.append(" ")
                elif cell[1]:
                    result.append(cell[0], style=cell[1])
                else:
                    result.append(cell[0])
            result.append("|")
            if i < NUM_ROWS - 1:
                result.append("\n")
