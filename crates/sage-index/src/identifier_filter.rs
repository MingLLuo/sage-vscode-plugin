use super::*;

pub(super) const IDENTIFIER_FILTER_BYTES: usize = 8_192;
const IDENTIFIER_FILTER_HASHES: u64 = 6;

pub(super) fn identifier_filter_for_source(source: &str) -> Vec<u8> {
    let mut filter = vec![0; IDENTIFIER_FILTER_BYTES];
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'_' && !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len() && is_word_byte(bytes[index]) {
            index += 1;
        }
        insert_identifier(&mut filter, &bytes[start..index]);
    }
    filter
}

pub(super) fn identifier_filter_might_contain(filter: &[u8], name: &str) -> bool {
    if filter.len() != IDENTIFIER_FILTER_BYTES || name.is_empty() {
        return false;
    }
    identifier_bit_indexes(name.as_bytes())
        .into_iter()
        .all(|bit| filter[bit / 8] & (1 << (bit % 8)) != 0)
}

fn insert_identifier(filter: &mut [u8], identifier: &[u8]) {
    for bit in identifier_bit_indexes(identifier) {
        filter[bit / 8] |= 1 << (bit % 8);
    }
}

fn identifier_bit_indexes(identifier: &[u8]) -> [usize; IDENTIFIER_FILTER_HASHES as usize] {
    let first = fnv1a(identifier, 0xcbf29ce484222325);
    let second = fnv1a(identifier, 0x84222325cbf29ce4) | 1;
    let bit_count = IDENTIFIER_FILTER_BYTES * 8;
    std::array::from_fn(|index| {
        first
            .wrapping_add((index as u64).wrapping_mul(second))
            .wrapping_add((index as u64).wrapping_mul(index as u64)) as usize
            % bit_count
    })
}

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    bytes.iter().fold(seed, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_filter_has_no_false_negatives_for_source_identifiers() {
        let source = "from sage.all import Combinations\nvalue = Combinations([1, 2], 1)\n";
        let filter = identifier_filter_for_source(source);

        for name in ["from", "sage", "all", "import", "Combinations", "value"] {
            assert!(identifier_filter_might_contain(&filter, name), "{name}");
        }
        assert!(!identifier_filter_might_contain(
            &filter,
            "DefinitelyAbsent"
        ));
    }

    #[test]
    fn missing_identifier_filter_is_not_treated_as_authoritative() {
        assert!(!identifier_filter_might_contain(&[], "target"));
    }
}
