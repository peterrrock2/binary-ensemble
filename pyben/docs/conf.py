# -*- coding: utf-8 -*-
#
# Configuration file for the Sphinx documentation builder.
#
# This file does only contain a selection of the most common options. For a
# full list see the documentation:
# http://www.sphinx-doc.org/en/master/config

import os
import sys


# -- Path setup --------------------------------------------------------------

# If extensions (or modules to document with autodoc) are in another directory,
# add these directories to sys.path here. If the directory is relative to the
# documentation root, use os.path.abspath to make it absolute, like shown here.
#
# import os
# import sys
# sys.path.insert(0, os.path.abspath('.'))

# autodoc needs to find our code.
sys.path.insert(0, os.path.abspath("../src"))

# -- Project information -----------------------------------------------------

project = "PyBen"
copyright = "2025, Peter Rock"
author = "Peter Rock"

# The short X.Y version
version = ""
# The full version, including alpha/beta/rc tags
release = ""

# -- General configuration ---------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#general-configuration

extensions = [
    "autoapi.extension",
    "sphinx.ext.coverage",
    "sphinx.ext.napoleon",
    "sphinx.ext.mathjax",
    "sphinx.ext.viewcode",
    "sphinx.ext.intersphinx",
    "sphinx_copybutton",
    "myst_nb",
]
nb_execution_mode = "off"  # render outputs already in the .ipynb; no execution
templates_path = ["_templates"]
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]

autoapi_type = "python"
autoapi_dirs = ["../pyben"]
autoapi_clean = True
autoapi_keep_files = False
autoapi_ignore = [
    "../docs/**",
    "**/_build/**",
    "**/.venv/**",
    "**/tests/**",
    "**/examples/**",
    "**/notebooks/**",
]
autoapi_options = [
    "members",
    "undoc-members",
    "show-inheritance",
    "show-module-summary",
    "imported-members",
]

# -- Options for HTML output -------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#options-for-html-output

html_theme = "sphinx_rtd_theme"
html_theme_options = {"style_nav_header_background": "#0099cd"}
html_static_path = ["_static"]

# The name of the Pygments (syntax highlighting) style to use.
pygments_style = "sphinx"

html_css_files = ["css/custom.css"]

# -- Extension configuration -------------------------------------------------

# Prepend the module name of classes.
add_module_names = True
autodoc_inherit_docstrings = False
