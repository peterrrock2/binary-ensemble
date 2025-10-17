Welcome to PyBen's documentation!
=================================

BEN (short for Binary-Ensemble) is a compression algorithm designed for efficient storage and access
of ensembles of districting plans, and was designed to work primarily as a companion to the
GerrySuite collection of packages (GerryChain, GerryTools, FRCW) and to also be compatible with
other ensemble generators (e.g. ForestRecom, Sequential Monte Carlo [SMC]).

This is a package containing some Python bindings for the for the 
`Binary-Ensemble <https://crates.io/crates/binary-ensemble>` Rust library. In particular,
this package provides some easy tools for compressing and  decompressing ensembles of
districting plans, as well as some utilities for working with ensembles stored in the
BEN and XBEN formats.

Installation
------------

To install PyBen, you can just use pip!

.. code-block:: bash

    pip install binary-ensemble


.. toctree:: 
    :caption: PyBEN Tutorial

    user/using_pyben


.. toctree::
   :caption: API Reference
   :maxdepth: 2

   autoapi/index
