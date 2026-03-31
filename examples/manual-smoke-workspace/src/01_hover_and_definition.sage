from local_docs import make_demo_matrix, PolynomialNotebook, summarize_coefficients
from package_demo import named_polynomial, AffineNote
from external_series import alternating_square_sum, EXTERNAL_LABEL, vendor_banner
from cythonish_bridge import fast_square

R.<x> = PolynomialRing(QQ)

demo_matrix = make_demo_matrix()
poly_preview = named_polynomial("x")
vendor_total = alternating_square_sum(6)
banner_text = vendor_banner("hover")
square_fast = fast_square(7)
label_value = EXTERNAL_LABEL
summary_text = summarize_coefficients([1, 3, 5])
note_box = AffineNote("demo")
notebook = PolynomialNotebook()
