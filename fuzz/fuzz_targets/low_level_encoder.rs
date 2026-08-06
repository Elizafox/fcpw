#![no_main]

use fcpw::{
    Encoder, Output, SliceOutput, Value, from_slice_value, validate, validate_deterministic,
};
use libfuzzer_sys::{
    arbitrary::{self, Unstructured},
    fuzz_target,
};

const MAX_DEPTH: usize = 5;
const MAX_ITEMS: usize = 8;
const MAX_BYTES: usize = 64;

fn small_bytes(input: &mut Unstructured<'_>) -> arbitrary::Result<Vec<u8>> {
    let len = usize::from(input.arbitrary::<u8>()?) % (MAX_BYTES + 1);
    Ok(input.bytes(len)?.to_vec())
}

fn value(input: &mut Unstructured<'_>, depth: usize) -> arbitrary::Result<Value> {
    let leaf_count = 11;
    let choices = if depth == MAX_DEPTH { leaf_count } else { 14 };
    match usize::from(input.arbitrary::<u8>()?) % choices {
        0 => Ok(Value::Unsigned(input.arbitrary()?)),
        1 => {
            let magnitude = u128::from(input.arbitrary::<u64>()?);
            Ok(Value::Negative(-1 - magnitude as i128))
        }
        2 => Ok(Value::Bytes(small_bytes(input)?)),
        3 => Ok(Value::Text(
            String::from_utf8_lossy(&small_bytes(input)?).into_owned(),
        )),
        4 => {
            let mut simple = input.arbitrary::<u8>()?;
            if (20..32).contains(&simple) {
                simple = 32;
            }
            Ok(Value::Simple(simple))
        }
        5 => Ok(Value::Bool(input.arbitrary()?)),
        6 => Ok(Value::Null),
        7 => Ok(Value::Undefined),
        8 => {
            let candidate = f64::from_bits(input.arbitrary()?);
            Ok(Value::Float(if candidate.is_nan() {
                0.0
            } else {
                candidate
            }))
        }
        9 => Ok(Value::Unsigned(u64::MAX)),
        10 => Ok(Value::Negative(-(u64::MAX as i128) - 1)),
        11 => {
            let len = usize::from(input.arbitrary::<u8>()?) % (MAX_ITEMS + 1);
            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(value(input, depth + 1)?);
            }
            Ok(Value::Array(items))
        }
        12 => {
            let len = usize::from(input.arbitrary::<u8>()?) % (MAX_ITEMS + 1);
            let mut entries = Vec::with_capacity(len);
            for _ in 0..len {
                entries.push((value(input, depth + 1)?, value(input, depth + 1)?));
            }
            Ok(Value::Map(entries))
        }
        13 => Ok(Value::Tag(
            input.arbitrary()?,
            Box::new(value(input, depth + 1)?),
        )),
        _ => unreachable!(),
    }
}

fn encode<O: Output>(value: &Value, output: O) -> fcpw::Result<O> {
    let mut encoder = Encoder::new(output);
    value.encode(&mut encoder)?;
    Ok(encoder.into_inner())
}

fuzz_target!(|data: &[u8]| {
    let mut input = Unstructured::new(data);
    let Ok(value) = value(&mut input, 0) else {
        return;
    };

    // The growable and allocation-free encoder paths must produce identical,
    // well-formed CBOR, and decoding must preserve the generated value.
    let encoded = encode(&value, Vec::new()).unwrap();
    validate(&encoded).unwrap();
    assert_eq!(from_slice_value(&encoded).unwrap(), value);

    let mut storage = vec![0; encoded.len()];
    let output = encode(&value, SliceOutput::new(&mut storage)).unwrap();
    assert_eq!(output.len(), encoded.len());
    assert_eq!(storage, encoded);

    // Every item needs at least one byte, so a buffer one byte too short must
    // report an error rather than panic or claim success.
    let mut short = vec![0; encoded.len() - 1];
    assert!(encode(&value, SliceOutput::new(&mut short)).is_err());

    // Exercise preferred-width float selection independently of Value's
    // source-width-preserving f64 representation.
    let bits = data
        .get(..8)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0);
    let mut preferred = Vec::new();
    Encoder::new(&mut preferred)
        .f64_preferred(f64::from_bits(bits))
        .unwrap();
    validate_deterministic(&preferred).unwrap();
});
