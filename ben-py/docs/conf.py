# Configuration file for the Sphinx documentation builder.
#
# Full option reference: https://www.sphinx-doc.org/en/master/usage/configuration.html

import json
import os
from importlib import metadata

# autodoc imports the package at runtime to read live docstrings, including those in
# the compiled ``_core`` extension. We rely on the pip-installed package for this rather
# than prepending the source tree to ``sys.path``: the source tree carries no built
# ``_core`` (it lives only in the wheel), so shadowing the install would break the import.

# -- Project information -----------------------------------------------------

project = "binary-ensemble"
copyright = "2025, Peter Rock"
author = "Peter Rock"

try:
    release = metadata.version("binary-ensemble")
except metadata.PackageNotFoundError:
    release = ""
version = ".".join(release.split(".")[:2])

# -- General configuration ---------------------------------------------------

extensions = [
    "sphinx.ext.autodoc",
    "sphinx.ext.napoleon",
    "sphinx.ext.viewcode",
    "sphinx.ext.intersphinx",
    "sphinx.ext.mathjax",
    "sphinx_copybutton",
    "sphinx_design",
    "sphinxext.opengraph",
    "myst_nb",
]

exclude_patterns = [
    "_build",
    "jupyter_execute",
    ".jupyter_cache",
    "Thumbs.db",
    ".DS_Store",
    "**/example_data/**",
]

# -- MyST (markdown) ---------------------------------------------------------

myst_enable_extensions = [
    "colon_fence",
    "deflist",
    "dollarmath",
    "linkify",
    "substitution",
    "tasklist",
]
myst_heading_anchors = 3

# -- MyST-NB (executable notebooks) ------------------------------------------
#
# Execution is env-driven. The hosted build (ReadTheDocs) leaves it "off" so it
# renders the committed notebook outputs and stays fast and reliable; CI and local
# verification set ``NB_EXECUTION_MODE=cache`` to actually run every notebook and
# fail the build on any drift between the docs and the live API.
nb_execution_mode = os.environ.get("NB_EXECUTION_MODE", "off")
nb_execution_timeout = 1800
nb_execution_raise_on_error = True
nb_merge_streams = True

# -- autodoc / napoleon ------------------------------------------------------

# Members are listed by the explicit ``autoclass``/``autofunction`` directives in the
# API pages, so ``automodule`` is left to render only the module docstring (no members);
# documenting each object twice produces "duplicate object description" warnings.
autodoc_default_options = {
    "show-inheritance": True,
    "member-order": "bysource",
}
autodoc_typehints = "description"
autodoc_inherit_docstrings = False
add_module_names = False

napoleon_google_docstring = True
napoleon_numpy_docstring = True
napoleon_use_rtype = False

# -- intersphinx -------------------------------------------------------------

intersphinx_mapping = {
    "python": ("https://docs.python.org/3", None),
    "networkx": ("https://networkx.org/documentation/stable/", None),
    "numpy": ("https://numpy.org/doc/stable/", None),
    "pandas": ("https://pandas.pydata.org/docs/", None),
}

# -- linkcheck ---------------------------------------------------------------
#
# Keep link checking separate from normal HTML builds because it depends on external
# services. CI runs it as its own step so transient failures are easy to diagnose.
linkcheck_timeout = 10
linkcheck_retries = 2
linkcheck_anchors = False
linkcheck_ignore = [
    r"http://localhost:\d+/",
    r"http://127\.0\.0\.1:\d+/",
    # GitHub source/blob URLs are useful in rendered docs but frequently rate-limit
    # unauthenticated CI linkcheck runs.
    r"https://github\.com/peterrrock2/binary-ensemble/(blob|tree)/.*",
    # crates.io serves a 404 to non-browser/HEAD requests, so the (valid) crate page link
    # can't be validated by linkcheck even though it resolves fine in a browser.
    r"https://crates\.io/.*",
    # mggg.org answers a browser instantly but stalls (read timeout, never replies) for the
    # datacenter IP + non-browser User-Agent linkcheck uses from CI, so no finite timeout
    # validates it. The link is valid; skip it in CI rather than chase the timeout.
    r"https://mggg\.org/.*",
    # maturin.rs answers a browser instantly but stalls (read timeout, never replies) for the
    # datacenter IP + non-browser User-Agent linkcheck uses from CI, so no finite timeout
    # validates it. The link is valid; skip it in CI rather than chase the timeout.
    r"https://maturin\.rs/.*",
    # pyo3.rs answers a browser instantly but stalls (read timeout, never replies) for the
    # datacenter IP + non-browser User-Agent linkcheck uses from CI, so no finite timeout
    # validates it. The link is valid; skip it in CI rather than chase the timeout.
    r"https://pyo3\.rs/.*",
]

