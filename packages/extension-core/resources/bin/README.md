# Packaged Rust Language Server Binaries

Local macOS release builds may place `sage-ls` binaries under this directory:

- `darwin-arm64/sage-ls`
- `darwin-x64/sage-ls`

The extension still prefers `SAGE_LS_PATH`, an explicit `sage.languageServer.rustPath`, and repository-local
`target/debug` or `target/release` builds during development. Packaged macOS binaries are used before falling back to
`sage-ls` on `PATH`, so local VSIX users do not need Rust or Cargo when a matching binary is included.

Windows and Linux packaging are not release targets for this preview. Some scripts retain defensive platform handling for
tests, but the maintained install path is macOS.

Build and stage the current host binary with:

```bash
npm run package:rust-binary
```

The staging script also writes `sage-ls.sha256` and `sage-ls.meta.json` next to the binary so release reviews can confirm
which build was included in the VSIX.
