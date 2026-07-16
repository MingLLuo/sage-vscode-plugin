use super::*;

#[test]
fn query_resolves_sage_all_reexports_to_source_modules() {
    let root = test_root("sage-all-reexports");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::create_dir_all(root.join("sage/modules")).unwrap();
    fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
    fs::write(
        root.join("sage/matrix/constructor.pyx"),
        "def matrix(*args):\n    \"\"\"Create a Sage matrix.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/matrix/special.py"),
        "def zero_matrix(*args):\n    \"\"\"Create a zero matrix.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/modules/free_module_element.pyx"),
        "def vector(*args):\n    \"\"\"Create a vector.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/rings/finite_rings/finite_field_constructor.py"),
        "def GF(order, name=None):\n    \"\"\"Return a finite field.\"\"\"\n    return order\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import (\n    matrix,\n    vector,\n    GF,\n    zero_matrix,\n)\nfield = GF(7)\nmat = matrix(field, 2, 2)\nvec = vector(field, [1, 2])\nzero = zero_matrix(field, 2, 2)\n";
    fs::write(&consumer, source).unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    index.rebuild().unwrap();

    for (name, expected_path, expected_doc) in [
        (
            "matrix",
            root.join("sage/matrix/constructor.pyx"),
            "Create a Sage matrix.",
        ),
        (
            "vector",
            root.join("sage/modules/free_module_element.pyx"),
            "Create a vector.",
        ),
        (
            "GF",
            root.join("sage/rings/finite_rings/finite_field_constructor.py"),
            "Return a finite field.",
        ),
        (
            "zero_matrix",
            root.join("sage/matrix/special.py"),
            "Create a zero matrix.",
        ),
    ] {
        let query = index.query_source_symbol(&consumer, source, name, None, None, Vec::new());
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong definition for {name}: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc)
        );
    }

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    for (mode, index) in [("rebuilt", &index), ("hydrated", &hydrated)] {
        for (line_prefix, name, expected_path) in [
            ("mat =", "matrix", root.join("sage/matrix/constructor.pyx")),
            ("zero =", "zero_matrix", root.join("sage/matrix/special.py")),
        ] {
            let (line, character) = position_in_line(source, line_prefix, name);
            let query = index.query_source_at_navigation(
                &consumer,
                source,
                QueryPosition { line, character },
            );
            assert_eq!(
                query
                    .definition
                    .as_ref()
                    .map(|definition| definition.path.as_path()),
                Some(normalize_path(expected_path).as_path()),
                "wrong {mode} position-based definition for {name}: {:?}",
                query.definition
            );
        }
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_sage_all_wildcard_exports_before_global_homonyms() {
    let root = test_root("sage-all-wildcard-exports");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::create_dir_all(root.join("sage/modules")).unwrap();
    fs::write(
        root.join("sage/matrix/constructor.pyx"),
        "def matrix(*args):\n    \"\"\"Create a Sage matrix.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/matrix/special.py"),
        "def zero_matrix(*args):\n    \"\"\"Create a zero matrix.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/modules/free_module_element.pyx"),
        "def vector(*args):\n    \"\"\"Create a vector.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
            root.join("homonyms.py"),
            "def matrix(self):\n    return self\n\ndef vector(self):\n    return self\n\ndef zero_matrix(self):\n    return self\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import *\nmat = matrix(GF(7), 2, 2)\nvec = vector(GF(7), [1, 2])\nzero = zero_matrix(GF(7), 2, 2)\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (name, expected_path) in [
        ("matrix", root.join("sage/matrix/constructor.pyx")),
        ("vector", root.join("sage/modules/free_module_element.pyx")),
        ("zero_matrix", root.join("sage/matrix/special.py")),
    ] {
        let query = index.query_source_symbol(&consumer, source, name, None, None, Vec::new());
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong wildcard definition for {name}: {:?}",
            query.definition
        );
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_indexed_sage_all_reexport_chains_without_hardcoded_map() {
    let root = test_root("sage-all-dynamic-reexports");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(
            root.join("sage/all.py"),
            "from sage.future.all import ChainFactory\nfrom sage.future.module import FuturePolynomialFactory as FutureFactory\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "from sage.future.module import ChainedFutureFactory as ChainFactory\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/module.py"),
            "def FuturePolynomialFactory(*args):\n    \"\"\"Build a future Sage polynomial factory.\"\"\"\n    return args\n\n\ndef ChainedFutureFactory(*args):\n    \"\"\"Build a chained future Sage factory.\"\"\"\n    return args\n",
        )
        .unwrap();
    fs::write(
            root.join("homonyms.py"),
            "def FutureFactory(*args):\n    \"\"\"Wrong local homonym.\"\"\"\n    return args\n\n\ndef ChainFactory(*args):\n    \"\"\"Wrong chained homonym.\"\"\"\n    return args\n",
        )
        .unwrap();
    let wildcard_consumer = root.join("wildcard_consumer.py");
    let wildcard_source =
        "from sage.all import *\nvalue = FutureFactory()\nchained = ChainFactory()\n";
    fs::write(&wildcard_consumer, wildcard_source).unwrap();
    let explicit_consumer = root.join("explicit_consumer.py");
    let explicit_source = "from sage.all import FutureFactory\nvalue = FutureFactory()\n";
    fs::write(&explicit_consumer, explicit_source).unwrap();

    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (name, expected_doc) in [
        ("FutureFactory", "Build a future Sage polynomial factory."),
        ("ChainFactory", "Build a chained future Sage factory."),
    ] {
        let query = index.query_source_symbol(
            &wildcard_consumer,
            wildcard_source,
            name,
            None,
            None,
            Vec::new(),
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert!(query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("sage.all")));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/future/module.py")).as_path()),
            "wrong dynamic wildcard definition for {name}: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc)
        );
    }

    let explicit_query = index.query_source_symbol(
        &explicit_consumer,
        explicit_source,
        "FutureFactory",
        None,
        None,
        Vec::new(),
    );
    assert_eq!(
        explicit_query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert_eq!(
        explicit_query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Build a future Sage polynomial factory.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_source_derived_catalog_namespace_members() {
    let root = test_root("sage-catalog-namespace-members");
    fs::create_dir_all(root.join("sage/coding")).unwrap();
    fs::create_dir_all(root.join("sage/schemes/toric")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.coding.all import *\nfrom sage.schemes.toric.all import *\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/coding/all.py"),
            "from sage.misc.lazy_import import lazy_import\nlazy_import('sage.coding', 'codes_catalog', 'codes')\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/coding/codes_catalog.py"),
            "from sage.misc.lazy_import import lazy_import as _lazy_import\n_lazy_import('.hamming_code', 'HammingCode')\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/coding/hamming_code.py"),
        "class HammingCode:\n    \"\"\"Representation of a Hamming code.\"\"\"\n    pass\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/schemes/toric/all.py"),
            "from sage.misc.lazy_import import lazy_import\nlazy_import('sage.schemes.toric.library', 'toric_varieties')\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/schemes/toric/library.py"),
            "class ToricVarietyFactory:\n    def P2(self):\n        \"\"\"Return the projective plane.\"\"\"\n        return self\n\ntoric_varieties = ToricVarietyFactory()\n",
        )
        .unwrap();
    let consumer = root.join("consumer.sage");
    let source =
        "from sage.all import *\nC = codes.HammingCode(GF(2), 3)\nX = toric_varieties.P2()\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let (line, character) = first_position(source, "codes");
    let query = index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(query
        .resolution_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("Sage namespace member")));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/coding/hamming_code.py")).as_path()),
        "wrong catalog member definition: {:?}",
        query.definition
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Representation of a Hamming code.")
    );

    let (line, character) = first_position(source, "toric_varieties");
    let query = index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/schemes/toric/library.py")).as_path()),
        "wrong factory namespace member definition: {:?}",
        query.definition
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return the projective plane.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_source_derived_staticmethod_namespace_members() {
    let root = test_root("sage-staticmethod-namespace-members");
    fs::create_dir_all(root.join("sage/graphs/generators")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.graphs.all import *\n").unwrap();
    fs::write(
        root.join("sage/graphs/all.py"),
        "from sage.graphs.graph_generators import graphs\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/graphs/graph_generators.py"),
            "class GraphGenerators:\n    from sage.graphs.generators import smallgraphs\n    PetersenGraph = staticmethod(smallgraphs.PetersenGraph)\n\ngraphs = GraphGenerators()\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/graphs/generators/smallgraphs.py"),
            "def PetersenGraph(immutable=False):\n    \"\"\"Return the Petersen Graph.\"\"\"\n    return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.sage");
    let source = "value = graphs.PetersenGraph()\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let (line, character) = first_position(source, "graphs");
    let query = index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/graphs/generators/smallgraphs.py")).as_path()),
        "wrong staticmethod namespace member definition: {:?}",
        query.definition
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return the Petersen Graph.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_sage_method_owners_and_suppresses_wrong_global_fallback() {
    let root = test_root("sage-method-owners");
    fs::create_dir_all(root.join("sage/combinat/matrices")).unwrap();
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::create_dir_all(root.join("sage/rings/polynomial")).unwrap();
    fs::create_dir_all(root.join("sage/calculus")).unwrap();
    fs::write(
        root.join("sage/combinat/matrices/latin.py"),
        "def dumps(value):\n    \"\"\"Wrong json.dumps fallback.\"\"\"\n    return value\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def rank(self):\n    \"\"\"Return matrix rank.\"\"\"\n    return 0\n\ndef base_ring(self):\n    \"\"\"Return matrix base ring.\"\"\"\n    return None\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/matrix/matrix2.pyx"),
        "def right_kernel(self):\n    \"\"\"Return matrix right kernel.\"\"\"\n    return None\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/rings/polynomial/multi_polynomial.pyx"),
            "def derivative(self, *args):\n    \"\"\"Differentiate this polynomial.\"\"\"\n    return self\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/rings/polynomial/multi_polynomial_libsingular.pyx"),
            "def ideal(self, *args):\n    \"\"\"Create an ideal from this ring.\"\"\"\n    return args\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/rings/polynomial/multi_polynomial_ideal.py"),
        "def variety(self, **kwds):\n    \"\"\"Return ideal variety.\"\"\"\n    return []\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/calculus/functional.py"),
            "def derivative(x):\n    \"\"\"Wrong global derivative fallback.\"\"\"\n    return x\n\ndef append(x):\n    return x\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import GF, PolynomialRing, matrix\nfield = GF(7)\nring = PolynomialRing(field, names=[\"x\"])\nmat = matrix(field, 2, 2)\nQ = matrix(field, 2, 2)\nA = matrix(field, 2, 2)\npoly = ring.gen(0)\nrank_value = mat.rank()\nqs_field = Q.base_ring()\nkernel = A.right_kernel()\njac = matrix(ring, 1, 1, lambda i, j: poly.derivative(ring.gen(0)))\nideal = ring.ideal([poly])\nroots = ideal.variety()\nno_jump = mat.append(1)\nencoded = json.dumps({})\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (needle, expected_owner, expected_path) in [
        ("rank", "Matrix", root.join("sage/matrix/matrix0.pyx")),
        ("base_ring", "Matrix", root.join("sage/matrix/matrix0.pyx")),
        (
            "right_kernel",
            "Matrix",
            root.join("sage/matrix/matrix2.pyx"),
        ),
        (
            "derivative",
            "PolynomialElement",
            root.join("sage/rings/polynomial/multi_polynomial.pyx"),
        ),
        (
            "ideal",
            "PolynomialRing",
            root.join("sage/rings/polynomial/multi_polynomial_libsingular.pyx"),
        ),
        (
            "variety",
            "Ideal",
            root.join("sage/rings/polynomial/multi_polynomial_ideal.py"),
        ),
    ] {
        let (line, character) = member_position(source, needle);
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.owner_type.as_deref(), Some(expected_owner));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(query.candidate_count, 1);
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong method target for {needle}: {:?}",
            query.definition
        );
    }

    let (line, character) = member_position(source, "append");
    let query = index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
    assert_eq!(query.owner_type.as_deref(), Some("Matrix"));
    assert!(query.definition.is_none(), "{:?}", query.definition);
    assert!(query.fallback_reason.is_some(), "{query:?}");
    let (line, character) = member_position(source, "dumps");
    let query = index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
    assert!(
        query.definition.is_none(),
        "unknown dotted stdlib member should not jump to Sage homonym: {query:?}"
    );
    assert!(query.fallback_reason.is_some(), "{query:?}");
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_graph_curve_and_number_field_methods() {
    let root = test_root("sage-object-methods");
    fs::create_dir_all(root.join("sage/graphs/base")).unwrap();
    fs::create_dir_all(root.join("sage/graphs")).unwrap();
    fs::create_dir_all(root.join("sage/schemes/elliptic_curves")).unwrap();
    fs::create_dir_all(root.join("sage/rings/number_field")).unwrap();
    fs::write(
        root.join("sage/graphs/generic_graph.py"),
        "def vertices(self):\n    \"\"\"Return graph vertices.\"\"\"\n    return []\n\n\
def shortest_path(self, u, v):\n    \"\"\"Return a shortest path.\"\"\"\n    return []\n\n\
def adjacency_matrix(self):\n    \"\"\"Return the graph adjacency matrix.\"\"\"\n    return None\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/graphs/base/c_graph.pyx"),
            "def is_connected(self):\n    \"\"\"Return whether the graph is connected.\"\"\"\n    return True\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/schemes/elliptic_curves/ell_finite_field.py"),
        "def points(self):\n    \"\"\"Return rational points.\"\"\"\n    return []\n\n\
def cardinality(self):\n    \"\"\"Return finite-curve cardinality.\"\"\"\n    return 0\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/schemes/elliptic_curves/ell_rational_field.py"),
        "def rank(self):\n    \"\"\"Return Mordell-Weil rank.\"\"\"\n    return 0\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/rings/number_field/number_field.py"),
            "def NumberField(polynomial, name=None):\n    \"\"\"Construct a number field.\"\"\"\n    return polynomial\n\n\
def gen(self, n=0):\n    \"\"\"Return a number field generator.\"\"\"\n    return n\n\n\
def degree(self):\n    \"\"\"Return the number field degree.\"\"\"\n    return 0\n\n\
def discriminant(self):\n    \"\"\"Return the number field discriminant.\"\"\"\n    return 0\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/rings/number_field/number_field_base.pyx"),
            "def ring_of_integers(self):\n    \"\"\"Return the ring of integers.\"\"\"\n    return self\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source =
        "from sage.all import Graph, DiGraph, EllipticCurve, NumberField, GF, PolynomialRing, QQ\n\
R = PolynomialRing(QQ, \"x\")\n\
x = R.gen()\n\
G = Graph([(0, 1), (1, 2)])\n\
DG = DiGraph({0: [1]})\n\
vertices = G.vertices()\n\
connected = G.is_connected()\n\
adjacency = G.adjacency_matrix()\n\
path = DG.shortest_path(0, 1)\n\
E = EllipticCurve(GF(431), [0, 1])\n\
pts = E.points()\n\
cardinality = E.order()\n\
mw_rank = E.rank()\n\
K = NumberField(x**2 + 1, \"a\")\n\
a = K.gen()\n\
degree = K.degree()\n\
integers = K.ring_of_integers()\n\
disc = K.discriminant()\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (needle, occurrence, expected_owner, expected_path, expected_doc) in [
        (
            "vertices",
            0,
            "Graph",
            root.join("sage/graphs/generic_graph.py"),
            "Return graph vertices.",
        ),
        (
            "is_connected",
            0,
            "Graph",
            root.join("sage/graphs/base/c_graph.pyx"),
            "Return whether the graph is connected.",
        ),
        (
            "adjacency_matrix",
            0,
            "Graph",
            root.join("sage/graphs/generic_graph.py"),
            "Return the graph adjacency matrix.",
        ),
        (
            "shortest_path",
            0,
            "Graph",
            root.join("sage/graphs/generic_graph.py"),
            "Return a shortest path.",
        ),
        (
            "points",
            0,
            "EllipticCurve",
            root.join("sage/schemes/elliptic_curves/ell_finite_field.py"),
            "Return rational points.",
        ),
        (
            "order",
            0,
            "EllipticCurve",
            root.join("sage/schemes/elliptic_curves/ell_finite_field.py"),
            "Return finite-curve cardinality.",
        ),
        (
            "rank",
            0,
            "EllipticCurve",
            root.join("sage/schemes/elliptic_curves/ell_rational_field.py"),
            "Return Mordell-Weil rank.",
        ),
        (
            "gen",
            1,
            "NumberField",
            root.join("sage/rings/number_field/number_field.py"),
            "Return a number field generator.",
        ),
        (
            "degree",
            0,
            "NumberField",
            root.join("sage/rings/number_field/number_field.py"),
            "Return the number field degree.",
        ),
        (
            "ring_of_integers",
            0,
            "NumberField",
            root.join("sage/rings/number_field/number_field_base.pyx"),
            "Return the ring of integers.",
        ),
        (
            "discriminant",
            0,
            "NumberField",
            root.join("sage/rings/number_field/number_field.py"),
            "Return the number field discriminant.",
        ),
    ] {
        let (line, character) = nth_member_position(source, needle, occurrence);
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.owner_type.as_deref(), Some(expected_owner));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong object method target for {needle}: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc),
            "wrong object method docs for {needle}: {:?}",
            query.documentation
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn type_definition_resolves_sage_object_variables() {
    let root = test_root("sage-type-definition");
    fs::create_dir_all(root.join("sage/graphs")).unwrap();
    fs::create_dir_all(root.join("sage/schemes/elliptic_curves")).unwrap();
    fs::create_dir_all(root.join("sage/rings/number_field")).unwrap();
    fs::write(
        root.join("sage/graphs/graph.py"),
        "class Graph:\n    \"\"\"Graph type docs.\"\"\"\n    pass\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/schemes/elliptic_curves/constructor.py"),
        "def EllipticCurve(*args):\n    \"\"\"Build an elliptic curve.\"\"\"\n    return args\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/rings/number_field/number_field.py"),
            "def NumberField(polynomial, name=None):\n    \"\"\"Construct a number field.\"\"\"\n    return polynomial\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import Graph, EllipticCurve, NumberField, GF\n\
graph = Graph([(0, 1)])\n\
curve = EllipticCurve(GF(431), [0, 1])\n\
field = NumberField(poly, \"a\")\n\
graph_vertices = graph.vertices()\n\
curve_points = curve.points()\n\
field_degree = field.degree()\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (needle, expected_path, expected_name) in [
        ("graph.vertices", root.join("sage/graphs/graph.py"), "Graph"),
        (
            "curve.points",
            root.join("sage/schemes/elliptic_curves/constructor.py"),
            "EllipticCurve",
        ),
        (
            "field.degree",
            root.join("sage/rings/number_field/number_field.py"),
            "NumberField",
        ),
    ] {
        let (line, character) = first_position(source, needle);
        let definition = index
            .type_definition_at_source(&consumer, source, QueryPosition { line, character })
            .expect("type definition should resolve");
        assert_eq!(definition.name, expected_name);
        assert_eq!(
            definition.path.as_path(),
            normalize_path(expected_path).as_path(),
            "wrong type definition target for {needle}: {definition:?}"
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_research_helper_sage_methods() {
    let root = test_root("research-helper-methods");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::create_dir_all(root.join("sage/modules")).unwrap();
    fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
    fs::create_dir_all(root.join("sage/rings/polynomial")).unwrap();
    fs::create_dir_all(root.join("sage/structure")).unwrap();
    fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def change_ring(self, ring):\n    \"\"\"Return this matrix over another ring.\"\"\"\n    return self\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/matrix/matrix1.pyx"),
            "def matrix_from_columns(self, columns):\n    \"\"\"Return a matrix built from selected columns.\"\"\"\n    return self\n\n\ndef matrix_from_rows(self, rows):\n    \"\"\"Return a matrix built from selected rows.\"\"\"\n    return self\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/matrix/matrix2.pyx"),
            "def charpoly(self, var='x'):\n    \"\"\"Return the characteristic polynomial.\"\"\"\n    return None\n\n\ndef adjugate(self):\n    \"\"\"Return the adjugate matrix.\"\"\"\n    return self\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/modules/free_module.py"),
            "def basis(self):\n    \"\"\"Return a module basis.\"\"\"\n    return []\n\n\ndef basis_matrix(self, ring=None):\n    \"\"\"Return a matrix whose rows are a basis.\"\"\"\n    return None\n\n\ndef dimension(self):\n    \"\"\"Return the module dimension.\"\"\"\n    return 0\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/rings/polynomial/polynomial_element_generic.py"),
            "def list(self, copy=True):\n    \"\"\"Return polynomial coefficients as a list.\"\"\"\n    return []\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/rings/polynomial/polynomial_element.pyx"),
            "def factor(self, **kwargs):\n    \"\"\"Factor this polynomial.\"\"\"\n    return []\n\n\ndef monic(self):\n    \"\"\"Return the monic polynomial.\"\"\"\n    return self\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/rings/polynomial/multi_polynomial_element.py"),
        "def degree(self, x=None):\n    \"\"\"Return the polynomial degree.\"\"\"\n    return 0\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/rings/finite_rings/finite_field_base.pyx"),
            "def order(self):\n    \"\"\"Return the finite field order.\"\"\"\n    return 0\n\n\ndef from_integer(self, n, reverse=False):\n    \"\"\"Create a field element from an integer.\"\"\"\n    return n\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/rings/finite_rings/element_base.pyx"),
            "def to_integer(self, reverse=False):\n    \"\"\"Return this finite-field element as an integer.\"\"\"\n    return 0\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/rings/finite_rings/element_givaro.pyx"),
            "def polynomial(self, name=None):\n    \"\"\"Return this finite-field element as a polynomial.\"\"\"\n    return None\n\n\ndef _integer_representation(self):\n    \"\"\"Return the packed integer representation.\"\"\"\n    return 0\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/structure/element.pyx"),
            "def base_ring(self):\n    \"\"\"Return the base ring of this element.\"\"\"\n    return None\n",
        )
        .unwrap();
    fs::write(
            root.join("homonyms.py"),
            "def list(value):\n    return value\n\ndef change_ring(value):\n    return value\n\ndef base_ring(value):\n    return value\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "def helper(A, poly, vec_obj, symbolic_matrix_obj, substitutions, field, f, y):\n    kernel = A.right_kernel()\n    direct_basis = A.right_kernel().basis()\n    basis = kernel.basis_matrix()\n    dims = kernel.dimension()\n    value = field.from_integer(field.order())\n    packed = value.integer_representation()\n    elem_poly = y.polynomial()\n    elem_coeffs = y.polynomial().list()\n    coeffs = poly.list()\n    cp = symbolic_matrix_obj.charpoly()\n    factors = cp.factor()\n    monic = f.monic()\n    local_ring = f.parent()\n    f1 = local_ring(f)\n    deg = f1.degree()\n    submatrix = symbolic_matrix_obj.matrix_from_columns([0]).matrix_from_rows([0]).adjugate()\n    changed = symbolic_matrix_obj.subs(substitutions).change_ring(field)\n    base = vec_obj.base_ring()\n    return basis, direct_basis, dims, packed, elem_poly, elem_coeffs, coeffs, factors, monic, deg, submatrix, value, changed, base\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (needle, _expected_owner, expected_path, expected_doc) in [
        (
            "basis",
            "FreeModule",
            root.join("sage/modules/free_module.py"),
            "Return a module basis.",
        ),
        (
            "basis_matrix",
            "FreeModule",
            root.join("sage/modules/free_module.py"),
            "Return a matrix whose rows are a basis.",
        ),
        (
            "dimension",
            "FreeModule",
            root.join("sage/modules/free_module.py"),
            "Return the module dimension.",
        ),
        (
            "list",
            "PolynomialElement",
            root.join("sage/rings/polynomial/polynomial_element_generic.py"),
            "Return polynomial coefficients as a list.",
        ),
        (
            "integer_representation",
            "FieldElement",
            root.join("sage/rings/finite_rings/element_base.pyx"),
            "Return this finite-field element as an integer.",
        ),
        (
            "polynomial",
            "FieldElement",
            root.join("sage/rings/finite_rings/element_givaro.pyx"),
            "Return this finite-field element as a polynomial.",
        ),
        (
            "charpoly",
            "Matrix",
            root.join("sage/matrix/matrix2.pyx"),
            "Return the characteristic polynomial.",
        ),
        (
            "factor",
            "PolynomialElement",
            root.join("sage/rings/polynomial/polynomial_element.pyx"),
            "Factor this polynomial.",
        ),
        (
            "monic",
            "PolynomialElement",
            root.join("sage/rings/polynomial/polynomial_element.pyx"),
            "Return the monic polynomial.",
        ),
        (
            "degree",
            "PolynomialElement",
            root.join("sage/rings/polynomial/multi_polynomial_element.py"),
            "Return the polynomial degree.",
        ),
        (
            "matrix_from_columns",
            "Matrix",
            root.join("sage/matrix/matrix1.pyx"),
            "Return a matrix built from selected columns.",
        ),
        (
            "matrix_from_rows",
            "Matrix",
            root.join("sage/matrix/matrix1.pyx"),
            "Return a matrix built from selected rows.",
        ),
        (
            "adjugate",
            "Matrix",
            root.join("sage/matrix/matrix2.pyx"),
            "Return the adjugate matrix.",
        ),
        (
            "from_integer",
            "Field",
            root.join("sage/rings/finite_rings/finite_field_base.pyx"),
            "Create a field element from an integer.",
        ),
        (
            "order",
            "Field",
            root.join("sage/rings/finite_rings/finite_field_base.pyx"),
            "Return the finite field order.",
        ),
        (
            "change_ring",
            "Matrix",
            root.join("sage/matrix/matrix0.pyx"),
            "Return this matrix over another ring.",
        ),
        (
            "base_ring",
            "Vector",
            root.join("sage/structure/element.pyx"),
            "Return the base ring of this element.",
        ),
    ] {
        let (line, character) = member_position(source, needle);
        let query =
            index.query_source_at(&consumer, source, QueryPosition { line, character }, None);
        assert_eq!(query.owner_type, None);
        assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
        assert!(query.definition.is_none());
        let expected_path = normalize_path(expected_path);
        assert!(
            query.definition_candidates.iter().any(|candidate| {
                candidate.definition.path == expected_path
                    && candidate.summary.as_deref() == Some(expected_doc)
            }),
            "missing expected helper-method candidate for {needle}: {:?}",
            query.definition_candidates
        );
    }

    fs::remove_dir_all(root).ok();
}