# -- HTML output -------------------------------------------------------------

html_theme = "furo"
html_title = "binary-ensemble"
html_static_path = ["_static"]
html_css_files = ["css/custom.css"]

pygments_style = "friendly"
pygments_dark_style = "github-dark"


# -- Color palettes ----------------------------------------------------------
#
# Each palette gives furo a light-mode and a dark-mode brand color as
# ``(primary, content)``: ``primary`` tints sidebar accents/headings, ``content``
# is the in-text link color (kept darker in light mode / lighter in dark mode for
# legible contrast). Swap the whole site palette by changing ACTIVE_PALETTE below,
# or without editing this file via the DOCS_PALETTE env var, e.g.:
#
#     DOCS_PALETTE=forest task docs-serve
#
# Add your own entries freely.
# Each entry maps a mode ("light"/"dark") to a dict of Furo CSS variables. Most
# palettes set only the two brand colors ("color-brand-primary" (sidebar/heading
# accents) and "color-brand-content" (in-text links)) via the _brand() helper, but a
# palette may set *any* Furo variable, e.g. backgrounds/foregrounds (see "aurora").
#
# A palette may also carry "dark_pygments" / "light_pygments": the names of the Pygments
# styles used for code blocks in dark / light mode when that palette is active (the
# switcher's "Auto" code theme). Each must be a style listed in CODE_THEMES below; a
# palette without one falls back to the global pygments_dark_style / pygments_style.
def _brand(primary, content):
    return {"color-brand-primary": primary, "color-brand-content": content}


PALETTES = {
    "ocean": {
        "light": _brand("#0099cd", "#0066a0"),
        "dark": _brand("#36c5f0", "#5cc8f5"),
    },
    "indigo": {
        "light": _brand("#4f46e5", "#4338ca"),
        "dark": _brand("#818cf8", "#a5b4fc"),
    },
    "forest": {
        "light": _brand("#047857", "#065f46"),
        "dark": _brand("#34d399", "#6ee7b7"),
    },
    "sunset": {
        "light": _brand("#ea580c", "#c2410c"),
        "dark": _brand("#fb923c", "#fdba74"),
    },
    "plum": {
        "light": _brand("#7c3aed", "#6d28d9"),
        "dark": _brand("#a78bfa", "#c4b5fd"),
    },
    "slate": {
        "light": _brand("#334155", "#1e293b"),
        "dark": _brand("#94a3b8", "#cbd5e1"),
    },
    # From a Huemint palette: a charcoal dark mode with neon-teal accents, and a
    # matching light mode that carries the teal as a darker, legible shade on white.
    "aurora": {
        "dark_pygments": "github-dark",
        "light": {
            "color-background-primary": "#feffff",
            "color-foreground-primary": "#242827",
            "color-brand-primary": "#0d9488",
            "color-brand-content": "#0f766e",
        },
        "dark": {
            "color-background-primary": "#242827",
            "color-background-secondary": "#000200",
            "color-foreground-primary": "#feffff",
            "color-brand-primary": "#36e8c8",
            "color-brand-content": "#36e8c8",
        },
    },
    # From a Huemint palette: warm ember accents (peach headings, teal links)
    # over a near-black dark mode; light mode carries the amber as a legible
    # darker shade on white.
    "ember": {
        "dark_pygments": "gruvbox-dark",
        "light": {
            "color-background-primary": "#fdf7f1",
            "color-background-secondary": "#f7ede3",
            "color-foreground-primary": "#0c0706",
            "color-brand-primary": "#d97906",
            "color-brand-content": "#a8560a",
        },
        "dark": {
            "color-background-primary": "#0c0706",
            "color-background-secondary": "#181206",
            "color-foreground-primary": "#e3ecf6",
            "color-brand-primary": "#fc9d66",
            "color-brand-content": "#45c9cb",
        },
    },
    # From a Huemint light palette (navy + blue with an orange pop); the dark
    # mode is derived: a deep-navy canvas with the true navy as the secondary
    # surface, a lightened blue for headings, and a warmed orange for links.
    "harbor": {
        "dark_pygments": "one-dark",
        "light": {
            "color-background-primary": "#f4f7fc",
            "color-background-secondary": "#e8eef7",
            "color-foreground-primary": "#1f2c5b",
            "color-brand-primary": "#2965ad",
            "color-brand-content": "#1f4a8a",
        },
        "dark": {
            "color-background-primary": "#131a36",
            "color-background-secondary": "#1f2c5b",
            "color-foreground-primary": "#fffdfe",
            "color-brand-primary": "#6ea8e0",
            "color-brand-content": "#ff7a45",
        },
    },
    # From a Huemint palette: a deep-indigo dark mode with a neon cyan/hot-pink
    # accent pair (synthwave); light mode carries them as a deep rose + dark teal
    # that stay legible on cream.
    "nebula": {
        "dark_pygments": "dracula",
        "light": {
            "color-background-primary": "#fbfaf2",
            "color-background-secondary": "#f1f0e6",
            "color-foreground-primary": "#17143b",
            "color-brand-primary": "#c8155a",
            "color-brand-content": "#0e7490",
        },
        "dark": {
            "color-background-primary": "#17143b",
            "color-background-secondary": "#211f1f",
            "color-foreground-primary": "#fbfaf2",
            "color-brand-primary": "#ea0758",
            "color-brand-content": "#2cdbde",
        },
    },
    # From a Huemint palette: a warm near-black dark mode with a bright orange /
    # cerulean (complementary) accent pair; light mode darkens both for white.
    # "color-brand-content": "#075985",
    # "color-brand-content": "#176995",
    "tangerine": {
        "dark_pygments": "warm-dark",
        "light_pygments": "warm-light",
        "light": {
            "color-background-primary": "#fbfaf2",
            "color-background-secondary": "#f1f0e6",
            "color-foreground-primary": "#140f0c",
            "color-brand-primary": "#c2410c",
            "color-brand-content": "#0077c4",
        },
        "dark": {
            "color-background-primary": "#1c1917",
            "color-background-secondary": "#292524",
            "color-foreground-primary": "#fcffff",
            "color-brand-primary": "#ff750f",
            "color-brand-content": "#0097d4",
        },
    },
}
ACTIVE_PALETTE = os.environ.get("DOCS_PALETTE", "tangerine")
_palette = PALETTES[ACTIVE_PALETTE]

