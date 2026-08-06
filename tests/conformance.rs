use fcpw::{
    BorrowedValue, DecodeOptions, Encoder, ErrorKind, Event, Parser, SequenceDecoder, SliceDecoder,
    Validation, validate, validate_deterministic,
};
use std::borrow::Cow;

#[test]
fn integer_boundaries_and_negative_values() {
    for (bytes, expected) in [
        (&[0x00][..], 0),
        (&[0x17], 23),
        (&[0x18, 0x18], 24),
        (&[0x19, 1, 0], 256),
        (&[0x1a, 0, 1, 0, 0], 65_536),
        (&[0x20], -1),
        (&[0x38, 0x63], -100),
    ] {
        let mut decoder = SliceDecoder::new(bytes);
        assert_eq!(decoder.integer().unwrap(), expected);
        decoder.finish().unwrap();
    }
}

#[test]
fn parser_decodes_all_float_widths() {
    let cases = [
        (&[0xf9, 0x3e, 0x00][..], 1.5),
        (&[0xfa, 0x3f, 0xc0, 0, 0], 1.5),
        (&[0xfb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0], 1.5),
    ];
    for (bytes, expected) in cases {
        assert_eq!(
            Parser::new(bytes).next().unwrap().unwrap(),
            Event::Float(expected)
        );
    }
}

#[test]
fn validates_indefinite_containers_and_strings() {
    validate(&[0x9f, 1, 2, 0xff]).unwrap();
    validate(&[0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff]).unwrap();
    validate(&[0xbf, 1, 2, 0xff]).unwrap();
    assert_eq!(
        validate(&[0xbf, 1, 0xff]).unwrap_err().kind(),
        ErrorKind::UnexpectedBreak
    );
}

#[test]
fn rejects_invalid_chunks_breaks_and_utf8() {
    assert_eq!(
        validate(&[0xff]).unwrap_err().kind(),
        ErrorKind::UnexpectedBreak
    );
    assert_eq!(
        validate(&[0x7f, 0x41, 0, 0xff]).unwrap_err().kind(),
        ErrorKind::UnexpectedType
    );
    assert_eq!(
        validate(&[0x62, 0xff, 0xff]).unwrap_err().kind(),
        ErrorKind::InvalidUtf8
    );
}

#[test]
fn structural_skip_fast_path_covers_complete_one_byte_items() {
    let items = [
        0x00, 0x17, 0x20, 0x37, 0x40, 0x60, 0x80, 0xa0, 0xe0, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7,
    ];
    for item in items {
        validate(&[item]).unwrap();
        validate_deterministic(&[item]).unwrap();
    }

    let raw: Vec<_> = SequenceDecoder::new(&items)
        .map(|item| item.unwrap().as_bytes()[0])
        .collect();
    assert_eq!(raw, items);

    for item in [0x18, 0x38, 0x58, 0x78, 0x98, 0xb8, 0xd7, 0xf8] {
        assert_eq!(validate(&[item]).unwrap_err().kind(), ErrorKind::Eof);
    }
    assert_eq!(
        validate(&[0xff]).unwrap_err().kind(),
        ErrorKind::UnexpectedBreak
    );
}

#[test]
fn deterministic_validation_checks_widths_lengths_and_order() {
    validate_deterministic(&[0x18, 0x17]).unwrap_err();
    validate_deterministic(&[0x9f, 1, 0xff]).unwrap_err();
    validate_deterministic(&[0xa2, 0x61, b'b', 0, 0x61, b'a', 0]).unwrap_err();
    validate_deterministic(&[0xa2, 0x61, b'a', 0, 0x61, b'b', 0]).unwrap();
    validate_deterministic(&[0xfa, 0x3f, 0xc0, 0, 0]).unwrap_err();
    validate_deterministic(&[0xf9, 0x3e, 0]).unwrap();
    for canonical_half in [
        [0xf9, 0x00, 0x00],
        [0xf9, 0x80, 0x00],
        [0xf9, 0x7c, 0x00],
        [0xf9, 0xfc, 0x00],
        [0xf9, 0x7e, 0x00],
    ] {
        validate_deterministic(&canonical_half).unwrap();
    }
    validate_deterministic(&[0xf9, 0x7e, 0x01]).unwrap_err();
}

#[test]
fn depth_and_collection_limits_are_enforced() {
    let mut options = DecodeOptions {
        max_depth: 1,
        ..DecodeOptions::default()
    };
    let mut decoder = SliceDecoder::with_options(&[0x81, 0x81, 0], options);
    assert_eq!(decoder.skip().unwrap_err().kind(), ErrorKind::DepthLimit);

    options.max_collection_len = 1;
    let mut decoder = SliceDecoder::with_options(&[0x82, 0, 0], options);
    assert_eq!(
        decoder.skip().unwrap_err().kind(),
        ErrorKind::CollectionLimit
    );
}

#[test]
fn sequence_preserves_raw_items_and_reports_index() {
    let bytes = [1, 0x62, b'o', b'k', 0x81, 2];
    let values: Vec<_> = SequenceDecoder::new(&bytes)
        .map(|x| x.unwrap().as_bytes().to_vec())
        .collect();
    assert_eq!(values, vec![vec![1], vec![0x62, b'o', b'k'], vec![0x81, 2]]);

    let mut sequence = SequenceDecoder::new(&[1, 0x1a]);
    sequence.next().unwrap().unwrap();
    assert_eq!(sequence.next().unwrap().unwrap_err().item_index(), Some(1));
}

#[test]
fn encoder_uses_preferred_integer_widths() {
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes);
    encoder.unsigned(23).unwrap();
    encoder.unsigned(24).unwrap();
    encoder.integer(-100).unwrap();
    assert_eq!(bytes, [0x17, 0x18, 0x18, 0x38, 99]);
}

#[test]
fn encoder_separates_source_and_preferred_float_widths() {
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes);
    encoder.f32(1.5).unwrap();
    encoder.f64(1.5).unwrap();
    assert_eq!(&bytes[..5], &[0xfa, 0x3f, 0xc0, 0x00, 0x00]);
    assert_eq!(bytes[5], 0xfb);

    let mut preferred = Vec::new();
    let mut encoder = Encoder::new(&mut preferred);
    encoder.f32_preferred(1.5).unwrap();
    encoder.f64_preferred(1.5).unwrap();
    encoder.f64_preferred(100_000.0).unwrap();
    encoder.f64_preferred(1.1).unwrap();
    assert_eq!(&preferred[..3], &[0xf9, 0x3e, 0x00]);
    assert_eq!(&preferred[3..6], &[0xf9, 0x3e, 0x00]);
    assert_eq!(preferred[6], 0xfa);
    assert_eq!(preferred[11], 0xfb);
}

