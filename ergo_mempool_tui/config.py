"""Configuration constants and API URLs."""
import os

# Block size constant (2 MB)
BLOCK_SIZE = 2 * 1024 * 1024  # 2,097,152 bytes

# API URLs (p2p first, main fallback)
API_PRIMARY = os.environ.get("ERGO_API_URL", "https://api-p2p.ergoplatform.com")
API_FALLBACK = "https://api.ergoplatform.com"
API_TIMEOUT = 12  # seconds

# Optional local node
NODE_URL = os.environ.get("ERGO_NODE_URL", "http://213.239.193.208:9053")

# Oracle price endpoint
ORACLE_URL = "https://erg-oracle-ergusd.spirepools.com/frontendData"

# Poll intervals (seconds)
POLL_MEMPOOL = 5
POLL_NETWORK = 10
POLL_BLOCKS = 30
POLL_PRICE = 300  # 5 min

# Ergo fee address
FEE_ADDRESS = (
    "2iHkR7CWvD1R4j1yZg5bkeDRQavjAaVPeTDFGGLZduHyfWMuYpmhHocX8GJoaieTx78"
    "FntzJbCBVL6rf96ocJoZdmWBL2fci7NqWgAirppPQmZ7fN9V6z13Ay6brPriBKYqLp1b"
    "T2Fk4FkFLCfdPpe"
)

# Theme palettes
THEMES = {
    "neon-green": {
        "bg": "#0a0e0f", "panel_bg": "#0c1213",
        "primary": "#39ff14", "accent": "#00ffcc",
        "warning": "#ffb000", "error": "#ff4444",
        "dim": "#4a7a4a", "cursor_bg": "#1a3a1a",
        "even_row": "#0a0e0f", "odd_row": "#0c1213",
    },
    "amber-terminal": {
        "bg": "#0f0c06", "panel_bg": "#12100a",
        "primary": "#ffb000", "accent": "#ffd700",
        "warning": "#ff6600", "error": "#ff4444",
        "dim": "#7a6a3a", "cursor_bg": "#3a2a0a",
        "even_row": "#0f0c06", "odd_row": "#12100a",
    },
    "blue-ice": {
        "bg": "#0a0e14", "panel_bg": "#0c1218",
        "primary": "#4fc3f7", "accent": "#80deea",
        "warning": "#ffb74d", "error": "#ef5350",
        "dim": "#37474f", "cursor_bg": "#1a2a3a",
        "even_row": "#0a0e14", "odd_row": "#0c1218",
    },
    "high-contrast": {
        "bg": "#000000", "panel_bg": "#0a0a0a",
        "primary": "#ffffff", "accent": "#00ffff",
        "warning": "#ffff00", "error": "#ff0000",
        "dim": "#666666", "cursor_bg": "#333333",
        "even_row": "#000000", "odd_row": "#0a0a0a",
    },
}

# Active colors dict - updated on theme switch
COLORS = dict(THEMES["neon-green"])


def set_theme(name: str) -> dict[str, str]:
    """Switch active color palette. Returns the new colors dict."""
    palette = THEMES[name]
    COLORS.update(palette)
    return COLORS

# Known mining pool addresses -> display name
# Sourced from v1 project (2ndtlmining/Ergomempool transactionOrigins.js)
# Pools use P2SH mining reward contracts (88dhgzEuTXa... prefix)
MINING_POOLS: dict[str, str] = {
    # Active P2SH mining payout addresses
    "88dhgzEuTXaRQTX5KNdnaWTTX7fEZVEQRn6qP4MJotPuRnS3QpoJxYpSaXoU1y7SHp8ZXMp92TH22DBY": "2Miners",
    "88dhgzEuTXaRiLRSYpvTCXWoN3A86gnWs3Z8BWkJGkGMXsR3WzUUyqbB47YAzhhsB6HJdJ4tC5AFYfSc": "2Miners Solo",
    "88dhgzEuTXaSuf5QC1TJDgdxqJMQEQAM6YaTTRqmUDrmPoVky1b16WAK5zMrq3p2mYqpUNKCyi5CLS9V": "Herominers",
    "88dhgzEuTXaTnTZomXPfuJ67oYJPbrv17yNkLjN6Nj8HxZEUf2iAdiv9gTqmnKKa2i75zmUtDnPQovBb": "Kryptex",
    "88dhgzEuTXaRp6WD5jWZSnXzBbA44g1xSMk6Xv2r6Cey8snSH78S6ZbWjP24yyPTDCCZByLpNXXe6NnN": "Nanopool",
    "88dhgzEuTXaTj2AZkM2vwnemCYyAUJymaFf8iJPUYmgLkJqQmPd3DTubYS5UfL75MhQbEjmuhBMbdspA": "K1Pool",
    "88dhgzEuTXaUPpNAbKL7UeNUFEcjkoqW6ev5P1hkynBmG4L5baYdZ8rSPYCDNmvwBLiJR7ABjndPhqGm": "DXPool",
    "88dhgzEuTXaQDYikoEkCMEPRxDiYnVRfiqhf3uLcMhbTPrTrrc7wkyF5LFMmgJyT4mPa6ucnmk3QTeUo": "SigPool",
    "88dhgzEuTXaQ2HPUskY3hvgMA5uCbQWwZNPbMC1Hem9zM2V9U7KMah7LYWS4Hm4WECGuc22nofdQbHbY": "WoolyPooly",
    "88dhgzEuTXaS7z8MA868ZjU6ujmWi8Wqcx7VK36jcogWt8cRCKoGJYE6ezDe5g7f6fG7arqXNhuNP8Je": "JJPool",
    "9hhM7ZqQCVX6iti8Jos8BLv2MTVis51mm8sQNfwXfUqNgTjEbqv": "Unmineable",
}

PLATFORM_COLORS = {
    "Spectrum": "#3498db",
    "Duckpools": "#27ae60",
    "ErgoPad": "#9b59b6",
    "SigmaFi": "#e74c3c",
    "AuctionHouse": "#f39c12",
    "SigmaDrop": "#1abc9c",
    "SkyHarbor": "#00bcd4",
    "MewFinance": "#e91e63",
    "CruxFinance": "#ff9800",
    "RosenBridge": "#673ab7",
    "USDOracle": "#34495e",
    "GoldOracle": "#d4a017",
    "MiningPool": "#8b4513",
    "SigUSD": "#2ecc71",
    "DexyUSD": "#703831",
    "Ergomixer": "#9b59b6",
    "Kucoin": "#23af91",
    "NonKyc": "#4a90d9",
    "Gate": "#2354e6",
    "MEXC": "#1972f0",
    "Huobi": "#1d8cf8",
    "CoinEx": "#3cb371",
    "TradeOgre": "#d4a017",
    "Xeggex": "#e67e22",
    "Probit": "#c0392b",
    "FluxSwap": "#2980b9",
    "TokenJay": "#e73ca0",
    "ErgoRaffle": "#e74c3c",
    "Padeia": "#8e44ad",
    "HODLERG": "#f1c40f",
    "Lilium": "#e91e63",
    "Gridbot": "#e73ccb",
    "Pulse": "#16a085",
    "Whale": "#2c3e50",
    "P2P": "#7ab87a",
    "Contract": "#8a6a9a",
    "Unknown": "#556655",
}
