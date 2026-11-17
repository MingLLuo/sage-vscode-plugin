# Configure a Python or Cython project

`.sage` files work without configuration. For other projects, apply the smallest profile that matches:

- **Sage-heavy Python** when `.py` files import from `sage.all`.
- **Native Cython** when `.pyx`, `.pxd`, or `.pxi` files need navigation.
- **Research workspace** when notes, generated files, and Sage code live together.

[Configure Workspace](command:sage.configureWorkspace)
