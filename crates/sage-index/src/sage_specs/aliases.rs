use crate::{SageMethodAliasSpec, SageOwnerType};

pub(crate) const SAGE_METHOD_ALIAS_SPECS: &[SageMethodAliasSpec] = &[
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "random",
        source_name: "random_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "identity",
        source_name: "identity_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "column",
        source_name: "column_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "diagonal",
        source_name: "diagonal_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "zero",
        source_name: "zero_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "ones",
        source_name: "ones_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "block",
        source_name: "block_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::MatrixConstructor,
        member: "block_diagonal",
        source_name: "block_diagonal_matrix",
        module: "sage.matrix.special",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::EllipticCurve,
        member: "order",
        source_name: "cardinality",
        module: "sage.schemes.elliptic_curves.ell_finite_field",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "integer_representation",
        source_name: "to_integer",
        module: "sage.rings.finite_rings.element_base",
    },
    SageMethodAliasSpec {
        owner_type: SageOwnerType::FieldElement,
        member: "_integer_representation",
        source_name: "_integer_representation",
        module: "sage.rings.finite_rings.element_givaro",
    },
];
