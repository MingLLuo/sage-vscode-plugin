# Security Policy

## Supported Versions

This repository is still a preview package. Security fixes are handled on the active `main` branch until versioned release
branches exist.

## Reporting a Vulnerability

Do not post exploit details, malicious workspaces, or secrets in a public issue.

Preferred reporting path:

1. Use GitHub private vulnerability reporting for this repository when it is enabled.
2. If private reporting is not available yet, open a minimal public issue asking for a maintainer security contact and do
   not include exploit details.

Please include:

- Affected version or commit.
- Whether the issue requires opening a workspace, running Sage code, installing a VSIX, or changing settings.
- Minimal reproduction details that do not expose private code or credentials.
- Any relevant `Sage: Copy Support Bundle` fields after removing private paths or tokens.

The extension starts local processes for language services, documentation, and Sage execution. Issues involving workspace
trust, command execution, path handling, packaged binaries, or generated VSIX contents are security-relevant and should be
reported through the private path above.
