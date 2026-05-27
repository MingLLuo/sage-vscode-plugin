# Packaged Rust Language Server Binaries

Release builds may place platform-specific `sage-ls` binaries under this directory:

- `darwin-arm64/sage-ls`
- `darwin-x64/sage-ls`
- `linux-x64/sage-ls`
- `win32-x64/sage-ls.exe`

The extension still prefers `SAGE_LS_PATH`, an explicit `sage.languageServer.rustPath`, and repository-local
`target/debug` or `target/release` builds during development. Packaged binaries are used before falling back to
`sage-ls` on `PATH`, so installed VSIX users do not need Rust or Cargo when a matching binary is included.

Build and stage the current host binary with:

```bash
npm run package:rust-binary
```

The staging script also writes `sage-ls.sha256` and `sage-ls.meta.json` next to the binary so release reviews can confirm
which build was included in the VSIX.
