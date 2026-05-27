# Support

Use the issue templates so maintainers receive enough context to reproduce Sage-specific editor behavior.

## Before Opening an Issue

1. Run `Sage: Show Environment Details`.
2. Run `Sage: Show Index Status` and `Sage: Show Docs Status`.
3. For hover, definition, completion, references, rename, or diagnostics, run `Sage: Run UX Self Check` at the affected
   cursor position.
4. If possible, run `npm run test:ci` for repository changes or `npm run test:release` for release-candidate changes.

## What to Include

- File type: `.sage`, Sage-heavy `.py`, `.pyx`, `.pxd`, or `.pxi`.
- Whether the workspace is trusted and local.
- Sage source root or runtime shape, without private path details if needed.
- `Sage: Copy Support Bundle` output with private paths redacted if necessary.
- Relevant lines from the `Sage` and `Sage Language Server` output channels.

## Security

For vulnerabilities, follow `SECURITY.md` instead of opening a detailed public issue.
