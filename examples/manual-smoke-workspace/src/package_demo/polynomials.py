"""Package import smoke tests for the manual Sage workspace."""


def named_polynomial(symbol_name: str) -> str:
    """Return a readable quadratic polynomial label."""

    return f"{symbol_name}^2 + 2*{symbol_name} + 1"


class AffineNote:
    """Carry a short label for class-hover and definition tests."""

    def __init__(self, label: str) -> None:
        self.label = label

    def render(self) -> str:
        return f"Affine note<{self.label}>"
