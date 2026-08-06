#![cfg(feature = "alloc")]

use fcpw::{ErrorKind, Value};

#[test]
fn predicates_and_borrowing_accessors_match_only_their_variants() {
    let mut bytes = Value::Bytes(vec![1, 2]);
    assert!(bytes.is_bytes());
    assert!(!bytes.is_text());
    assert_eq!(bytes.as_bytes(), Some(&[1, 2][..]));
    assert_eq!(bytes.as_text(), None);
    bytes.as_bytes_mut().unwrap().push(3);
    assert_eq!(bytes.as_bytes(), Some(&[1, 2, 3][..]));

    let values = [
        Value::Unsigned(1),
        Value::Negative(-1),
        Value::Text("x".into()),
        Value::Array(vec![]),
        Value::Map(vec![]),
        Value::Tag(1, Box::new(Value::Null)),
        Value::Simple(32),
        Value::Bool(true),
        Value::Null,
        Value::Undefined,
        Value::Float(1.0),
    ];
    assert!(values[0].is_unsigned() && values[0].is_integer());
    assert!(values[1].is_integer() && !values[1].is_unsigned());
    assert!(values[2].is_text());
    assert!(values[3].is_array());
    assert!(values[4].is_map());
    assert!(values[5].is_tag() && !values[5].is_integer());
    assert!(values[6].is_simple());
    assert!(values[7].is_bool());
    assert!(values[8].is_null());
    assert!(values[9].is_undefined());
    assert!(values[10].is_float());
    assert_eq!(values[5].as_tag().map(|(tag, _)| tag), Some(1));
    assert_eq!(values[6].as_simple(), Some(32));
    assert_eq!(values[7].as_bool(), Some(true));
    assert_eq!(values[10].as_float(), Some(1.0));
}

#[test]
fn mutable_and_consuming_collection_accessors_preserve_contents() {
    let duplicate_map = vec![
        (Value::from("key"), Value::from(1_u8)),
        (Value::from("key"), Value::from(2_u8)),
    ];
    let mut map = Value::from(duplicate_map.clone());
    map.as_map_mut().unwrap().push((Value::Null, Value::Null));
    assert_eq!(&map.as_map().unwrap()[..2], duplicate_map);
    assert_eq!(map.into_map().unwrap().len(), 3);

    let text = String::from("owned");
    assert_eq!(Value::from(text.clone()).into_text(), Ok(text));
    assert_eq!(Value::from(vec![1_u8, 2]).into_bytes(), Ok(vec![1, 2]));
    assert_eq!(
        Value::from(vec![Value::Null]).into_array(),
        Ok(vec![Value::Null])
    );
    assert_eq!(Value::Null.into_text(), Err(Value::Null));
}

#[test]
fn integer_conversions_are_checked_and_do_not_interpret_bignum_tags() {
    assert_eq!(u8::try_from(Value::Unsigned(255)).unwrap(), 255);
    assert_eq!(i8::try_from(Value::Negative(-128)).unwrap(), -128);
    assert_eq!(
        i128::try_from(Value::Unsigned(u64::MAX)).unwrap(),
        u64::MAX as i128
    );
    assert_eq!(
        u128::try_from(Value::Unsigned(u64::MAX)).unwrap(),
        u64::MAX as u128
    );

    for value in [Value::Unsigned(256), Value::Negative(-1)] {
        assert_eq!(
            u8::try_from(value).unwrap_err().kind(),
            ErrorKind::IntegerOverflow
        );
    }
    assert_eq!(
        i8::try_from(Value::Negative(-129)).unwrap_err().kind(),
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        u64::try_from(Value::Text("1".into())).unwrap_err().kind(),
        ErrorKind::UnexpectedType
    );
    assert_eq!(
        u128::try_from(Value::Tag(2, Box::new(Value::Bytes(vec![1]))))
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedType
    );
}

#[test]
fn primitive_from_conversions_choose_unambiguous_variants() {
    assert_eq!(Value::from(-1_i64), Value::Negative(-1));
    assert_eq!(Value::from(0_i64), Value::Unsigned(0));
    assert_eq!(Value::from(1_u64), Value::Unsigned(1));
    assert_eq!(Value::from("text"), Value::Text("text".into()));
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from(()), Value::Null);
}
