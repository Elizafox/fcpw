#![cfg(feature = "serde")]

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    alpha: u64,
    beta: bool,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Choice {
    Unit,
    Tuple(u8, bool),
    Struct { value: u16 },
}

#[test]
fn typed_tags_round_trip_and_retain_nesting() {
    let value = fcpw::Tagged::with_tag(
        100,
        fcpw::Tagged::with_tag(
            200,
            Record {
                alpha: 7,
                beta: true,
            },
        ),
    );
    let bytes = fcpw::to_vec(&value).unwrap();
    assert_eq!(bytes[0], 0xd8);
    let decoded: fcpw::Tagged<fcpw::Tagged<Record>> = fcpw::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);

    let untagged = fcpw::Tagged::new(None, 9_u64);
    assert_eq!(
        fcpw::from_slice::<fcpw::Tagged<u64>>(&fcpw::to_vec(&untagged).unwrap()).unwrap(),
        untagged
    );
}

#[test]
fn packed_encoding_matches_serde_cbor() {
    let record = Record {
        alpha: 3,
        beta: true,
    };
    assert_eq!(
        fcpw::to_vec_packed(&record).unwrap(),
        serde_cbor::ser::to_vec_packed(&record).unwrap()
    );
    for choice in [
        Choice::Unit,
        Choice::Tuple(4, false),
        Choice::Struct { value: 500 },
    ] {
        let bytes = fcpw::to_vec_packed(&choice).unwrap();
        assert_eq!(bytes, serde_cbor::ser::to_vec_packed(&choice).unwrap());
        assert_eq!(fcpw::from_slice::<Choice>(&bytes).unwrap(), choice);
    }
}

#[test]
fn reader_deserializer_decodes_consecutive_items_and_offsets() {
    let mut bytes = fcpw::to_vec(&1_u64).unwrap();
    bytes.extend(
        fcpw::to_vec(&Record {
            alpha: 8,
            beta: false,
        })
        .unwrap(),
    );
    let mut reader = fcpw::ReaderDeserializer::new(bytes.as_slice());
    assert_eq!(reader.deserialize_next::<u64>().unwrap(), Some(1));
    assert_eq!(reader.byte_offset(), 1);
    assert_eq!(
        reader.deserialize_next::<Record>().unwrap(),
        Some(Record {
            alpha: 8,
            beta: false
        })
    );
    assert_eq!(reader.byte_offset(), bytes.len());
    assert_eq!(reader.deserialize_next::<Record>().unwrap(), None);
}

#[test]
fn reader_deserializer_handles_many_buffered_items_without_losing_offsets() {
    let bytes: Vec<u8> = (0..20_000).map(|value| (value % 24) as u8).collect();
    let mut reader = fcpw::ReaderDeserializer::new(bytes.as_slice());

    for (index, expected) in (0..20_000).map(|value| (value % 24) as u64).enumerate() {
        assert_eq!(reader.deserialize_next::<u64>().unwrap(), Some(expected));
        assert_eq!(reader.byte_offset(), index + 1);
    }
    assert_eq!(reader.deserialize_next::<u64>().unwrap(), None);
    assert_eq!(reader.byte_offset(), bytes.len());
}

#[test]
fn compatibility_alias_and_self_describe_tag() {
    let mut via_alias = Vec::new();
    fcpw::into_writer(
        &Record {
            alpha: 1,
            beta: true,
        },
        &mut via_alias,
    )
    .unwrap();
    assert_eq!(
        via_alias,
        fcpw::to_vec(&Record {
            alpha: 1,
            beta: true
        })
        .unwrap()
    );

    let mut prefix = Vec::new();
    fcpw::write_self_describe(&mut prefix).unwrap();
    assert_eq!(prefix, [0xd9, 0xd9, 0xf7]);
}
