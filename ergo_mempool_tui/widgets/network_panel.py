"""Network info display."""
from textual.widgets import Static
from textual.reactive import reactive

from ergo_mempool_tui.api.models import NodeInfo


class NetworkPanel(Static):
    """Shows node/network info: height, headers, peers, mempool count."""

    _info: NodeInfo = NodeInfo()
    mempool_count: reactive[int] = reactive(0)

    def update_info(self, info: NodeInfo) -> None:
        self._info = info
        self.refresh()

    def watch_mempool_count(self, value: int) -> None:
        self.refresh()

    @staticmethod
    def _format_hashrate(difficulty: float) -> str:
        """Convert difficulty to estimated hashrate (H/s). Ergo block time ~120s."""
        if difficulty <= 0:
            return "---"
        hashrate = difficulty / 120  # H/s (approximate)
        if hashrate >= 1e15:
            return f"{hashrate / 1e15:.2f} PH/s"
        elif hashrate >= 1e12:
            return f"{hashrate / 1e12:.2f} TH/s"
        elif hashrate >= 1e9:
            return f"{hashrate / 1e9:.2f} GH/s"
        elif hashrate >= 1e6:
            return f"{hashrate / 1e6:.2f} MH/s"
        else:
            return f"{hashrate:.0f} H/s"

    def render(self) -> str:
        i = self._info
        hashrate = self._format_hashrate(i.difficulty)
        version = i.node_version or "---"
        return (
            f"  NETWORK INFO\n"
            f"  Height   {i.height:>10,}\n"
            f"  Peers    {i.peer_count:>10}\n"
            f"  Hashrate {hashrate:>10}\n"
            f"  Node     {version:>10}\n"
            f"  Mempool TXs {self.mempool_count:>6}"
        )
