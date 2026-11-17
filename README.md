# Sage VS Code Plugin

SageMath support for Visual Studio Code: a fast Rust language server that understands `.sage` files, Sage-heavy
Python, and Sage's Cython sources, plus commands to run code in Sage.

## Install

1. Download the `.vsix` for your machine from the
   [latest release](https://github.com/MingLLuo/sage-vscode-plugin/releases/latest) (`darwin-arm64`, `darwin-x64`,
   `win32-x64`, `win32-arm64`, `linux-x64`, `linux-arm64`, `alpine-x64`, `alpine-arm64`).
2. In VS Code run **Extensions: Install from VSIX...** and pick the file. In Remote SSH, WSL, or Dev Container windows,
   pick the file for the remote machine.
3. Open a folder with a `.sage` file. Sage sources nearby or in your Sage installation are found automatically.

The [User Guide](./docs/install-and-configure.md) covers settings, Sage-heavy Python projects, and troubleshooting.

## What You Get

| Area | Features |
| --- | --- |
| Understanding code | hover with rendered documentation, completion, signature help, inlay hints |
| Navigation | go to definition / declaration / implementation, references, rename, call hierarchy, document symbols, workspace symbols |
| Feedback | syntax diagnostics, semantic tokens (highlighting), folding |
| Running | Run Current File, Run Selection in REPL (`Shift+Enter`), Run Cell (`Ctrl/Cmd+Enter` in files with `# %%` cells), managed REPL |
| Help | Getting Started walkthrough, status bar with recovery actions, Show Environment / Index / Docs Status, Copy Support Bundle |

Navigation works from Sage source files, so Sage does not have to be installed for editing. Running code and
runtime-backed documentation use your Sage installation. The interface follows VS Code's display language (English or
Simplified Chinese).

Find References for a common name such as `matrix` across the whole Sage library returns in about 1 s, and the server
answers hover and completion while it indexes. Measurements are in [docs/perf](./docs/perf/README.md).

## Documentation

- [User Guide](./docs/install-and-configure.md): install, settings, commands, troubleshooting.
- [Changelog](./packages/extension-core/CHANGELOG.md)
- [Developer Guide](./docs/developer-guide.md): build, test gates, debugging, release process.
- [Architecture](./ARCHITECTURE.md) and [design notes](./docs/design/)
- [Contributing](./CONTRIBUTING.md), [Support](./SUPPORT.md), [Security](./SECURITY.md)

## Build From Source

```bash
npm ci
npm run package:vsix   # writes dist/sage-vscode-extension-<version>-<platform>.vsix for this machine
code --install-extension dist/sage-vscode-extension-0.2.0-darwin-arm64.vsix --force
```

Requires Node.js 22.9+ and npm 11+; Rust is pinned by `rust-toolchain.toml`. See the
[Developer Guide](./docs/developer-guide.md) for the development host, tests, and tools such as
`npm run configure:workspace`, `npm run doctor:mac`, and `npm run export:reference` (an offline, browser-only project
reference).

## Limits

- No notebook or Jupyter kernel integration yet.
- Packages are distributed through GitHub releases only; they are not signed or published to the Marketplace.
