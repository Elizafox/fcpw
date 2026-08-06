#![no_main]

use std::collections::BTreeMap;

use fcpw::{
    ErrorKind, from_slice, serialized_size, to_slice, to_vec, to_vec_deterministic, to_vec_into,
    validate, validate_deterministic,
};
use libfuzzer_sys::{
    arbitrary::{self, Unstructured},
    fuzz_target,
};
use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 16;
const MAX_BYTES: usize = 96;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Choice {
    Unit,
    Newtype(i64),
    Tuple(u8, String),
    Struct { code: u32, enabled: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Payload {
    signed: i64,
    unsigned: u64,
    float32: f32,
    float64: f64,
    flag: bool,
    text: String,
    bytes: Vec<u8>,
    optional: Option<i32>,
    sequence: Vec<i16>,
    map: BTreeMap<String, i64>,
    choice: Choice,
}

fn small_bytes(input: &mut Unstructured<'_>) -> arbitrary::Result<Vec<u8>> {
    let len = usize::from(input.arbitrary::<u8>()?) % (MAX_BYTES + 1);
    Ok(input.bytes(len)?.to_vec())
}

fn text(input: &mut Unstructured<'_>) -> arbitrary::Result<String> {
    Ok(String::from_utf8_lossy(&small_bytes(input)?).into_owned())
}

fn payload(input: &mut Unstructured<'_>) -> arbitrary::Result<Payload> {
    let float32 = f32::from_bits(input.arbitrary()?);
    let float64 = f64::from_bits(input.arbitrary()?);

    let sequence_len = usize::from(input.arbitrary::<u8>()?) % (MAX_ITEMS + 1);
    let mut sequence = Vec::with_capacity(sequence_len);
    for _ in 0..sequence_len {
        sequence.push(input.arbitrary()?);
    }

    let map_len = usize::from(input.arbitrary::<u8>()?) % (MAX_ITEMS + 1);
    let mut map = BTreeMap::new();
    for _ in 0..map_len {
        map.insert(text(input)?, input.arbitrary()?);
    }

    let choice = match input.arbitrary::<u8>()? % 4 {
        0 => Choice::Unit,
        1 => Choice::Newtype(input.arbitrary()?),
        2 => Choice::Tuple(input.arbitrary()?, text(input)?),
        _ => Choice::Struct {
            code: input.arbitrary()?,
            enabled: input.arbitrary()?,
        },
    };

    Ok(Payload {
        signed: input.arbitrary()?,
        unsigned: input.arbitrary()?,
        float32: if float32.is_nan() { 0.0 } else { float32 },
        float64: if float64.is_nan() { 0.0 } else { float64 },
        flag: input.arbitrary()?,
        text: text(input)?,
        bytes: small_bytes(input)?,
        optional: input.arbitrary()?,
        sequence,
        map,
        choice,
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(value) = payload(&mut Unstructured::new(data)) else {
        return;
    };

    let encoded = to_vec(&value).unwrap();
    validate(&encoded).unwrap();
    assert_eq!(from_slice::<Payload>(&encoded).unwrap(), value);
    assert_eq!(serialized_size(&value).unwrap(), encoded.len());

    let mut reused = vec![0xaa; encoded.len() + 17];
    to_vec_into(&value, &mut reused).unwrap();
    assert_eq!(reused, encoded);

    let mut exact = vec![0; encoded.len()];
    assert_eq!(to_slice(&value, &mut exact).unwrap(), encoded);
    let mut short = vec![0; encoded.len() - 1];
    assert_eq!(
        to_slice(&value, &mut short).unwrap_err().kind(),
        ErrorKind::OutputTooSmall
    );

    let deterministic = to_vec_deterministic(&value).unwrap();
    validate_deterministic(&deterministic).unwrap();
    assert_eq!(from_slice::<Payload>(&deterministic).unwrap(), value);
    assert_eq!(to_vec_deterministic(&value).unwrap(), deterministic);

    // Cross-decode against a mature implementation. Exact byte equality is
    // intentionally not required: FCPW preserves source float width, whereas
    // serde_cbor may select a shorter exact representation.
    let reference = serde_cbor::to_vec(&value).unwrap();
    validate(&reference).unwrap();
    assert_eq!(serde_cbor::from_slice::<Payload>(&encoded).unwrap(), value);
    assert_eq!(from_slice::<Payload>(&reference).unwrap(), value);
});
