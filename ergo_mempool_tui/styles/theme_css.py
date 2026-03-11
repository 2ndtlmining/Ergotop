"""Generate TCSS from a color palette dict."""


def generate_css(c: dict[str, str]) -> str:
    """Return complete TCSS string with colors from palette dict."""
    return f"""
Screen {{
    background: {c['bg']};
    color: {c['primary']};
}}

#header-bar {{
    dock: top;
    height: 1;
    background: {c['panel_bg']};
    color: {c['accent']};
    text-style: bold;
    border-bottom: solid {c['dim']};
}}

#footer-bar {{
    dock: bottom;
    height: 2;
    background: {c['panel_bg']};
    color: {c['dim']};
    border-top: solid {c['dim']};
    padding: 0 1;
}}

#main-container {{
    layout: horizontal;
    height: 1fr;
}}

/* Left column */
#left-column {{
    width: 34;
    border-right: solid {c['dim']};
}}

#utilization-bar {{
    height: 4;
    background: {c['panel_bg']};
    color: {c['primary']};
    border-bottom: dashed {c['dim']};
    padding: 0;
}}

#mempool-summary {{
    height: 1fr;
    background: {c['panel_bg']};
    color: {c['primary']};
    border-bottom: dashed {c['dim']};
    padding: 0;
}}

#blocks-panel {{
    height: auto;
    min-height: 6;
    background: {c['panel_bg']};
    color: {c['primary']};
    padding: 0;
}}

/* Center column */
#center-column {{
    width: 1fr;
    min-width: 40;
}}

#tx-visualizer {{
    height: 10;
    background: {c['panel_bg']};
    color: {c['primary']};
    border-bottom: dashed {c['dim']};
    padding: 0;
}}

#tx-table {{
    height: 1fr;
    background: {c['panel_bg']};
}}

#filter-bar {{
    dock: top;
    height: 1;
    display: none;
    background: {c['panel_bg']};
    color: {c['accent']};
    border-bottom: solid {c['dim']};
}}

#filter-bar.visible {{
    display: block;
}}

/* Right column */
#right-column {{
    width: 30;
    border-left: solid {c['dim']};
}}

#network-panel {{
    height: 8;
    background: {c['panel_bg']};
    color: {c['primary']};
    border-bottom: dashed {c['dim']};
    padding: 0;
}}

#origin-chart {{
    height: 12;
    background: {c['panel_bg']};
    color: {c['primary']};
    border-bottom: dashed {c['dim']};
    padding: 0;
}}

#tx-detail {{
    height: 1fr;
    background: {c['panel_bg']};
    color: {c['primary']};
    padding: 0;
}}

/* DataTable styling */
DataTable {{
    background: {c['panel_bg']};
    color: {c['primary']};
}}

DataTable > .datatable--header {{
    background: {c['panel_bg']};
    color: {c['accent']};
    text-style: bold;
}}

DataTable > .datatable--cursor {{
    background: {c['cursor_bg']};
    color: {c['accent']};
}}

DataTable > .datatable--even-row {{
    background: {c['even_row']};
}}

DataTable > .datatable--odd-row {{
    background: {c['odd_row']};
}}

/* Help modal */
#help-modal {{
    align: center middle;
    width: 50;
    height: 24;
    background: {c['panel_bg']};
    color: {c['primary']};
    border: solid {c['accent']};
    padding: 1 2;
    layer: modal;
}}

/* Filter input */
Input {{
    background: {c['panel_bg']};
    color: {c['primary']};
    border: solid {c['dim']};
}}

Input:focus {{
    border: solid {c['accent']};
}}
"""
