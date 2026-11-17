`rectangle-v1.destruction.json` freezes the former planar 255-chunk rectangle
(generator 0.2.0, seed 7) for damaged-support solver and contact regressions.
These tests identify the original failing support interfaces and must survive
changes to production asset partitioning. No display GLB is required by them.

The production catalog tests still load `assets/buildings` directly. New
irregular geometry is validated independently by the catalog, landing, collapse
and force-overlay acceptance paths; passing this historical fixture is not
evidence that new assets pass those checks.
