#[test]
fn generator_navigation_keeps_number_field_and_polynomial_implementations_separate() {
    let consumer = "from sage.all import NumberField, PolynomialRing, QQ\n\
P.<u> = PolynomialRing(QQ)\n\
single_parent_gen = P.gen()\n\
single_degree = u.degree()\n\
Q = PolynomialRing(QQ, 'q')\n\
q = Q.gen()\n\
direct_single_degree = q.degree()\n\
R.<x, y, z> = PolynomialRing(QQ, 3)\n\
multi_parent_gen = R.gen()\n\
multi_degree = x.degree()\n\
K.<a> = NumberField(x^2 + 1)\n\
number_field_polynomial = a.polynomial()\n\
b = K.gen()\n\
number_field_call_polynomial = b.polynomial()\n";
    let (root, index) = strict_navigation_index(
        "strict-generator-representations",
        &[
            (
                "sage/all.py",
                "from sage.rings.number_field.number_field import NumberField\n\
from sage.rings.polynomial.polynomial_ring_constructor import PolynomialRing\n\
from sage.rings.rational_field import QQ\n",
            ),
            (
                "sage/rings/number_field/number_field.py",
                "def NumberField(polynomial, name=None):\n\
    return polynomial\n\
\n\
class NumberField_generic:\n\
    def gen(self):\n\
        return None\n",
            ),
            (
                "sage/rings/number_field/number_field_element.pyx",
                "cdef class NumberFieldElement:\n\
    def polynomial(self, var='x'):\n\
        return None\n",
            ),
            (
                "sage/rings/polynomial/polynomial_ring_constructor.py",
                "def PolynomialRing(base, *args):\n\
    return base\n",
            ),
            (
                "sage/rings/polynomial/polynomial_ring.py",
                "class PolynomialRing_generic:\n\
    def gen(self, n=0):\n\
        return None\n",
            ),
            (
                "sage/rings/polynomial/multi_polynomial_libsingular.pyx",
                "class MPolynomialRing_libsingular:\n\
    def gen(self, n=0):\n\
        return None\n",
            ),
            (
                "sage/rings/polynomial/polynomial_element.pyx",
                "cdef class Polynomial:\n\
    def degree(self, gen=None):\n\
        return 0\n",
            ),
            (
                "sage/rings/polynomial/multi_polynomial_element.py",
                "class MPolynomial:\n\
    def degree(self, x=None):\n\
        return 0\n",
            ),
            ("sage/rings/rational_field.py", "QQ = object()\n"),
            ("consumer.sage", consumer),
        ],
    );
    let consumer_path = root.join("consumer.sage");
    let univariate_ring_path =
        normalize_path(root.join("sage/rings/polynomial/polynomial_ring.py"));
    let multi_ring_path =
        normalize_path(root.join("sage/rings/polynomial/multi_polynomial_libsingular.pyx"));
    let multi_element_path =
        normalize_path(root.join("sage/rings/polynomial/multi_polynomial_element.py"));
    let number_field_element_path =
        normalize_path(root.join("sage/rings/number_field/number_field_element.pyx"));

    for occurrence in 0..2 {
        let (line, character) = nth_member_position(consumer, "gen", occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(
            query.owner_type.as_deref(),
            Some("UnivariatePolynomialRing")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(univariate_ring_path.as_path()),
            "proven single-variable rings should select their exact implementation"
        );
    }

    for (member, occurrence) in [("degree", 0), ("degree", 1)] {
        let (line, character) = nth_member_position(consumer, member, occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_ne!(
            query.resolution_confidence.as_deref(),
            Some("high"),
            "single-variable polynomial evidence must not choose a multivariate target: {query:?}"
        );
        assert_ne!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(if member == "gen" {
                multi_ring_path.as_path()
            } else {
                multi_element_path.as_path()
            }),
            "single-variable polynomial navigation selected a multivariate implementation"
        );
    }

    for (member, occurrence, expected_path) in [
        ("gen", 2, multi_ring_path.as_path()),
        ("degree", 2, multi_element_path.as_path()),
    ] {
        let (line, character) = nth_member_position(consumer, member, occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(expected_path),
            "explicit multivariate preparser evidence should retain an exact target: {query:?}"
        );
    }

    for occurrence in 0..2 {
        let (line, character) = nth_member_position(consumer, "polynomial", occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(
            query.owner_type.as_deref(),
            Some("NumberFieldElement"),
            "NumberField generators need a distinct element owner: {query:?}"
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(number_field_element_path.as_path()),
            "NumberField element polynomial() resolved outside its implementation hierarchy"
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn polyhedron_navigation_uses_constructor_identity_and_exact_base_modules() {
    let consumer = "from sage.all import Polyhedron\n\
from sage.geometry.all import Polyhedron as GeometryPolyhedron\n\
from sage.geometry.polyhedron.all import Polyhedron as ModulePolyhedron\n\
poly = Polyhedron(vertices=[[0, 0], [1, 0], [0, 1]])\n\
vertices = poly.vertices()\n\
dimension = poly.dim()\n\
facets = poly.facets()\n\
volume = poly.volume()\n\
contained = poly.contains([0, 0])\n\
meet = poly.intersection(poly)\n\
dual = poly.polar()\n\
picture = poly.plot()\n\
future = poly.future_invariant()\n\
dual_vertices = dual.vertices()\n\
meet_volume = meet.volume()\n";
    let (root, index) = strict_navigation_index(
        "strict-polyhedron-navigation",
        &[
            ("sage/all.py", "from sage.geometry.all import *\n"),
            (
                "sage/geometry/all.py",
                "from sage.geometry.polyhedron.all import *\n",
            ),
            (
                "sage/geometry/polyhedron/all.py",
                "from sage.misc.lazy_import import lazy_import\n\
lazy_import('sage.geometry.polyhedron.constructor', 'Polyhedron')\n",
            ),
            (
                "sage/geometry/polyhedron/constructor.py",
                "def Polyhedron(vertices=None, **kwds):\n\
    \"\"\"Construct a polyhedron.\"\"\"\n\
    return vertices\n",
            ),
            (
                "sage/geometry/polyhedron/base0.py",
                "class Polyhedron_base0:\n\
    def vertices(self):\n\
        \"\"\"Return the vertices.\"\"\"\n\
        return ()\n",
            ),
            (
                "sage/geometry/polyhedron/base1.py",
                "class Polyhedron_base1:\n\
    def dim(self):\n\
        \"\"\"Return the dimension.\"\"\"\n\
        return 0\n\
\n\
    def contains(self, point):\n\
        \"\"\"Test point containment.\"\"\"\n\
        return False\n",
            ),
            (
                "sage/geometry/polyhedron/base3.py",
                "class Polyhedron_base3:\n\
    def facets(self):\n\
        \"\"\"Return the facets.\"\"\"\n\
        return ()\n",
            ),
            (
                "sage/geometry/polyhedron/base4.py",
                "class Polyhedron_base4:\n\
    def future_invariant(self):\n\
        \"\"\"Return a source-derived future invariant.\"\"\"\n\
        return 0\n",
            ),
            (
                "sage/geometry/polyhedron/base5.py",
                "class Polyhedron_base5:\n\
    def polar(self, in_affine_span=False):\n\
        \"\"\"Return the polar polyhedron.\"\"\"\n\
        return self\n\
\n\
    def intersection(self, other):\n\
        \"\"\"Return the intersection polyhedron.\"\"\"\n\
        return self\n",
            ),
            (
                "sage/geometry/polyhedron/base6.py",
                "class Polyhedron_base6:\n\
    def plot(self, **kwds):\n\
        \"\"\"Plot the polyhedron.\"\"\"\n\
        return None\n",
            ),
            (
                "sage/geometry/polyhedron/base7.py",
                "class Polyhedron_base7:\n\
    def volume(self, measure='ambient', engine='auto', **kwds):\n\
        \"\"\"Return the volume.\"\"\"\n\
        return 0\n",
            ),
            (
                "sage/geometry/polyhedron/base_ZZ.py",
                "class Polyhedron_ZZ:\n\
    def polar(self):\n\
        \"\"\"Return the integer-specialized polar.\"\"\"\n\
        return self\n",
            ),
            (
                "sage/geometry/polyhedron/face.py",
                "class PolyhedronFace:\n\
    def vertices(self):\n\
        \"\"\"Return face vertices, not polyhedron vertices.\"\"\"\n\
        return ()\n",
            ),
            ("consumer.py", consumer),
        ],
    );

    let constructor_path = normalize_path(root.join("sage/geometry/polyhedron/constructor.py"));
    for line_needle in [
        "from sage.all import",
        "from sage.geometry.all import",
        "from sage.geometry.polyhedron.all import",
    ] {
        let (line, character) = position_in_line(consumer, line_needle, "Polyhedron");
        let query = index.query_source_at_navigation(
            &root.join("consumer.py"),
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(constructor_path.as_path()),
            "wrong Polyhedron export target for {line_needle}: {:?}",
            query.definition
        );
    }

    for (member, occurrence, expected_module) in [
        ("vertices", 0, "base0.py"),
        ("dim", 0, "base1.py"),
        ("facets", 0, "base3.py"),
        ("volume", 0, "base7.py"),
        ("contains", 0, "base1.py"),
        ("intersection", 0, "base5.py"),
        ("polar", 0, "base5.py"),
        ("plot", 0, "base6.py"),
        ("future_invariant", 0, "base4.py"),
        ("vertices", 1, "base0.py"),
        ("volume", 1, "base7.py"),
    ] {
        let (line, character) = nth_member_position(consumer, member, occurrence);
        let query = index.query_source_at_navigation(
            &root.join("consumer.py"),
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("Polyhedron"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(
                normalize_path(root.join("sage/geometry/polyhedron").join(expected_module))
                    .as_path()
            ),
            "wrong Polyhedron method target for {member}: {:?}",
            query.definition
        );
    }

    let (completion_line, completion_character) = first_position(consumer, "poly.future_");
    let completions = index.completion_items_at_source(
        consumer,
        QueryPosition {
            line: completion_line,
            character: completion_character + "poly.future_".len() as u32,
        },
        20,
    );
    assert!(
        completions
            .iter()
            .any(|completion| completion.label == "future_invariant"),
        "recursive Polyhedron method cache did not contribute completion: {completions:?}"
    );

    fs::remove_dir_all(root).ok();
}
