# fixture-tests: v1 registry test data

These are ZenSight's v1 registry files: `registry/` in TOML, `registry-kdl/`
in KDL. The v1 tools' tests read them as data: zenctl's trycmd cases,
zenkey-fleet's tests and benches, zengui's scene fixtures and the `just
gui-demo` recipe.

The crate that compiled them through `zenkey-build` as a codegen regression
corpus lives on the `v1` branch, beside the v1 `zenkey` and `zenkey-build`
sources. `main` is the zk2 line in the strangler layout (#615). These files
stay at this path so that every test that prints the path keeps its
snapshot.