# Whether to render the in-browser palette/code-theme dropdowns. Off by default so the
# published site ships locked to the active palette and its default code themes; set
# DOCS_SWITCHER=1 while developing to expose the controls and experiment live.
SHOW_SWITCHER = os.environ.get("DOCS_SWITCHER", "").lower() not in (
    "",
    "0",
    "false",
    "no",
)

html_theme_options = {
    "source_repository": "https://github.com/peterrrock2/binary-ensemble/",
    "source_branch": "main",
    "source_directory": "ben-py/docs/",
    # Bake only the brand colors; the switcher script paints the full active palette
    # (including any background/foreground overrides) on load, so it stays the sole
    # owner of those and switching palettes in the browser reverts cleanly.
    "light_css_variables": {
        k: v for k, v in _palette["light"].items() if k.startswith("color-brand-")
    },
    "dark_css_variables": {
        k: v for k, v in _palette["dark"].items() if k.startswith("color-brand-")
    },
    "footer_icons": [
        {
            "name": "GitHub",
            "url": "https://github.com/peterrrock2/binary-ensemble",
            "html": "",
            "class": "fa-brands fa-github",
        },
    ],
}

# -- OpenGraph (social cards) ------------------------------------------------

ogp_site_url = "https://binary-ensemble.readthedocs.io/"
ogp_description_length = 200
ogp_enable_meta_description = True
# Emit OpenGraph meta tags but skip the matplotlib-rendered preview images (their default
# font lacks some glyphs we use, e.g. the "↔" arrow).
ogp_social_cards = {"enable": False}


# -- Swappable code (Pygments) themes ----------------------------------------
#
# Furo bakes one light + one dark Pygments theme (pygments_style / pygments_dark_style)
# into pygments.css. To make code themes swappable (per palette and live in the
# browser) we render each style below and key it off a <body> attribute the switcher
# sets. Pygments' own `.highlight { background }` line rides along, so every theme
# brings its matching code-block surface.
#
# Two attributes, two behaviors:
#   * data-code-theme: an explicit pick from the dropdown. Scoped `html body[…]` so it
#                        applies in BOTH light and dark mode and out-specifies Furo's
#                        own rules regardless of stylesheet order.
#   * data-code-auto: the active palette's "dark_pygments" default (the "Auto" entry),
#                        scoped to dark mode only so light mode keeps the global light
#                        style. The auto-mode (`prefers-color-scheme`) variant mirrors
#                        Furo's `:not([data-theme="light"])` selector for system readers.
#
# CODE_THEMES is the menu the switcher offers, grouped into the <optgroup>s shown in the
# dropdown. Add or remove any valid Pygments style name (`python -m pygments -L styles`);
# the "Dark"/"Light" labels are just hints about which mode a style suits.
CODE_THEMES = {
    "Dark": [
        "warm-dark",
        "github-dark",
        "gruvbox-dark",
        "one-dark",
        "dracula",
        "nord",
        "monokai",
        "material",
        "zenburn",
        "native",
        "solarized-dark",
        "paraiso-dark",
        "stata-dark",
        "fruity",
        "coffee",
    ],
    "Light": [
        "warm-light",
        "github-light",
        "gruvbox-light",
        "solarized-light",
        "friendly",
        "tango",
        "xcode",
        "lovelace",
        "manni",
        "paraiso-light",
        "arduino",
        "vs",
    ],
}


