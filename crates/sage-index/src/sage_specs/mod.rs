mod aliases;
mod exports;
mod methods;
mod owner_modules;

pub(super) use aliases::SAGE_METHOD_ALIAS_SPECS;
pub(super) use exports::SAGE_EXPORT_MAP;
pub(super) use methods::SAGE_METHOD_SPECS;
pub(super) use owner_modules::SAGE_OWNER_METHOD_MODULES;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn assert_non_empty(value: &str, field: &str) {
        assert!(!value.trim().is_empty(), "{field} must not be empty");
    }

    #[test]
    fn export_specs_have_unique_keys_and_non_empty_fields() {
        let mut keys = BTreeSet::new();
        for target in SAGE_EXPORT_MAP {
            assert_non_empty(target.import_module, "export import_module");
            assert_non_empty(target.name, "export name");
            assert_non_empty(target.source_module, "export source_module");
            assert_non_empty(target.source_name, "export source_name");
            assert!(
                keys.insert((target.import_module, target.name)),
                "duplicate Sage export key: {}::{}",
                target.import_module,
                target.name,
            );
        }
    }

    #[test]
    fn owner_module_specs_have_unique_keys_and_non_empty_fields() {
        let mut keys = BTreeSet::new();
        for spec in SAGE_OWNER_METHOD_MODULES {
            assert_non_empty(spec.module, "owner module");
            assert!(
                keys.insert((spec.owner_type, spec.module)),
                "duplicate Sage owner-module key: {:?}::{}",
                spec.owner_type,
                spec.module,
            );
        }
    }

    #[test]
    fn method_specs_have_unique_keys_and_non_empty_fields() {
        let mut keys = BTreeSet::new();
        for spec in SAGE_METHOD_SPECS {
            assert_non_empty(spec.member, "method member");
            assert_non_empty(spec.module, "method module");
            assert!(
                keys.insert((spec.owner_type, spec.member)),
                "duplicate Sage method key: {:?}.{}",
                spec.owner_type,
                spec.member,
            );
        }
        for spec in SAGE_METHOD_ALIAS_SPECS {
            assert_non_empty(spec.member, "method alias member");
            assert_non_empty(spec.source_name, "method alias source_name");
            assert_non_empty(spec.module, "method alias module");
            assert!(
                keys.insert((spec.owner_type, spec.member)),
                "duplicate Sage method/alias key: {:?}.{}",
                spec.owner_type,
                spec.member,
            );
        }
    }
}
