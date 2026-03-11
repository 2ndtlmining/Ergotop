"""Entry point: python -m ergo_mempool_tui"""
from ergo_mempool_tui.app import MempoolApp

if __name__ == "__main__":
    app = MempoolApp()
    app.run()
