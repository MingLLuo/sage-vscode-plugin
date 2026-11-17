# Design Notes

Each note describes current behavior and the rule to keep when changing it. The overall structure is in
[ARCHITECTURE.md](../../ARCHITECTURE.md).

| Note | Covers |
| --- | --- |
| [rust-lsp-v2.md](./rust-lsp-v2.md) | language server structure, request lanes, index snapshots |
| [lsp-baseline-features.md](./lsp-baseline-features.md) | supported LSP features and the navigation confidence contract |
| [source-mapping-strategy.md](./source-mapping-strategy.md) | how `.sage` syntax is rewritten and mapped back |
| [native-source-support.md](./native-source-support.md) | Cython (`.pyx`, `.pxd`, `.pxi`) indexing and navigation |
| [runtime-introspection-fallback.md](./runtime-introspection-fallback.md) | asking a running Sage for docs and locations |
| [runtime-source-root-discovery.md](./runtime-source-root-discovery.md) | finding Sage source trees from the installation |
