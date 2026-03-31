# Source Mapping Strategy

## Problem

Sage `.sage` files are not plain Python. Rich editor features will eventually require a mapping layer between user
source and the Python-oriented analysis model used by the language server.

## Bootstrap Decision

Bootstrap work will not implement full preparser mapping yet. Instead it will:

- keep the server architecture ready for a mapping component
- document source mapping as a high-risk design area
- avoid promising precise diagnostics or navigation for transformed syntax in the first scaffold

## Expected Next Design Step

The first real mapping design note should define:

- which `.sage` constructs are supported first
- whether mapping is token-based, line-based, or rewrite-based
- how diagnostics and edits are projected back into original source ranges

