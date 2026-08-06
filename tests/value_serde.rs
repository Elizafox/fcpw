#![cfg(feature = "serde")]

use fcpw::{Value, from_slice, to_vec, value};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    name: String,
    bytes: serde_bytes::ByteBuf,
    wide: u128,
    optional: Option<u32>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Choice {
    Unit,
    Tuple(u32, String),
    Struct { enabled: bool },
}

#[test]
fn value_works_with_one_shot_serde_apis_and_preserves_cbor_extensions() {
    let values = [
        Value::Unsigned(42),
        Value::Negative(-2),
        Value::Bytes(vec![1, 2]),
        Value::Text("hello".into()),
        Value::Array(vec![Value::Bool(true), Value::Null]),
        Value::Map(vec![(Value::Text("k".into()), Value::Float(1.5))]),
        Value::Tag(100, Box::new(Value::Undefined)),
        Value::Simple(32),
        Value::Bool(false),
        Value::Null,
        Value::Undefined,
        Value::Float(-0.0),
    ];
    for expected in values {
        let bytes = to_vec(&expected).unwrap();
        assert_eq!(from_slice::<Value>(&bytes).unwrap(), expected);
    }
}

#[test]
fn value_conversion_round_trips_structs_enums_and_large_integers() {
    let record = Record {
        name: "record".into(),
        bytes: vec![0, 1, 2].into(),
        wide: u64::MAX as u128 + 1,
        optional: Some(7),
    };
    let dynamic = value::to_value(&record).unwrap();
    assert_eq!(value::from_value::<Record>(dynamic).unwrap(), record);

    for choice in [
        Choice::Unit,
        Choice::Tuple(4, "four".into()),
        Choice::Struct { enabled: true },
    ] {
        let dynamic = value::to_value(&choice).unwrap();
        assert_eq!(value::from_value::<Choice>(dynamic).unwrap(), choice);
    }
}

#[test]
fn null_and_undefined_are_distinct_values_but_both_deserialize_as_none() {
    assert_ne!(Value::Null, Value::Undefined);
    assert_eq!(value::from_value::<Option<u8>>(Value::Null).unwrap(), None);
    assert_eq!(
        value::from_value::<Option<u8>>(Value::Undefined).unwrap(),
        None
    );
}

#[test]
fn from_value_preserves_cbor_extensions_for_value_and_typed_tags() {
    let dynamic = Value::Tag(100, Box::new(Value::Simple(32)));
    assert_eq!(
        value::from_value::<Value>(dynamic.clone()).unwrap(),
        dynamic
    );

    let tagged = value::from_value::<fcpw::Tagged<Record>>(
        value::to_value(&fcpw::Tagged::with_tag(
            100,
            Record {
                name: "tagged".into(),
                bytes: vec![1, 2, 3].into(),
                wide: 7,
                optional: None,
            },
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(tagged.tag, Some(100));
    assert_eq!(tagged.value.name, "tagged");
}