# Custom (non-builtin) Pygments styles, keyed by the name used in CODE_THEMES and the
# data-code-theme attribute. _pygments_theme_css resolves these to the Style class
# instead of a builtin style name (HtmlFormatter accepts either).
#
# "warm-light" is built to fit the warm palettes (tangerine/nebula/ember): a cream
# background with tokens drawn from the brand accent family (orange and amber warms
# against cerulean and teal cools) rather than a stock theme's unrelated hues. Every
# token color is chosen to clear ~4.5:1 contrast on the cream background.
def _warm_light():
    from pygments.style import Style
    from pygments.token import (
        Comment,
        Error,
        Generic,
        Keyword,
        Name,
        Number,
        Operator,
        String,
        Token,
    )

    return type(
        "WarmLightStyle",
        (Style,),
        {
            "name": "warm-light",
            "background_color": "#f6f1e7",
            "highlight_color": "#e7dcc4",
            "styles": {
                Token: "#20180f",
                Comment: "italic #685c4b",
                Comment.Preproc: "noitalic #c2410c",
                Keyword: "bold #c2410c",
                Keyword.Type: "nobold #623c00",
                Keyword.Constant: "nobold #b8336a",
                Operator: "#6a4a2a",
                Operator.Word: "bold #c2410c",
                Name.Builtin: "bold #08527d",
                Name.Function: "bold #08527d",
                Name.Class: "bold #0a5a86",
                Name.Namespace: "bold #0a5a86",
                Name.Exception: "bold #d10a46",
                Name.Variable: "#20180f",
                Name.Constant: "#623c00",
                Name.Decorator: "#c2410c",
                Name.Attribute: "#0a5a86",
                Name.Tag: "bold #0a6d3f",
                String: "bold #0a6d3f",
                String.Doc: "italic #685c4b",
                String.Escape: "bold #c2410c",
                Number: "bold #861657",
                Generic.Heading: "bold #20180f",
                Generic.Subheading: "bold #0a5a86",
                Generic.Deleted: "#b3261e",
                Generic.Inserted: "#0a6d3f",
                Generic.Error: "#b3261e",
                Generic.Emph: "italic",
                Generic.Strong: "bold",
                Generic.Prompt: "bold #685c4b",
                Error: "border:#b3261e",
            },
        },
    )


# "warm-dark" is the dark companion to warm-light: the SAME token roles and bold/italic
# treatment, in bright dark-mode colors. It keeps fruity's blue / green / orange family and
# warm-light's magenta numbers; every token is chosen to clear ~5.5:1+ on the dark canvas.
# (Mirrors warm-light's token set, so the two themes feel consistent across light/dark.)
def _warm_dark():
    from pygments.style import Style
    from pygments.token import (
        Comment,
        Error,
        Generic,
        Keyword,
        Name,
        Number,
        Operator,
        String,
        Token,
    )

    return type(
        "WarmDarkStyle",
        (Style,),
        {
            "name": "warm-dark",
            "background_color": "#292524",
            "highlight_color": "#2a2218",
            "line_number_color": "inherit",
            "line_number_background_color": "transparent",
            "styles": {
                Token: "#f4efe6",
                Comment: "italic #9a8f7c",
                Comment.Preproc: "noitalic #ff750f",
                Keyword: "bold #ff750f",
                Keyword.Type: "nobold #d8a657",
                Keyword.Constant: "nobold #f27da4",
                Operator: "#c2b9a8",
                Operator.Word: "bold #ff750f",
                Name.Builtin: "bold #3a96cf",
                Name.Function: "bold #3a96cf",
                Name.Class: "bold #3a96cf",
                Name.Namespace: "bold #3a96cf",
                Name.Exception: "bold #ff5d80",
                Name.Variable: "#f4efe6",
                Name.Constant: "#d8a657",
                Name.Decorator: "#ff750f",
                Name.Attribute: "#3a96cf",
                Name.Tag: "bold #79b473",
                String: "bold #79b473",
                String.Doc: "italic #9a8f7c",
                String.Escape: "bold #ff750f",
                Number: "bold #c490d1",
                Generic.Heading: "bold #f4efe6",
                Generic.Subheading: "bold #3a96cf",
                Generic.Deleted: "#ff6b6b",
                Generic.Inserted: "#79b473",
                Generic.Error: "#ff6b6b",
                Generic.Emph: "italic",
                Generic.Strong: "bold",
                Generic.Prompt: "bold #9a8f7c",
                Error: "border:#ff6b6b",
            },
        },
    )


