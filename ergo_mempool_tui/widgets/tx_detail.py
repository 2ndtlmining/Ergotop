"""TX detail panel shown when a transaction is selected."""
from textual.widgets import Static
from textual.reactive import reactive

from ergo_mempool_tui.api.models import Transaction


class TxDetail(Static):
    """Shows detailed info for the selected transaction."""

    tx: Transaction | None = None

    def set_tx(self, tx: Transaction | None) -> None:
        self.tx = tx
        self.refresh()

    def render(self) -> str:
        if self.tx is None:
            return "  TX DETAIL\n  Select a TX with Enter"

        tx = self.tx
        lines = [
            "  TX DETAIL",
            f"  ID: {tx.id[:12]}...{tx.id[-4:]}",
            f"  Size: {tx.size / 1024:.1f} KB" if tx.size >= 1024 else f"  Size: {tx.size} B",
            f"  Fee: {tx.fee:.4f} ERG",
            f"  Value: {tx.value:.2f} ERG",
            f"  Origin: {tx.origin}",
            f"  Inputs: {len(tx.inputs)}  Outputs: {len(tx.outputs)}",
            "",
        ]

        # Collect all unique tokens across inputs and outputs
        all_tokens: dict[str, str] = {}
        for io in tx.inputs + tx.outputs:
            for asset in io.assets:
                if asset.token_id not in all_tokens:
                    all_tokens[asset.token_id] = asset.name or asset.token_id[:8]

        # Show token transfers summary
        if all_tokens:
            lines.append("  -- Tokens --")
            # Sum net token flow (outputs - inputs) per token
            for tid, name in list(all_tokens.items())[:4]:
                out_amt = sum(a.amount for io in tx.outputs for a in io.assets if a.token_id == tid)
                lines.append(f"  {name[:12]}: {out_amt:,.2f}")
            if len(all_tokens) > 4:
                lines.append(f"  ... +{len(all_tokens) - 4} more tokens")
            lines.append("")

        # Show up to 6 inputs
        if tx.inputs:
            lines.append("  -- Inputs --")
            for io in tx.inputs[:6]:
                addr = io.address[:8] + ".." if io.address else "?"
                line = f"  {addr} {io.value:.2f} ERG"
                if io.assets:
                    token_names = [a.name[:6] or a.token_id[:6] for a in io.assets[:2]]
                    line += f" +{','.join(token_names)}"
                lines.append(line)
            if len(tx.inputs) > 6:
                lines.append(f"  ... +{len(tx.inputs) - 6} more")

        # Show up to 6 outputs
        if tx.outputs:
            lines.append("  -- Outputs --")
            for io in tx.outputs[:6]:
                addr = io.address[:8] + ".." if io.address else "?"
                line = f"  {addr} {io.value:.2f} ERG"
                if io.assets:
                    token_names = [a.name[:6] or a.token_id[:6] for a in io.assets[:2]]
                    line += f" +{','.join(token_names)}"
                lines.append(line)
            if len(tx.outputs) > 6:
                lines.append(f"  ... +{len(tx.outputs) - 6} more")

        return "\n".join(lines)
