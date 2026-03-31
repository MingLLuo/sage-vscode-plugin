"""Workspace-local symbols for hover, docs, and definition smoke tests."""

DEFAULT_DIMENSION = 3


def make_demo_matrix() -> list[list[int]]:
    """Return a small nested list that looks like a matrix.

    The implementation stays pure Python so the static language server can
    inspect it without depending on a live Sage runtime.
    """

    return [[1, 2], [3, 5]]


def summarize_coefficients(values: list[int]) -> str:
    """Return a comma-separated summary for documentation and hover tests."""

    return ", ".join(str(value) for value in values)


class PolynomialNotebook:
    """Minimal class used for class hover and definition checks."""

    def __init__(self, title: str = "demo") -> None:
        self.title = title

    def describe(self) -> str:
        return f"Polynomial notebook: {self.title}"