#[test]
fn borrowed_dynamic_values_retain_slices() {
    let input = [0x82, 0x42, 1, 2, 0x62, b'o', b'k'];
    let value = BorrowedValue::decode(&input).unwrap();
    match value {
        BorrowedValue::Array(values) => {
            assert!(matches!(&values[0], BorrowedValue::Bytes(v) if matches!(v, Cow::Borrowed(_))));
            assert!(matches!(&values[1], BorrowedValue::Text(v) if matches!(v, Cow::Borrowed(_))));
        }
        _ => panic!("expected array"),
    }
}

#[test]
fn indefinite_dynamic_strings_are_joined() {
    let bytes = [0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff];
    assert_eq!(
        BorrowedValue::decode(&bytes).unwrap(),
        BorrowedValue::Text(Cow::Owned(String::from("hi!")))
    );
}

#[test]
fn dynamic_values_decode_nested_variations_directly() {
    let bytes = [
        0xbf, 0x01, 0x82, 0x21, 0xd8, 0x64, 0xf9, 0x3e, 0x00, 0x61, b'k', 0x5f, 0x42, 1, 2, 0x41,
        3, 0xff, 0xff,
    ];
    let expected = BorrowedValue::Map(vec![
        (
            BorrowedValue::Unsigned(1),
            BorrowedValue::Array(vec![
                BorrowedValue::Negative(-2),
                BorrowedValue::Tag(100, Box::new(BorrowedValue::Float(1.5))),
            ]),
        ),
        (
            BorrowedValue::Text(Cow::Borrowed("k")),
            BorrowedValue::Bytes(Cow::Owned(vec![1, 2, 3])),
        ),
    ]);
    assert_eq!(BorrowedValue::decode(&bytes).unwrap(), expected);
    assert_eq!(
        fcpw::from_slice_value(&bytes).unwrap(),
        expected.clone().into()
    );
    assert_eq!(
        BorrowedValue::decode(&[0xf8, 32]).unwrap(),
        BorrowedValue::Simple(32)
    );
}

#[test]
fn dynamic_values_reject_malformed_structure_at_the_cursor() {
    for (bytes, kind, offset) in [
        (&[0xbf, 1, 0xff][..], ErrorKind::UnexpectedBreak, 2),
        (&[0x5f, 0x5f, 0xff, 0xff], ErrorKind::UnexpectedType, 1),
        (&[0x1f], ErrorKind::InvalidAdditionalInfo, 0),
        (&[1, 2], ErrorKind::TrailingData, 1),
    ] {
        let borrowed_error = BorrowedValue::decode(bytes).unwrap_err();
        assert_eq!(borrowed_error.kind(), kind);
        assert_eq!(borrowed_error.offset(), offset);

        let owned_error = fcpw::from_slice_value(bytes).unwrap_err();
        assert_eq!(owned_error.kind(), kind);
        assert_eq!(owned_error.offset(), offset);
    }
}

#[test]
fn strict_option_remains_forward_compatible_with_unknown_tags() {
    let options = DecodeOptions {
        validation: Validation::Strict,
        ..DecodeOptions::default()
    };
    let mut decoder = SliceDecoder::with_options(&[0xd9, 0x03, 0xe7, 1], options);
    decoder.skip().unwrap();
}

#[cfg(feature = "serde")]
#[test]
fn serde_struct_and_borrowed_field_round_trip() {
    use serde::{Deserialize, Serialize};
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Record<'a> {
        id: u64,
        name: &'a str,
        values: Vec<i32>,
        active: bool,
    }
    let value = Record {
        id: 42,
        name: "Ada",
        values: vec![-1, 2, 3],
        active: true,
    };
    let bytes = fcpw::to_vec(&value).unwrap();
    assert_eq!(fcpw::serialized_size(&value).unwrap(), bytes.len());
    let decoded: Record<'_> = fcpw::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);
}

#[cfg(feature = "serde")]
#[test]
fn serde_large_integers_use_bignum_tags() {
    let unsigned = u64::MAX as u128 + 1;
    let bytes = fcpw::to_vec(&unsigned).unwrap();
    assert_eq!(bytes[0], 0xc2);
    assert_eq!(fcpw::from_slice::<u128>(&bytes).unwrap(), unsigned);

    let signed = i128::MIN;
    let bytes = fcpw::to_vec(&signed).unwrap();
    assert_eq!(bytes[0], 0xc3);
    assert_eq!(fcpw::from_slice::<i128>(&bytes).unwrap(), signed);
}