CUSTOM_STYLES = {"warm-light": _warm_light(), "warm-dark": _warm_dark()}


def _pygments_theme_css():
    from pygments.formatters import HtmlFormatter

    menu = [s for group in CODE_THEMES.values() for s in group]
    dark_defaults = [p["dark_pygments"] for p in PALETTES.values() if p.get("dark_pygments")]
    light_defaults = [p["light_pygments"] for p in PALETTES.values() if p.get("light_pygments")]

    # A style name may resolve to a builtin (the string) or a registered custom class.
    def make_formatter(style):
        return HtmlFormatter(style=CUSTOM_STYLES.get(style, style))

    def rules(formatter, prefix):
        # get_style_defs prefixes the token rules (and the `.highlight {background}` line)
        # with `prefix`; keep only those, dropping Pygments' un-prefixed globals
        # (pre{}, td.linenos{}) so nothing leaks outside code blocks.
        return "\n".join(
            line
            for line in formatter.get_style_defs(f"{prefix} .highlight").splitlines()
            if line.startswith(f"{prefix} .highlight")
        )

    blocks = []
    # Explicit picks (and any palette default, so it resolves even if absent from the
    # menu) apply in any mode via the order-independent `html body` prefix.
    for style in dict.fromkeys(menu + dark_defaults + light_defaults):
        blocks.append(rules(make_formatter(style), f'html body[data-code-theme="{style}"]'))
    # "Auto" applies a palette's dark/light default, each scoped to its own mode so the
    # other mode keeps the global Pygments style. The auto-mode (`prefers-color-scheme`)
    # variants mirror Furo's `:not([data-theme=…])` selectors for system readers.
    for style in dict.fromkeys(dark_defaults):
        fmt = make_formatter(style)
        blocks.append(rules(fmt, f'body[data-theme="dark"][data-code-auto="{style}"]'))
        auto = rules(fmt, f'body:not([data-theme="light"])[data-code-auto="{style}"]')
        blocks.append("@media (prefers-color-scheme: dark){\n" + auto + "\n}")
    for style in dict.fromkeys(light_defaults):
        fmt = make_formatter(style)
        blocks.append(rules(fmt, f'body[data-theme="light"][data-code-auto-light="{style}"]'))
        auto = rules(fmt, f'body:not([data-theme="dark"])[data-code-auto-light="{style}"]')
        blocks.append("@media (prefers-color-scheme: light){\n" + auto + "\n}")
    return "\n".join(blocks)


# The rendered themes are large, so write them to one linked stylesheet (the browser
# caches it once) instead of inlining them into every page. The file lives in a
# build-only, git-ignored "_generated" static dir that html_static_path picks up.
_generated = os.path.join(os.path.dirname(__file__), "_generated", "css")
os.makedirs(_generated, exist_ok=True)
with open(os.path.join(_generated, "pygments-themes.css"), "w", encoding="utf-8") as _f:
    _f.write(_pygments_theme_css())
html_static_path.append("_generated")
html_css_files.append("css/pygments-themes.css")


# -- In-browser palette + code-theme switcher --------------------------------
#
# Expose the registries to the page (single source of truth) and add the switcher script.
# It always paints the active palette and its default code themes on load (the full
# palette isn't baked into the theme, only its brand colors are), and additionally
# renders the palette/code dropdowns when DOCS_SHOW_SWITCHER is true. Choices recolor the
# live site and persist in localStorage; delete this setup() and js/palette-switcher.js
# to remove it.
def setup(app):
    app.add_js_file(
        None,
        body=(
            f"window.DOCS_PALETTES = {json.dumps(PALETTES)};\n"
            f"window.DOCS_PALETTE_DEFAULT = {json.dumps(ACTIVE_PALETTE)};\n"
            f"window.DOCS_CODE_THEMES = {json.dumps(CODE_THEMES)};\n"
            f"window.DOCS_SHOW_SWITCHER = {json.dumps(SHOW_SWITCHER)};"
        ),
    )
    app.add_js_file("js/palette-switcher.js")
