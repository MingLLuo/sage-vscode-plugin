lazy_import("external_series", "alternating_square_sum", "alt_square_sum")
lazy_import("package_demo.polynomials", "named_polynomial")
lazy_import("local_docs", "PolynomialNotebook", "NotebookAlias")

lazy_total = alt_square_sum(5)
lazy_poly = named_polynomial("y")
lazy_notebook = NotebookAlias("lazy")
