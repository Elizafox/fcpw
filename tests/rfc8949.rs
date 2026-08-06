use fcpw::{Value, from_slice_value, to_vec_value, validate, validate_deterministic};
use proptest::prelude::*;
use serde::Deserialize;

const APPENDIX_A: &str = include_str!("fixtures/appendix_a.json");

#[derive(Deserialize)]
struct Vector {
    hex: String,
    roundtrip: bool,
}

fn hex(input: &str) -> Vec<u8> {
    assert!(input.len().is_multiple_of(2));
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => panic!("invalid hexadecimal test fixture"),
            };
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect()
}

#[test]
fn cbor_working_group_appendix_a_vectors_decode_and_roundtrip() {
    let vectors: Vec<Vector> = serde_json::from_str(APPENDIX_A).unwrap();
    assert_eq!(vectors.len(), 82, "fixture changed unexpectedly");

    for vector in vectors {
        let encoded = hex(&vector.hex);
        // RFC 8949 removed RFC 7049's simple(24) example: an argument below
        // 32 after additional-information value 24 is not well formed.
        if vector.hex == "f818" {
            assert!(validate(&encoded).is_err());
            continue;
        }
        validate(&encoded).unwrap_or_else(|error| panic!("{}: {error}", vector.hex));
        let value =
            from_slice_value(&encoded).unwrap_or_else(|error| panic!("{}: {error}", vector.hex));

        // A dynamic float is represented as f64, so normal Value encoding
        // intentionally emits binary64 rather than preserving its input width.
        if vector.roundtrip && !matches!(value, Value::Float(_)) {
            let reencoded = to_vec_value(&value).unwrap();
            assert_eq!(reencoded, encoded, "round trip differs for {}", vector.hex);
        }
    }
}

// RFC 8949 Appendix F.1: examples that are not well-formed CBOR.
#[test]
fn appendix_f_malformed_examples_are_rejected() {
    let malformed = [
        "1c", // reserved additional information
        "1d", "1e", "f800",   // simple value encoded with an invalid argument
        "5f00ff", // non-byte-string chunk
        "5f21ff", "5f4100",   // missing break
        "7f4100ff", // byte chunk in a text string
        "7f61ffff", // invalid UTF-8 text chunk
        "9f00",     // missing break
        "bf0000",   // missing break
        "bf00ff",   // map missing a value
        "ff",       // break outside an indefinite item
        "81ff",     // break in a definite array
        "a1ff00",   // break in a definite map
        "c0ff",     // break as tagged value
        "1f",       // indefinite integer
        "3f",       // indefinite negative integer
        "df",       // indefinite tag
    ];

    for encoded in malformed {
        assert!(
            validate(&hex(encoded)).is_err(),
            "accepted malformed {encoded}"
        );
    }
}

#[test]
fn rfc_preferred_serialization_examples_have_expected_determinism() {
    for preferred in ["00", "1818", "190100", "f93e00", "f97e00", "6161", "8100"] {
        validate_deterministic(&hex(preferred)).unwrap();
    }
    for non_preferred in ["1800", "190018", "fa3fc00000", "f97e01", "780161", "9f00ff"] {
        assert!(validate_deterministic(&hex(non_preferred)).is_err());
    }
}

fn value_strategy() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        any::<u64>().prop_map(Value::Unsigned),
        any::<i64>().prop_filter_map("negative", |value| (value < 0)
            .then_some(Value::Negative(value as i128))),
        any::<bool>().prop_map(Value::Bool),
        any::<f64>()
            .prop_filter("NaN is not equal to itself", |value| !value.is_nan())
            .prop_map(Value::Float),
        proptest::collection::vec(any::<u8>(), 0..64).prop_map(Value::Bytes),
        ".{0,64}".prop_map(Value::Text),
        Just(Value::Null),
        Just(Value::Undefined),
    ];

    leaf.prop_recursive(4, 128, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            proptest::collection::vec((inner.clone(), inner.clone()), 0..8).prop_map(Value::Map),
            (any::<u64>(), inner).prop_map(|(tag, value)| Value::Tag(tag, Box::new(value))),
        ]
    })
}

proptest! {
    #[test]
    fn dynamic_values_roundtrip(value in value_strategy()) {
        let encoded = to_vec_value(&value).unwrap();
        validate(&encoded).unwrap();
        let decoded = from_slice_value(&encoded).unwrap();

        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn validation_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = validate(&bytes);
        let _ = validate_deterministic(&bytes);
        let _ = from_slice_value(&bytes);
    }
}
