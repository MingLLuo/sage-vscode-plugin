# Packaged Rust Language Server Binaries

The packager supports these native binary directories:

- `darwin-arm64/sage-ls`, `darwin-x64/sage-ls`
- `linux-arm64/sage-ls`, `linux-x64/sage-ls` (musl, also used by the corresponding Alpine VSIX)
- `win32-arm64/sage-ls.exe`, `win32-x64/sage-ls.exe`

The launch order is explicit `sage.languageServer.rustPath`, `SAGE_LS_PATH`, matching packaged binary,
repository `target/debug` or `target/release` only in development mode, then `sage-ls` on `PATH`.

Build the current host target with `npm run package:vsix`, or select a target explicitly:

```bash
npm run package:vsix -- --target darwin-arm64
```

Cross-compilation requires the Rust target and native compiler/linker. CI uses six native runners to produce eight VSIX
platform identifiers: macOS, Linux, Alpine, and Windows, each for x64 and arm64. Each VSIX contains only its own binary,
SHA-256, build metadata, and generated third-party notices. Windows uses static CRT; Linux uses musl and mimalloc.

Every platform is built and tested on its native GitHub runner (`package.yml`) before a release is published.