#[cfg(feature = "serde")]
#[test]
fn optimized_native_integer_decoder_preserves_boundaries_and_errors() {
    let signed16 = [i16::MIN, -257, -256, -25, -24, -1, 0, 23, 24, i16::MAX];
    let encoded = fcpw::to_vec(&signed16).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<i16>>(&encoded).unwrap(), signed16);

    let signed32 = [
        i32::MIN,
        -65_537,
        -65_536,
        -257,
        -256,
        -25,
        -24,
        -1,
        0,
        23,
        24,
        i32::MAX,
    ];
    let encoded = fcpw::to_vec(&signed32).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<i32>>(&encoded).unwrap(), signed32);

    let unsigned16 = [0, 23, 24, 255, 256, u16::MAX];
    let encoded = fcpw::to_vec(&unsigned16).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<u16>>(&encoded).unwrap(), unsigned16);

    let unsigned32 = [0, 23, 24, 255, 256, 65_535, 65_536, u32::MAX];
    let encoded = fcpw::to_vec(&unsigned32).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<u32>>(&encoded).unwrap(), unsigned32);

    let unsigned = [
        0,
        23,
        24,
        255,
        256,
        65_535,
        65_536,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        u64::MAX,
    ];
    let encoded = fcpw::to_vec(&unsigned).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<u64>>(&encoded).unwrap(), unsigned);

    let signed = [
        i64::MIN,
        i32::MIN as i64,
        -65_537,
        -65_536,
        -257,
        -256,
        -25,
        -24,
        -1,
        0,
        23,
        24,
        i64::MAX,
    ];
    let encoded = fcpw::to_vec(&signed).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<i64>>(&encoded).unwrap(), signed);

    assert_eq!(
        fcpw::from_slice::<i16>(&[0x19, 0x80, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<i16>(&[0x39, 0x80, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<i32>(&[0x1a, 0x80, 0, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<i32>(&[0x3a, 0x80, 0, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<u16>(&[0x1a, 0, 1, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<u32>(&[0x1b, 0, 0, 0, 1, 0, 0, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<i64>(&[0x1b, 0x80, 0, 0, 0, 0, 0, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<i64>(&[0x3b, 0x80, 0, 0, 0, 0, 0, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );

    // Keep the existing signed bignum fallback outside the native fast path.
    assert_eq!(fcpw::from_slice::<i64>(&[0xc2, 0x41, 1]).unwrap(), 1);
}

#[cfg(feature = "serde")]
#[test]
fn optimized_forwarded_types_preserve_fallback_semantics() {
    assert!(!fcpw::from_slice::<bool>(&[0xf4]).unwrap());
    assert!(fcpw::from_slice::<bool>(&[0xf5]).unwrap());
    assert!(fcpw::from_slice::<bool>(&[0xc0, 0xf5]).unwrap());

    let signed = [i8::MIN, -25, -24, -1, 0, 23, 24, i8::MAX];
    let encoded = fcpw::to_vec(&signed).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<i8>>(&encoded).unwrap(), signed);
    let unsigned = [0, 23, 24, 255];
    let encoded = fcpw::to_vec(&unsigned).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<u8>>(&encoded).unwrap(), unsigned);
    assert_eq!(
        fcpw::from_slice::<i8>(&[0x18, 0x80]).unwrap_err().kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<i8>(&[0x38, 0x80]).unwrap_err().kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice::<u8>(&[0x19, 1, 0]).unwrap_err().kind(),
        ErrorKind::IntegerOverflow
    );

    assert_eq!(fcpw::from_slice::<Option<u8>>(&[0xf6]).unwrap(), None);
    assert_eq!(fcpw::from_slice::<Option<u8>>(&[0xf7]).unwrap(), None);
    assert_eq!(fcpw::from_slice::<Option<u8>>(&[7]).unwrap(), Some(7));
    assert_eq!(
        fcpw::from_slice::<Vec<Option<u8>>>(&[0x9f, 0xf6, 7, 0xff]).unwrap(),
        [None, Some(7)]
    );
    assert_eq!(
        fcpw::from_slice::<Option<()>>(&[0xc0, 0xf6]).unwrap(),
        Some(())
    );

    assert_eq!(
        fcpw::from_slice::<&str>(&[0xc0, 0x62, b'o', b'k']).unwrap(),
        "ok"
    );
    assert_eq!(
        fcpw::from_slice::<String>(&[0x7f, 0x61, b'o', 0x61, b'k', 0xff]).unwrap(),
        "ok"
    );
    for character in ['a', 'ß', '水', '🦀'] {
        let encoded = fcpw::to_vec(&character).unwrap();
        assert_eq!(fcpw::from_slice::<char>(&encoded).unwrap(), character);
    }
    assert_eq!(
        fcpw::from_slice::<serde_bytes::ByteBuf>(&[0x5f, 0x42, 1, 2, 0x41, 3, 0xff]).unwrap(),
        serde_bytes::ByteBuf::from(vec![1, 2, 3])
    );

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct KnownOnly {
        known: u8,
    }
    let valid_unknown = [
        0xa2, 0x65, b'k', b'n', b'o', b'w', b'n', 1, 0x65, b'e', b'x', b't', b'r', b'a', 0x82, 2, 3,
    ];
    assert_eq!(
        fcpw::from_slice::<KnownOnly>(&valid_unknown).unwrap(),
        KnownOnly { known: 1 }
    );
    let invalid_unknown = [
        0xa2, 0x65, b'k', b'n', b'o', b'w', b'n', 1, 0x65, b'e', b'x', b't', b'r', b'a', 0x61, 0xff,
    ];
    assert_eq!(
        fcpw::from_slice::<KnownOnly>(&invalid_unknown)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidUtf8
    );
}

#[cfg(feature = "serde")]
#[test]
fn bulk_boolean_array_decoder_preserves_structure_and_types() {
    let values: Vec<bool> = (0..4096).map(|value| value % 3 != 0).collect();
    let bytes = fcpw::to_vec(&values).unwrap();
    assert_eq!(fcpw::from_slice_bool_array(&bytes).unwrap(), values);
    assert_eq!(
        fcpw::from_slice_bool_array(&[0x9f, 0xf4, 0xf5, 0xf4, 0xff]).unwrap(),
        [false, true, false]
    );
    assert_eq!(
        fcpw::from_slice_bool_array(&[0x81, 0x01])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedType
    );
    assert_eq!(
        fcpw::from_slice_bool_array(&[0x82, 0xf4])
            .unwrap_err()
            .kind(),
        ErrorKind::Eof
    );
}

#[cfg(feature = "serde")]
#[test]
fn bulk_u8_array_decoder_preserves_widths_and_overflow() {
    let values: Vec<u8> = (0..=u8::MAX).cycle().take(4096).collect();
    let bytes = fcpw::to_vec(&values).unwrap();
    assert_eq!(fcpw::from_slice_u8_array(&bytes).unwrap(), values);
    assert_eq!(
        fcpw::from_slice_u8_array(&[0x9f, 0x00, 0x18, 0xff, 0x19, 0x00, 0x17, 0xff]).unwrap(),
        [0, 255, 23]
    );
    assert_eq!(
        fcpw::from_slice_u8_array(&[0x81, 0x19, 0x01, 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice_u8_array(&[0x81, 0x20]).unwrap_err().kind(),
        ErrorKind::UnexpectedType
    );
}

#[cfg(feature = "serde")]
#[test]
fn known_initial_string_and_bytes_paths_preserve_header_semantics() {
    let text_24 = "a".repeat(24);
    let text_256 = "b".repeat(256);
    let mut text = vec![0x83, 0x60, 0x78, 24];
    text.extend_from_slice(text_24.as_bytes());
    text.extend_from_slice(&[0x79, 0x01, 0x00]);
    text.extend_from_slice(text_256.as_bytes());
    assert_eq!(
        fcpw::from_slice::<Vec<&str>>(&text).unwrap(),
        ["", text_24.as_str(), text_256.as_str()]
    );

    let bytes_24 = vec![0x5a; 24];
    let bytes_256 = vec![0xa5; 256];
    let mut bytes = vec![0x83, 0x40, 0x58, 24];
    bytes.extend_from_slice(&bytes_24);
    bytes.extend_from_slice(&[0x59, 0x01, 0x00]);
    bytes.extend_from_slice(&bytes_256);
    assert_eq!(
        fcpw::from_slice::<Vec<serde_bytes::ByteBuf>>(&bytes).unwrap(),
        [
            serde_bytes::ByteBuf::new(),
            serde_bytes::ByteBuf::from(bytes_24),
            serde_bytes::ByteBuf::from(bytes_256),
        ]
    );

    assert_eq!(
        fcpw::from_slice::<&str>(&[0x78]).unwrap_err().kind(),
        ErrorKind::Eof
    );
    assert_eq!(
        fcpw::from_slice::<&str>(&[0x61, 0xff]).unwrap_err().kind(),
        ErrorKind::InvalidUtf8
    );
    assert_eq!(
        fcpw::from_slice::<serde_bytes::ByteBuf>(&[0x59, 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::Eof
    );
}

#[cfg(feature = "serde")]
#[test]
fn ordinary_typed_decoder_consumes_indefinite_boundaries_without_events() {
    assert_eq!(
        fcpw::from_slice::<Vec<Vec<u8>>>(&[0x9f, 0x9f, 1, 2, 0xff, 0x80, 0xff]).unwrap(),
        [vec![1, 2], vec![]]
    );
    assert_eq!(
        fcpw::from_slice::<Vec<u8>>(&[0x9f, 0xc0, 1, 2, 0xff]).unwrap(),
        [1, 2]
    );

    #[derive(Debug, serde::Deserialize, PartialEq)]
    enum Choice {
        Number(u8),
    }
    let indefinite = [
        0xbf, 0x66, b'N', b'u', b'm', b'b', b'e', b'r', 0x18, 42, 0xff,
    ];
    assert_eq!(
        fcpw::from_slice::<Choice>(&indefinite).unwrap(),
        Choice::Number(42)
    );
}

#[cfg(feature = "serde")]
#[test]
fn definite_and_indefinite_map_access_preserve_semantics() {
    use std::collections::BTreeMap;

    let expected = BTreeMap::from([(String::from("a"), 1u8), (String::from("b"), 2)]);
    let definite = fcpw::to_vec(&expected).unwrap();
    assert_eq!(
        fcpw::from_slice::<BTreeMap<String, u8>>(&definite).unwrap(),
        expected
    );

    let indefinite = [0xbf, 0x61, b'a', 1, 0x61, b'b', 2, 0xff];
    assert_eq!(
        fcpw::from_slice::<BTreeMap<String, u8>>(&indefinite).unwrap(),
        expected
    );

    assert_eq!(
        fcpw::from_slice::<BTreeMap<String, u8>>(&[0xbf, 0x61, b'a', 0xff])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedBreak
    );
}

#[cfg(feature = "serde")]
#[test]
fn stateful_deserializer_tracks_consecutive_items_and_trailing_data() {
    use serde::Deserialize;

    let input = [0x01, 0x62, b'h', b'i'];
    let mut deserializer = fcpw::Deserializer::from_slice(&input);

    assert_eq!(u8::deserialize(&mut deserializer).unwrap(), 1);
    assert_eq!(deserializer.byte_offset(), 1);
    assert_eq!(deserializer.remaining(), &input[1..]);
    let error = deserializer.end().unwrap_err();
    assert_eq!(error.kind(), fcpw::ErrorKind::TrailingData);
    assert_eq!(error.offset(), 1);

    assert_eq!(<&str>::deserialize(&mut deserializer).unwrap(), "hi");
    assert_eq!(deserializer.byte_offset(), input.len());
    assert!(deserializer.remaining().is_empty());
    deserializer.end().unwrap();
}

#[cfg(feature = "serde")]
#[test]
fn stateful_deserializer_preserves_options_and_error_offsets() {
    use serde::Deserialize;

    let options = fcpw::DecodeOptions {
        max_collection_len: 1,
        ..fcpw::DecodeOptions::default()
    };
    let mut deserializer =
        fcpw::Deserializer::from_slice_with_options(&[0x82, 0x00, 0x01], options);
    let error = Vec::<u8>::deserialize(&mut deserializer).unwrap_err();
    assert_eq!(error.kind(), fcpw::ErrorKind::CollectionLimit);
    assert_eq!(error.offset(), 1);

    let mut truncated = fcpw::Deserializer::from_slice(&[0x19, 0x01]);
    let error = u16::deserialize(&mut truncated).unwrap_err();
    assert_eq!(error.kind(), fcpw::ErrorKind::Eof);
    assert_eq!(error.offset(), 1);
}

#[cfg(feature = "serde")]
#[test]
fn direct_serde_decoder_enforces_limits_and_handles_enums() {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Choice {
        Unit,
        Number(u64),
        Pair(u8, u8),
        Named { enabled: bool },
    }

    for value in [
        Choice::Unit,
        Choice::Number(42),
        Choice::Pair(1, 2),
        Choice::Named { enabled: true },
    ] {
        let bytes = fcpw::to_vec(&value).unwrap();
        assert_eq!(fcpw::from_slice::<Choice>(&bytes).unwrap(), value);
    }

    let options = DecodeOptions {
        max_depth: 1,
        ..DecodeOptions::default()
    };
    assert_eq!(
        fcpw::from_slice_with_options::<Vec<Vec<u8>>>(&[0x81, 0x81, 0], options)
            .unwrap_err()
            .kind(),
        ErrorKind::DepthLimit
    );

    let options = DecodeOptions {
        max_collection_len: 1,
        ..DecodeOptions::default()
    };
    assert_eq!(
        fcpw::from_slice_with_options::<Vec<u8>>(&[0x82, 0, 0], options)
            .unwrap_err()
            .kind(),
        ErrorKind::CollectionLimit
    );
}

#[cfg(feature = "serde")]
#[test]
fn typed_deterministic_decode_validates_inline() {
    use std::collections::BTreeMap;

    let deterministic = DecodeOptions {
        validation: Validation::Deterministic,
        ..DecodeOptions::default()
    };

    for bytes in [&[0x18, 0x00][..], &[0x19, 0x00, 0x18]] {
        assert_eq!(
            fcpw::from_slice_with_options::<u8>(bytes, deterministic)
                .unwrap_err()
                .kind(),
            ErrorKind::NonDeterministic
        );
    }
    assert_eq!(
        fcpw::from_slice_with_options::<String>(&[0x78, 0x01, b'a'], deterministic)
            .unwrap_err()
            .kind(),
        ErrorKind::NonDeterministic
    );
    assert_eq!(
        fcpw::from_slice_with_options::<Vec<u8>>(&[0x9f, 1, 0xff], deterministic)
            .unwrap_err()
            .kind(),
        ErrorKind::NonDeterministic
    );
    assert_eq!(
        fcpw::from_slice_with_options::<BTreeMap<String, u8>>(
            &[0xa2, 0x61, b'b', 0, 0x61, b'a', 0],
            deterministic,
        )
        .unwrap_err()
        .kind(),
        ErrorKind::NonDeterministic
    );
    assert_eq!(
        fcpw::from_slice_with_options::<f64>(&[0xfa, 0x3f, 0xc0, 0, 0], deterministic)
            .unwrap_err()
            .kind(),
        ErrorKind::NonDeterministic
    );

    for encoded in [
        [0xf9, 0x00, 0x00],
        [0xf9, 0x80, 0x00],
        [0xf9, 0x7c, 0x00],
        [0xf9, 0xfc, 0x00],
        [0xf9, 0x7e, 0x00],
    ] {
        fcpw::from_slice_with_options::<f64>(&encoded, deterministic).unwrap();
    }

    let strict = DecodeOptions {
        validation: Validation::Strict,
        ..DecodeOptions::default()
    };
    assert_eq!(
        fcpw::from_slice_with_options::<u8>(&[0x18, 0x00], strict).unwrap(),
        0
    );
}

#[cfg(feature = "serde")]
#[test]
fn unknown_length_sequences_stream_as_indefinite_cbor() {
    use serde::{Serialize, Serializer, ser::SerializeSeq};

    struct UnknownLength;
    impl Serialize for UnknownLength {
        fn serialize<S: Serializer>(&self, serializer: S) -> core::result::Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(None)?;
            sequence.serialize_element(&1u8)?;
            sequence.serialize_element(&2u8)?;
            sequence.end()
        }
    }

    let bytes = fcpw::to_vec(&UnknownLength).unwrap();
    assert_eq!(bytes, [0x9f, 1, 2, 0xff]);
    assert_eq!(fcpw::from_slice::<Vec<u8>>(&bytes).unwrap(), [1, 2]);
}

#[cfg(feature = "serde")]
#[test]
fn optimized_integer_array_encoder_matches_scalar_semantics() {
    let values: Vec<i32> = (0..4096).map(|value| value - 2048).collect();
    let bytes = fcpw::to_vec(&values).unwrap();
    validate(&bytes).unwrap();
    assert_eq!(fcpw::from_slice::<Vec<i32>>(&bytes).unwrap(), values);
    assert_eq!(fcpw::from_slice_i32_array(&bytes).unwrap(), values);

    assert_eq!(
        fcpw::from_slice_i32_array(&[0x9f, 0x00, 0x38, 0x18, 0x1a, 0x7f, 0xff, 0xff, 0xff, 0xff])
            .unwrap(),
        [0, -25, i32::MAX]
    );
    assert_eq!(
        fcpw::from_slice_i32_array(&[0x81, 0x1a, 0x80, 0x00, 0x00, 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        fcpw::from_slice_i32_array(&[0x81, 0xf4])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedType
    );

    let values16 = [i16::MIN, -257, -1, 0, 256, i16::MAX];
    let bytes16 = fcpw::to_vec(&values16).unwrap();
    assert_eq!(fcpw::from_slice_i16_array(&bytes16).unwrap(), values16);
    assert_eq!(
        fcpw::from_slice_i16_array(&[0x9f, 0x00, 0x39, 0x7f, 0xff, 0xff]).unwrap(),
        [0, i16::MIN]
    );
    assert_eq!(
        fcpw::from_slice_i16_array(&[0x81, 0x19, 0x80, 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );

    let values8 = [i8::MIN, -25, -1, 0, 24, i8::MAX];
    let bytes8 = fcpw::to_vec(&values8).unwrap();
    assert_eq!(fcpw::from_slice_i8_array(&bytes8).unwrap(), values8);
    assert_eq!(
        fcpw::from_slice_i8_array(&[0x9f, 0x00, 0x38, 0x7f, 0xff]).unwrap(),
        [0, i8::MIN]
    );
    assert_eq!(
        fcpw::from_slice_i8_array(&[0x81, 0x18, 0x80])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );

    let values64 = [
        i64::MIN,
        i32::MIN as i64 - 1,
        -1,
        0,
        i32::MAX as i64 + 1,
        i64::MAX,
    ];
    let bytes64 = fcpw::to_vec(&values64).unwrap();
    assert_eq!(fcpw::from_slice_i64_array(&bytes64).unwrap(), values64);
    assert_eq!(
        fcpw::from_slice_i64_array(&[
            0x9f, 0x00, 0x3b, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ])
        .unwrap(),
        [0, i64::MIN]
    );
    assert_eq!(
        fcpw::from_slice_i64_array(&[0x81, 0x1b, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );

    let unsigned = [
        0,
        23,
        24,
        u16::MAX as u64 + 1,
        u32::MAX as u64 + 1,
        u64::MAX,
    ];
    let unsigned_bytes = fcpw::to_vec(&unsigned).unwrap();
    assert_eq!(
        fcpw::from_slice_u64_array(&unsigned_bytes).unwrap(),
        unsigned
    );
    assert_eq!(
        fcpw::from_slice_u64_array(&[
            0x9f, 0x00, 0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
        ])
        .unwrap(),
        [0, u64::MAX]
    );
    assert_eq!(
        fcpw::from_slice_u64_array(&[0x81, 0x20])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedType
    );

    let unsigned32 = [0, 23, 24, u16::MAX as u32 + 1, u32::MAX];
    let unsigned32_bytes = fcpw::to_vec(&unsigned32).unwrap();
    assert_eq!(
        fcpw::from_slice_u32_array(&unsigned32_bytes).unwrap(),
        unsigned32
    );
    assert_eq!(
        fcpw::from_slice_u32_array(&[0x9f, 0x00, 0x1a, 0xff, 0xff, 0xff, 0xff, 0xff]).unwrap(),
        [0, u32::MAX]
    );
    assert_eq!(
        fcpw::from_slice_u32_array(&[0x81, 0x1b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );

    let unsigned16 = [0, 23, 24, u8::MAX as u16 + 1, u16::MAX];
    let unsigned16_bytes = fcpw::to_vec(&unsigned16).unwrap();
    assert_eq!(
        fcpw::from_slice_u16_array(&unsigned16_bytes).unwrap(),
        unsigned16
    );
    assert_eq!(
        fcpw::from_slice_u16_array(&[0x9f, 0x00, 0x19, 0xff, 0xff, 0xff]).unwrap(),
        [0, u16::MAX]
    );
    assert_eq!(
        fcpw::from_slice_u16_array(&[0x81, 0x1a, 0x00, 0x01, 0x00, 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::IntegerOverflow
    );

    for (value, encoded) in [
        (-65_537i32, vec![0x3a, 0x00, 0x01, 0x00, 0x00]),
        (-257, vec![0x39, 0x01, 0x00]),
        (-25, vec![0x38, 0x18]),
        (-24, vec![0x37]),
        (23, vec![0x17]),
        (24, vec![0x18, 0x18]),
        (256, vec![0x19, 0x01, 0x00]),
        (65_536, vec![0x1a, 0x00, 0x01, 0x00, 0x00]),
    ] {
        assert_eq!(fcpw::to_vec(&value).unwrap(), encoded);
    }
}

#[cfg(feature = "serde")]
#[test]
fn optimized_float_array_paths_preserve_widths_and_values() {
    let values = [
        0.0,
        -0.0,
        1.5,
        65_504.0,
        100_000.0,
        0.1,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    let bytes = fcpw::to_vec(&values).unwrap();
    assert_eq!(bytes.len(), 1 + values.len() * 9);
    assert!(bytes[1..].chunks_exact(9).all(|value| value[0] == 0xfb));
    let decoded: Vec<f64> = fcpw::from_slice(&bytes).unwrap();
    assert_eq!(decoded, values);
    assert_eq!(fcpw::from_slice_f64_array(&bytes).unwrap(), values);
    validate(&bytes).unwrap();

    let nan = fcpw::to_vec(&f64::NAN).unwrap();
    assert_eq!(nan[0], 0xfb);
    assert!(fcpw::from_slice::<f64>(&nan).unwrap().is_nan());
    assert_eq!(
        fcpw::to_vec_deterministic(&f64::NAN).unwrap(),
        [0xf9, 0x7e, 0x00]
    );

    let smallest_f32 = f32::from_bits(1);
    assert_eq!(
        fcpw::to_vec(&smallest_f32).unwrap(),
        [0xfa, 0x00, 0x00, 0x00, 0x01]
    );
    let mut scalar = Vec::new();
    Encoder::new(&mut scalar).f32(smallest_f32).unwrap();
    assert_eq!(scalar, [0xfa, 0x00, 0x00, 0x00, 0x01]);

    let preferred = fcpw::to_vec_deterministic(&[1.5f64, 100_000.0, 1.1]).unwrap();
    assert_eq!(preferred[0], 0x83);
    assert_eq!(preferred[1], 0xf9);
    assert_eq!(preferred[4], 0xfa);
    assert_eq!(preferred[9], 0xfb);
    assert_eq!(
        validate_deterministic(&fcpw::to_vec(&1.5f64).unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::NonDeterministic
    );

    for half in 0u16..=u16::MAX {
        let encoded = [0xf9, (half >> 8) as u8, half as u8];
        let value = fcpw::from_slice::<f64>(&encoded).unwrap();
        let reencoded = fcpw::to_vec_deterministic(&value).unwrap();
        let is_nan = half & 0x7c00 == 0x7c00 && half & 0x03ff != 0;
        if is_nan {
            assert_eq!(reencoded, [0xf9, 0x7e, 0x00]);
        } else {
            assert_eq!(reencoded, encoded);
        }
    }

    let mixed = [
        0x9f, 0x01, 0x20, 0xf9, 0x3e, 0x00, 0xfa, 0x47, 0xc3, 0x50, 0x00, 0xfb, 0x3f, 0xb9, 0x99,
        0x99, 0x99, 0x99, 0x99, 0x9a, 0xff,
    ];
    assert_eq!(
        fcpw::from_slice_f64_array(&mixed).unwrap(),
        [1.0, -1.0, 1.5, 100_000.0, 0.1]
    );
    assert_eq!(
        fcpw::from_slice_f64_array(&[0x81, 0x61, b'x'])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedType
    );

    let values32 = [0.0f32, -0.0, 1.5, 100_000.0, 0.1, f32::INFINITY];
    let bytes32 = fcpw::to_vec(&values32).unwrap();
    assert_eq!(bytes32.len(), 1 + values32.len() * 5);
    assert!(bytes32[1..].chunks_exact(5).all(|value| value[0] == 0xfa));
    assert_eq!(fcpw::from_slice_f32_array(&bytes32).unwrap(), values32);

    assert_eq!(
        fcpw::from_slice_f32_array(&mixed).unwrap(),
        [1.0f32, -1.0, 1.5, 100_000.0, 0.1]
    );
    assert_eq!(
        fcpw::from_slice_f32_array(&[0x81, 0x61, b'x'])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedType
    );
}

#[cfg(feature = "serde")]
#[test]
fn deterministic_serializer_sorts_map_keys() {
    use serde::ser::{SerializeMap, SerializeSeq};
    use std::collections::BTreeMap;

    let map = BTreeMap::from([("z", 1u8), ("a", 2)]);
    let bytes = fcpw::to_vec_deterministic(&map).unwrap();
    validate_deterministic(&bytes).unwrap();

    struct UnknownLength;
    impl serde::Serialize for UnknownLength {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(None)?;
            sequence.serialize_element(&1u8)?;
            sequence.serialize_element(&2u8)?;
            sequence.end()
        }
    }
    assert_eq!(
        fcpw::to_vec_deterministic(&UnknownLength).unwrap(),
        [0x82, 1, 2]
    );

    struct DuplicateKeys;
    impl serde::Serialize for DuplicateKeys {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("same", &1u8)?;
            map.serialize_entry("same", &2u8)?;
            map.end()
        }
    }
    assert_eq!(
        fcpw::to_vec_deterministic(&DuplicateKeys)
            .unwrap_err()
            .kind(),
        ErrorKind::DuplicateKey
    );
}

#[cfg(feature = "serde")]
#[test]
fn reusable_vec_encoders_retain_capacity_and_clear_failures() {
    let values = [1i32, -2, 100_000, i32::MIN];
    let expected = fcpw::to_vec(&values).unwrap();
    let mut output = Vec::with_capacity(256);
    output.extend_from_slice(b"old contents");
    let pointer = output.as_ptr();
    let capacity = output.capacity();

    fcpw::to_vec_into(&values, &mut output).unwrap();
    assert_eq!(output, expected);
    assert_eq!(output.as_ptr(), pointer);
    assert_eq!(output.capacity(), capacity);

    fcpw::EncodeConfig::new()
        .serialize_into(&true, &mut output)
        .unwrap();
    assert_eq!(output, [0xf5]);
    assert_eq!(output.as_ptr(), pointer);

    let floats = [1.5f64, 100_000.0, 1.1];
    let deterministic = fcpw::to_vec_deterministic(&floats).unwrap();
    fcpw::to_vec_deterministic_into(&floats, &mut output).unwrap();
    assert_eq!(output, deterministic);
    fcpw::EncodeConfig::deterministic()
        .serialize_into(&floats, &mut output)
        .unwrap();
    assert_eq!(output, deterministic);
    assert_eq!(output.as_ptr(), pointer);

    let map = std::collections::BTreeMap::from([("z", 1u8), ("a", 2)]);
    let deterministic_map = fcpw::to_vec_deterministic(&map).unwrap();
    let mut scratch = fcpw::DeterministicScratch::new();
    fcpw::to_vec_deterministic_into_with_scratch(&map, &mut output, &mut scratch).unwrap();
    assert_eq!(output, deterministic_map);
    fcpw::to_vec_deterministic_into_with_scratch(&map, &mut output, &mut scratch).unwrap();
    assert_eq!(output, deterministic_map);

    struct FailsAfterOneElement;
    impl serde::Serialize for FailsAfterOneElement {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;

            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element(&1u8)?;
            Err(<S::Error as serde::ser::Error>::custom(
                "intentional failure",
            ))
        }
    }

    assert!(fcpw::to_vec_into(&FailsAfterOneElement, &mut output).is_err());
    assert!(output.is_empty());
    assert_eq!(output.as_ptr(), pointer);
    assert_eq!(output.capacity(), capacity);
    assert!(fcpw::to_vec_deterministic_into(&FailsAfterOneElement, &mut output).is_err());
    assert!(output.is_empty());
    assert_eq!(output.as_ptr(), pointer);
}

#[cfg(feature = "serde")]
#[test]
fn writer_backed_serializer_is_incremental_reusable_and_returns_output() {
    use serde::Serialize as _;

    #[derive(Default)]
    struct RecordingOutput {
        bytes: Vec<u8>,
        writes: Vec<usize>,
    }
    impl fcpw::Output for RecordingOutput {
        fn write_all(&mut self, bytes: &[u8]) -> fcpw::Result<()> {
            self.writes.push(bytes.len());
            self.bytes.extend_from_slice(bytes);
            Ok(())
        }
    }

    let value = (1u8, "incremental", vec![2u16, 3, 4]);
    let mut serializer = fcpw::Serializer::new(RecordingOutput::default());
    value.serialize(&mut serializer).unwrap();
    true.serialize(&mut serializer).unwrap();
    let output = serializer.into_inner();

    let mut expected = fcpw::to_vec(&value).unwrap();
    expected.extend_from_slice(&fcpw::to_vec(&true).unwrap());
    assert_eq!(output.bytes, expected);
    assert!(output.writes.len() > 1);

    let mut storage = [0u8; 2];
    let mut serializer = fcpw::Serializer::new(fcpw::SliceOutput::new(&mut storage));
    assert_eq!(
        "too long"
            .serialize(&mut serializer)
            .unwrap_err()
            .to_string(),
        "OutputTooSmall"
    );
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_sequence_preserves_order() {
    let values: Vec<u64> = fcpw::parallel::from_sequence(&[1, 2, 3]).unwrap();
    assert_eq!(values, [1, 2, 3]);

    let error = fcpw::parallel::from_sequence::<String>(&[0x61, b'a', 0x61, 0xff]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidUtf8);
    assert_eq!(error.item_index(), Some(1));

    let force_pipeline = fcpw::parallel::ParallelOptions {
        min_bytes: 0,
        min_items: 0,
    };
    let input: Vec<u8> = (0..64).map(|value| (value % 24) as u8).collect();
    let values = fcpw::parallel::from_sequence_with_options::<u64>(&input, force_pipeline).unwrap();
    assert_eq!(values, (0..64).map(|value| value % 24).collect::<Vec<_>>());

    let mut invalid = vec![0x61, b'a'];
    invalid.extend([0; 8]);
    invalid.push(0x18);
    let boundary_error =
        fcpw::parallel::from_sequence_with_options::<u64>(&invalid, force_pipeline).unwrap_err();
    assert_eq!(boundary_error.kind(), ErrorKind::Eof);
    assert_eq!(boundary_error.item_index(), Some(9));
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_structural_boundaries_match_public_sequence_errors() {
    use serde::de::IgnoredAny;

    let valid = [
        0x9f, 0x01, 0x7f, 0x61, b'a', 0xff, 0xff, 0xbf, 0x01, 0x82, 0xf4, 0xd8, 0x64, 0x02, 0xff,
    ];
    let decoded = fcpw::parallel::from_sequence::<IgnoredAny>(&valid).unwrap();
    assert_eq!(decoded.len(), 2);

    for bytes in [
        &[0x62, b'a'][..],
        &[0x5f, 0x61, b'a', 0xff],
        &[0xbf, 0x01, 0xff],
        &[0x81, 0xff],
        &[0xd8, 0x64],
        &[0xf8, 0x01],
        &[0xff],
        &[0x61, 0xff],
    ] {
        let expected = SequenceDecoder::new(bytes).next().unwrap().unwrap_err();
        let actual = fcpw::parallel::from_sequence::<IgnoredAny>(bytes).unwrap_err();
        assert_eq!(actual.kind(), expected.kind(), "{bytes:02x?}");
        assert_eq!(actual.offset(), expected.offset(), "{bytes:02x?}");
        assert_eq!(actual.item_index(), expected.item_index(), "{bytes:02x?}");
    }
}

#[cfg(feature = "diagnostic")]
#[test]
fn diagnostic_format_and_parse() {
    assert_eq!(
        fcpw::diagnostic::format(&[0x82, 1, 0x62, b'o', b'k']).unwrap(),
        r#"[1, "ok"]"#
    );
    assert_eq!(
        fcpw::diagnostic::parse(r#"{"ok": true}"#).unwrap(),
        fcpw::Value::Map(vec![(
            fcpw::Value::Text(String::from("ok")),
            fcpw::Value::Bool(true)
        )])
    );
}

#[cfg(all(feature = "serde", feature = "std"))]
#[test]
fn slice_and_io_adapters_round_trip() {
    use std::io::{self, Write};

    let mut storage = [0; 16];
    let encoded = fcpw::to_slice(&[1u8, 2, 3], &mut storage).unwrap();
    let mut written = Vec::new();
    fcpw::to_writer(&mut written, &[1u8, 2, 3]).unwrap();
    assert_eq!(encoded, written);
    assert_eq!(fcpw::serialized_size(&[1u8, 2, 3]).unwrap(), encoded.len());

    let mut too_small = [0; 3];
    assert_eq!(
        fcpw::to_slice(&[1u8, 2, 3], &mut too_small)
            .unwrap_err()
            .kind(),
        ErrorKind::OutputTooSmall
    );

    #[derive(Default)]
    struct PartialWriter(Vec<u8>);
    impl Write for PartialWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(1);
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut partial = PartialWriter::default();
    fcpw::to_writer(&mut partial, &[1u8, 2, 3]).unwrap();
    assert_eq!(partial.0, encoded);

    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let error = fcpw::to_writer(FailingWriter, &[1u8, 2, 3]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(error.offset(), 0);
    assert_eq!(error.io_error().unwrap().kind(), io::ErrorKind::BrokenPipe);
    assert!(std::error::Error::source(&error).is_some());

    let decoded: Vec<u8> = fcpw::from_reader(written.as_slice()).unwrap();
    assert_eq!(decoded, [1, 2, 3]);
}

#[cfg(feature = "serde")]
#[test]
fn reusable_bulk_decoders_preserve_capacity_and_clear_on_error() {
    macro_rules! check_reuse {
        ($values:expr, $into:path) => {{
            let values = $values;
            let bytes = fcpw::to_vec(&values).unwrap();
            let mut output = Vec::with_capacity(values.len() + 16);
            let capacity = output.capacity();
            $into(&bytes, &mut output).unwrap();
            assert_eq!(output, values);
            assert_eq!(output.capacity(), capacity);
            assert!($into(&[0x81], &mut output).is_err());
            assert!(output.is_empty());
            assert_eq!(output.capacity(), capacity);
        }};
    }

    check_reuse!(
        vec![false, true, false, true],
        fcpw::from_slice_bool_array_into
    );
    check_reuse!(vec![0u8, 24, 255], fcpw::from_slice_u8_array_into);
    check_reuse!(vec![0u16, 256, u16::MAX], fcpw::from_slice_u16_array_into);
    check_reuse!(
        vec![0u32, 65_536, u32::MAX],
        fcpw::from_slice_u32_array_into
    );
    check_reuse!(
        vec![0u64, 1 << 32, u64::MAX],
        fcpw::from_slice_u64_array_into
    );
    check_reuse!(vec![i8::MIN, 0, i8::MAX], fcpw::from_slice_i8_array_into);
    check_reuse!(vec![i16::MIN, 0, i16::MAX], fcpw::from_slice_i16_array_into);
    check_reuse!(vec![i32::MIN, 0, i32::MAX], fcpw::from_slice_i32_array_into);
    check_reuse!(vec![i64::MIN, 0, i64::MAX], fcpw::from_slice_i64_array_into);
    check_reuse!(
        vec![0.25f32, -1.5, f32::MAX],
        fcpw::from_slice_f32_array_into
    );
    check_reuse!(
        vec![0.25f64, -1.5, f64::MAX],
        fcpw::from_slice_f64_array_into
    );
}

#[cfg(all(feature = "serde", feature = "std"))]
#[test]
fn from_reader_handles_chunk_boundaries_and_trailing_data() {
    use std::io::{self, Read};

    struct ShortReads<'a> {
        bytes: &'a [u8],
        position: usize,
    }
    impl Read for ShortReads<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let remaining = &self.bytes[self.position..];
            let length = remaining.len().min(output.len()).min(3);
            output[..length].copy_from_slice(&remaining[..length]);
            self.position += length;
            Ok(length)
        }
    }

    let value = "reader chunk boundary".repeat(1024);
    let bytes = fcpw::to_vec(&value).unwrap();
    let decoded: String = fcpw::from_reader(ShortReads {
        bytes: &bytes,
        position: 0,
    })
    .unwrap();
    assert_eq!(decoded, value);

    let mut buffer = Vec::with_capacity(bytes.len());
    let capacity = buffer.capacity();
    let decoded: String = fcpw::from_reader_with_buffer(bytes.as_slice(), &mut buffer).unwrap();
    assert_eq!(decoded, value);
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), capacity);

    let mut trailing = fcpw::to_vec(&42u64).unwrap();
    trailing.extend([0; 32 * 1024]);
    assert_eq!(
        fcpw::from_reader::<u64, _>(trailing.as_slice())
            .unwrap_err()
            .kind(),
        ErrorKind::TrailingData
    );
}

#[cfg(all(feature = "serde", feature = "std"))]
#[test]
fn structured_io_errors_preserve_cause_offset_and_retry_interrupts() {
    use std::io::{self, Read, Write};

    struct FailingReader {
        bytes: &'static [u8],
        position: usize,
        interrupted: bool,
    }
    impl Read for FailingReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.position < self.bytes.len() {
                let length = (self.bytes.len() - self.position).min(output.len());
                output[..length]
                    .copy_from_slice(&self.bytes[self.position..self.position + length]);
                self.position += length;
                return Ok(length);
            }
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "reader failed",
            ))
        }
    }

    let read_error = fcpw::from_reader::<u64, _>(FailingReader {
        bytes: &[0x19],
        position: 0,
        interrupted: false,
    })
    .unwrap_err();
    assert_eq!(read_error.kind(), ErrorKind::Io);
    assert_eq!(read_error.offset(), 1);
    assert_eq!(
        read_error.io_error().unwrap().kind(),
        io::ErrorKind::ConnectionReset
    );
    assert!(read_error.to_string().contains("reader failed"));
    assert_eq!(
        std::error::Error::source(&read_error).unwrap().to_string(),
        "reader failed"
    );

    struct FailingWriter {
        remaining: usize,
        interrupted: bool,
    }
    impl Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.remaining == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer failed"));
            }
            let written = bytes.len().min(self.remaining);
            self.remaining -= written;
            Ok(written)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let write_error = fcpw::to_writer(
        FailingWriter {
            remaining: 2,
            interrupted: false,
        },
        &"payload",
    )
    .unwrap_err();
    assert_eq!(write_error.kind(), ErrorKind::Io);
    assert_eq!(write_error.offset(), 2);
    assert_eq!(
        write_error.io_error().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(write_error.to_string().contains("writer failed"));
}
