"""Extra-path module for vendor resolution smoke tests."""

EXTERNAL_LABEL = "vendor-series"


def alternating_square_sum(limit: int) -> int:
    """Return an alternating sum of consecutive squares."""

    total = 0
    for index in range(1, limit + 1):
        square = index * index
        total += square if index % 2 else -square
    return total


def vendor_banner(name: str) -> str:
    """Return a stable label for docs-panel and hover checks."""

    return f"vendor::{name}"
