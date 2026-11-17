`damaged-support.destruction.json` is a current-format v2 numerical regression
graph: 255 rigid chunks and 845 connections with stable identifiers. Each
connection contains one actual planar convex interface reconstructed from the
adjacent convex proxies in the existing Y-up asset coordinates.

Area, centroid, normal and full area covariance are recomputed from serialized
float32 interface vertices. The conversion preserves every chunk, connection
and support identifier; it does not provide an old-format loading path. The
placeholder mesh name is unused by these solver and headless physics tests.

This fixed graph reproduces weak remaining supports independently of changes
to production partitioning. Production catalog tests continue to load the
actual generated assets separately.
