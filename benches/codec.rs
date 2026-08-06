use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use std::{collections::BTreeMap, hint::black_box};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    id: u64,
    timestamp: i64,
    name: String,
    active: bool,
    scores: Vec<i32>,
    metadata: Vec<(String, String)>,
    payload: ByteBuf,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct BorrowedRecord<'a> {
    id: u64,
    name: &'a str,
    #[serde(borrow, with = "serde_bytes")]
    payload: &'a [u8],
}

fn record(scale: usize) -> Record {
    Record {
        id: 9_007_199_254_740_991,
        timestamp: -1_725_000_000,
        name: "variation-tolerant CBOR benchmark record".repeat(scale),
        active: true,
        scores: (0..scale * 32).map(|n| n as i32 - 50).collect(),
        metadata: (0..scale * 4)
            .map(|n| (format!("key-{n:03}"), format!("value-{n:03}")))
            .collect(),
        payload: ByteBuf::from(
            (0..scale * 256)
                .map(|n| n.wrapping_mul(31) as u8)
                .collect::<Vec<_>>(),
        ),
    }
}

fn fcpw_bytes(value: &Record) -> Vec<u8> {
    fcpw::to_vec(value).unwrap()
}

fn ciborium_bytes(value: &Record) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).unwrap();
    bytes
}

fn serde_cbor_bytes(value: &Record) -> Vec<u8> {
    serde_cbor::to_vec(value).unwrap()
}

fn cbor4ii_bytes(value: &Record) -> Vec<u8> {
    cbor4ii::serde::to_vec(Vec::new(), value).unwrap()
}

fn codec_benchmarks(c: &mut Criterion) {
    for (name, value) in [("small", record(1)), ("medium", record(16))] {
        let wire = fcpw_bytes(&value);
        let outputs = [
            ("fcpw", wire.len()),
            ("ciborium", ciborium_bytes(&value).len()),
            ("serde_cbor", serde_cbor_bytes(&value).len()),
            ("cbor4ii", cbor4ii_bytes(&value).len()),
        ];
        eprintln!("{name} encoded sizes: {outputs:?}");

        let mut decode = c.benchmark_group(format!("decode/{name}"));
        decode.throughput(Throughput::Bytes(wire.len() as u64));
        decode.bench_function(BenchmarkId::new("fcpw", wire.len()), |b| {
            b.iter(|| fcpw::from_slice::<Record>(black_box(&wire)).unwrap())
        });
        decode.bench_function(BenchmarkId::new("ciborium", wire.len()), |b| {
            b.iter(|| ciborium::from_reader::<Record, _>(black_box(wire.as_slice())).unwrap())
        });
        decode.bench_function(BenchmarkId::new("serde_cbor", wire.len()), |b| {
            b.iter(|| serde_cbor::from_slice::<Record>(black_box(&wire)).unwrap())
        });
        decode.bench_function(BenchmarkId::new("cbor4ii", wire.len()), |b| {
            b.iter(|| cbor4ii::serde::from_slice::<Record>(black_box(&wire)).unwrap())
        });
        decode.finish();

        let mut encode = c.benchmark_group(format!("encode/{name}"));
        encode.throughput(Throughput::Bytes(wire.len() as u64));
        encode.bench_function("fcpw", |b| b.iter(|| fcpw_bytes(black_box(&value))));
        encode.bench_function("ciborium", |b| b.iter(|| ciborium_bytes(black_box(&value))));
        encode.bench_function("serde_cbor", |b| {
            b.iter(|| serde_cbor_bytes(black_box(&value)))
        });
        encode.bench_function("cbor4ii", |b| b.iter(|| cbor4ii_bytes(black_box(&value))));
        encode.finish();
    }
}

fn validation_benchmarks(c: &mut Criterion) {
    for (name, value) in [("small", record(1)), ("medium", record(16))] {
        let wire = fcpw_bytes(&value);
        let mut group = c.benchmark_group(format!("validate/{name}"));
        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_function("fcpw", |b| {
            b.iter(|| fcpw::validate(black_box(&wire)).unwrap())
        });
        group.finish();

        let deterministic_wire = fcpw::to_vec_deterministic(&value).unwrap();
        let mut decode = c.benchmark_group(format!("decode-validation/{name}"));
        decode.throughput(Throughput::Bytes(wire.len() as u64));
        decode.bench_function("strict", |b| {
            let options = fcpw::DecodeOptions {
                validation: fcpw::Validation::Strict,
                ..fcpw::DecodeOptions::default()
            };
            b.iter(|| fcpw::from_slice_with_options::<Record>(black_box(&wire), options).unwrap())
        });
        decode.bench_function("deterministic", |b| {
            let options = fcpw::DecodeOptions {
                validation: fcpw::Validation::Deterministic,
                ..fcpw::DecodeOptions::default()
            };
            b.iter(|| {
                fcpw::from_slice_with_options::<Record>(black_box(&deterministic_wire), options)
                    .unwrap()
            })
        });
        decode.finish();
    }
}

fn focused_benchmarks(c: &mut Criterion) {
    let integers: Vec<i32> = (0..4096).map(|value| value - 2048).collect();
    let integer_wire = fcpw::to_vec(&integers).unwrap();
    let mut integers_decode = c.benchmark_group("decode/integer-array");
    integers_decode.throughput(Throughput::Bytes(integer_wire.len() as u64));
    integers_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<i32>>(black_box(&integer_wire)).unwrap())
    });
    integers_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_i32_array(black_box(&integer_wire)).unwrap())
    });
    integers_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<i32>>(black_box(&integer_wire)).unwrap())
    });
    integers_decode.finish();

    let mut integers_encode = c.benchmark_group("encode/integer-array");
    integers_encode.throughput(Throughput::Bytes(integer_wire.len() as u64));
    integers_encode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::to_vec(black_box(&integers)).unwrap())
    });
    integers_encode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::to_vec(Vec::new(), black_box(&integers)).unwrap())
    });
    integers_encode.finish();

    let borrowed = BorrowedRecord {
        id: 42,
        name: "borrowed zero-copy benchmark",
        payload: black_box(&[0x5a; 4096]),
    };
    let borrowed_wire = fcpw::to_vec(&borrowed).unwrap();
    let mut borrowed_decode = c.benchmark_group("decode/borrowed");
    borrowed_decode.throughput(Throughput::Bytes(borrowed_wire.len() as u64));
    borrowed_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<BorrowedRecord<'_>>(black_box(&borrowed_wire)).unwrap())
    });
    borrowed_decode.bench_function("cbor4ii", |b| {
        b.iter(|| {
            cbor4ii::serde::from_slice::<BorrowedRecord<'_>>(black_box(&borrowed_wire)).unwrap()
        })
    });
    borrowed_decode.finish();

    let floats: Vec<f64> = (0..4096).map(|value| value as f64 * 0.1 + 0.03).collect();
    let float_wire = fcpw::to_vec(&floats).unwrap();
    let mut float_decode = c.benchmark_group("decode/float-array");
    float_decode.throughput(Throughput::Bytes(float_wire.len() as u64));
    float_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<f64>>(black_box(&float_wire)).unwrap())
    });
    float_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_f64_array(black_box(&float_wire)).unwrap())
    });
    float_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<f64>>(black_box(&float_wire)).unwrap())
    });
    float_decode.finish();

    let floats32: Vec<f32> = (0..4096).map(|value| value as f32 * 0.1 + 0.03).collect();
    let float32_wire = fcpw::to_vec(&floats32).unwrap();
    let mut float32_decode = c.benchmark_group("decode/float32-array");
    float32_decode.throughput(Throughput::Bytes(float32_wire.len() as u64));
    float32_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<f32>>(black_box(&float32_wire)).unwrap())
    });
    float32_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_f32_array(black_box(&float32_wire)).unwrap())
    });
    float32_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<f32>>(black_box(&float32_wire)).unwrap())
    });
    float32_decode.finish();

    let mut float_encode = c.benchmark_group("encode/float-array");
    float_encode.throughput(Throughput::Bytes(float_wire.len() as u64));
    float_encode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::to_vec(black_box(&floats)).unwrap())
    });
    float_encode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::to_vec(Vec::new(), black_box(&floats)).unwrap())
    });
    float_encode.finish();
}

fn scalar_and_collection_benchmarks(c: &mut Criterion) {
    let booleans: Vec<bool> = (0..4096).map(|value| value % 3 != 0).collect();
    let boolean_wire = fcpw::to_vec(&booleans).unwrap();
    let mut boolean_decode = c.benchmark_group("decode/bool-array");
    boolean_decode.throughput(Throughput::Bytes(boolean_wire.len() as u64));
    boolean_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<bool>>(black_box(&boolean_wire)).unwrap())
    });
    boolean_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_bool_array(black_box(&boolean_wire)).unwrap())
    });
    boolean_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<bool>>(black_box(&boolean_wire)).unwrap())
    });
    boolean_decode.finish();
    let unsigned8: Vec<u8> = (0..4096).map(|value| value as u8).collect();
    let unsigned8_wire = fcpw::to_vec(&unsigned8).unwrap();
    let mut unsigned8_decode = c.benchmark_group("decode/u8-array");
    unsigned8_decode.throughput(Throughput::Bytes(unsigned8_wire.len() as u64));
    unsigned8_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<u8>>(black_box(&unsigned8_wire)).unwrap())
    });
    unsigned8_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_u8_array(black_box(&unsigned8_wire)).unwrap())
    });
    unsigned8_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<u8>>(black_box(&unsigned8_wire)).unwrap())
    });
    unsigned8_decode.finish();
    let integers64: Vec<i64> = (0..4096)
        .map(|value| (value as i64 - 2048) * 1_000_000_007)
        .collect();
    let integer64_wire = fcpw::to_vec(&integers64).unwrap();
    let mut integer64_decode = c.benchmark_group("decode/i64-array");
    integer64_decode.throughput(Throughput::Bytes(integer64_wire.len() as u64));
    integer64_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<i64>>(black_box(&integer64_wire)).unwrap())
    });
    integer64_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_i64_array(black_box(&integer64_wire)).unwrap())
    });
    integer64_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<i64>>(black_box(&integer64_wire)).unwrap())
    });
    integer64_decode.finish();

    let unsigned64: Vec<u64> = (0..4096)
        .map(|value| value as u64 * 1_000_000_007)
        .collect();
    let unsigned64_wire = fcpw::to_vec(&unsigned64).unwrap();
    let mut unsigned64_decode = c.benchmark_group("decode/u64-array");
    unsigned64_decode.throughput(Throughput::Bytes(unsigned64_wire.len() as u64));
    unsigned64_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<u64>>(black_box(&unsigned64_wire)).unwrap())
    });
    unsigned64_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_u64_array(black_box(&unsigned64_wire)).unwrap())
    });
    unsigned64_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<u64>>(black_box(&unsigned64_wire)).unwrap())
    });
    unsigned64_decode.finish();

    let mut mixed_unsigned64 = Vec::with_capacity(4096);
    mixed_unsigned64.extend((0..1024).map(|value| (value % 24) as u64));
    mixed_unsigned64.extend((0..1024).map(|value| 24 + (value % 232) as u64));
    mixed_unsigned64.extend((0..1024).map(|value| 256 + (value * 61) as u64));
    mixed_unsigned64.extend((0..1024).map(|value| 65_536 + (value * 1_000_003) as u64));
    let mixed_unsigned64_wire = fcpw::to_vec(&mixed_unsigned64).unwrap();
    let mut mixed_unsigned64_decode = c.benchmark_group("decode/u64-mixed-width-array");
    mixed_unsigned64_decode.throughput(Throughput::Bytes(mixed_unsigned64_wire.len() as u64));
    mixed_unsigned64_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<u64>>(black_box(&mixed_unsigned64_wire)).unwrap())
    });
    mixed_unsigned64_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_u64_array(black_box(&mixed_unsigned64_wire)).unwrap())
    });
    mixed_unsigned64_decode.bench_function("cbor4ii", |b| {
        b.iter(|| {
            cbor4ii::serde::from_slice::<Vec<u64>>(black_box(&mixed_unsigned64_wire)).unwrap()
        })
    });
    mixed_unsigned64_decode.finish();

    let mut mixed_integers64 = Vec::with_capacity(4096);
    mixed_integers64.extend((0..1024).map(|value| value as i64 % 48 - 24));
    mixed_integers64.extend((0..1024).map(|value| {
        let argument = 24 + (value % 232) as i64;
        if value & 1 == 0 { argument } else { !argument }
    }));
    mixed_integers64.extend((0..1024).map(|value| {
        let argument = 256 + (value * 61) as i64;
        if value & 1 == 0 { argument } else { !argument }
    }));
    mixed_integers64.extend((0..1024).map(|value| {
        let argument = 65_536 + (value * 1_000_003) as i64;
        if value & 1 == 0 { argument } else { !argument }
    }));
    let mixed_integers64_wire = fcpw::to_vec(&mixed_integers64).unwrap();
    let mut mixed_integers64_decode = c.benchmark_group("decode/i64-mixed-width-array");
    mixed_integers64_decode.throughput(Throughput::Bytes(mixed_integers64_wire.len() as u64));
    mixed_integers64_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<i64>>(black_box(&mixed_integers64_wire)).unwrap())
    });
    mixed_integers64_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_i64_array(black_box(&mixed_integers64_wire)).unwrap())
    });
    mixed_integers64_decode.bench_function("cbor4ii", |b| {
        b.iter(|| {
            cbor4ii::serde::from_slice::<Vec<i64>>(black_box(&mixed_integers64_wire)).unwrap()
        })
    });
    mixed_integers64_decode.finish();

    let unsigned32: Vec<u32> = (0..4096).map(|value| value as u32 * 1_000_003).collect();
    let unsigned32_wire = fcpw::to_vec(&unsigned32).unwrap();
    let mut unsigned32_decode = c.benchmark_group("decode/u32-array");
    unsigned32_decode.throughput(Throughput::Bytes(unsigned32_wire.len() as u64));
    unsigned32_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<u32>>(black_box(&unsigned32_wire)).unwrap())
    });
    unsigned32_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_u32_array(black_box(&unsigned32_wire)).unwrap())
    });
    unsigned32_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<u32>>(black_box(&unsigned32_wire)).unwrap())
    });
    unsigned32_decode.finish();

    let unsigned16: Vec<u16> = (0..4096)
        .map(|value| (value as u16).wrapping_mul(17))
        .collect();
    let unsigned16_wire = fcpw::to_vec(&unsigned16).unwrap();
    let mut unsigned16_decode = c.benchmark_group("decode/u16-array");
    unsigned16_decode.throughput(Throughput::Bytes(unsigned16_wire.len() as u64));
    unsigned16_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<u16>>(black_box(&unsigned16_wire)).unwrap())
    });
    unsigned16_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_u16_array(black_box(&unsigned16_wire)).unwrap())
    });
    unsigned16_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<u16>>(black_box(&unsigned16_wire)).unwrap())
    });
    unsigned16_decode.finish();

    let integers16: Vec<i16> = (0..4096).map(|value| (value as i16 - 2048) * 15).collect();
    let integer16_wire = fcpw::to_vec(&integers16).unwrap();
    let mut integer16_decode = c.benchmark_group("decode/i16-array");
    integer16_decode.throughput(Throughput::Bytes(integer16_wire.len() as u64));
    integer16_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<i16>>(black_box(&integer16_wire)).unwrap())
    });
    integer16_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_i16_array(black_box(&integer16_wire)).unwrap())
    });
    integer16_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<i16>>(black_box(&integer16_wire)).unwrap())
    });
    integer16_decode.finish();

    let integers8: Vec<i8> = (0..4096).map(|value| value as i8).collect();
    let integer8_wire = fcpw::to_vec(&integers8).unwrap();
    let mut integer8_decode = c.benchmark_group("decode/i8-array");
    integer8_decode.throughput(Throughput::Bytes(integer8_wire.len() as u64));
    integer8_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<i8>>(black_box(&integer8_wire)).unwrap())
    });
    integer8_decode.bench_function("fcpw-bulk", |b| {
        b.iter(|| fcpw::from_slice_i8_array(black_box(&integer8_wire)).unwrap())
    });
    integer8_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<i8>>(black_box(&integer8_wire)).unwrap())
    });
    integer8_decode.finish();

    let options: Vec<Option<i32>> = (0..4096)
        .map(|value| (value % 4 != 0).then_some(value - 2048))
        .collect();
    let option_wire = fcpw::to_vec(&options).unwrap();
    let mut option_decode = c.benchmark_group("decode/option-array");
    option_decode.throughput(Throughput::Bytes(option_wire.len() as u64));
    option_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<Option<i32>>>(black_box(&option_wire)).unwrap())
    });
    option_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<Option<i32>>>(black_box(&option_wire)).unwrap())
    });
    option_decode.finish();

    let strings: Vec<String> = (0..4096)
        .map(|value| format!("s{:02}", value % 64))
        .collect();
    let string_wire = fcpw::to_vec(&strings).unwrap();
    let mut string_decode = c.benchmark_group("decode/tiny-borrowed-strings");
    string_decode.throughput(Throughput::Bytes(string_wire.len() as u64));
    string_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<Vec<&str>>(black_box(&string_wire)).unwrap())
    });
    string_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<Vec<&str>>(black_box(&string_wire)).unwrap())
    });
    string_decode.finish();

    let bytes = ByteBuf::from(
        (0usize..65_536)
            .map(|value| value.wrapping_mul(31) as u8)
            .collect::<Vec<_>>(),
    );
    let byte_wire = fcpw::to_vec(&bytes).unwrap();
    let mut byte_decode = c.benchmark_group("decode/byte-string");
    byte_decode.throughput(Throughput::Bytes(byte_wire.len() as u64));
    byte_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<ByteBuf>(black_box(&byte_wire)).unwrap())
    });
    byte_decode.bench_function("cbor4ii", |b| {
        b.iter(|| cbor4ii::serde::from_slice::<ByteBuf>(black_box(&byte_wire)).unwrap())
    });
    byte_decode.finish();

    let map: BTreeMap<String, u64> = (0..1024)
        .map(|value| (format!("key-{value:04}"), value * 1_000_003))
        .collect();
    let map_wire = fcpw::to_vec(&map).unwrap();
    let mut map_decode = c.benchmark_group("decode/string-map");
    map_decode.throughput(Throughput::Bytes(map_wire.len() as u64));
    map_decode.bench_function("fcpw", |b| {
        b.iter(|| fcpw::from_slice::<BTreeMap<String, u64>>(black_box(&map_wire)).unwrap())
    });
    map_decode.bench_function("cbor4ii", |b| {
        b.iter(|| {
            cbor4ii::serde::from_slice::<BTreeMap<String, u64>>(black_box(&map_wire)).unwrap()
        })
    });
    map_decode.finish();

    let mut deterministic = c.benchmark_group("encode/string-map");
    deterministic.throughput(Throughput::Bytes(map_wire.len() as u64));
    deterministic.bench_function("fcpw-normal", |b| {
        b.iter(|| fcpw::to_vec(black_box(&map)).unwrap())
    });
    deterministic.bench_function("fcpw-deterministic", |b| {
        b.iter(|| fcpw::to_vec_deterministic(black_box(&map)).unwrap())
    });
    let mut reusable_output = Vec::new();
    deterministic.bench_function("fcpw-deterministic-output-reuse", |b| {
        b.iter(|| fcpw::to_vec_deterministic_into(black_box(&map), &mut reusable_output).unwrap())
    });
    let mut scratch = fcpw::DeterministicScratch::new();
    deterministic.bench_function("fcpw-deterministic-full-reuse", |b| {
        b.iter(|| {
            fcpw::to_vec_deterministic_into_with_scratch(
                black_box(&map),
                &mut reusable_output,
                &mut scratch,
            )
            .unwrap()
        })
    });
    deterministic.finish();
}

fn dynamic_and_output_benchmarks(c: &mut Criterion) {
    let value = record(16);
    let wire = fcpw::to_vec(&value).unwrap();

    let tagged = fcpw::Tagged::with_tag(100, value.clone());
    let tagged_wire = fcpw::to_vec(&tagged).unwrap();
    let mut parity = c.benchmark_group("parity/typed-tag-medium");
    parity.throughput(Throughput::Bytes(tagged_wire.len() as u64));
    parity.bench_function("encode", |b| {
        b.iter(|| fcpw::to_vec(black_box(&tagged)).unwrap())
    });
    parity.bench_function("decode", |b| {
        b.iter(|| fcpw::from_slice::<fcpw::Tagged<Record>>(black_box(&tagged_wire)).unwrap())
    });
    parity.finish();

    let mut packed = c.benchmark_group("parity/packed-medium");
    packed.bench_function("fcpw", |b| {
        b.iter(|| fcpw::to_vec_packed(black_box(&value)).unwrap())
    });
    packed.bench_function("serde-cbor", |b| {
        b.iter(|| serde_cbor::ser::to_vec_packed(black_box(&value)).unwrap())
    });
    packed.finish();

    let mut dynamic = c.benchmark_group("decode/dynamic-medium");
    dynamic.throughput(Throughput::Bytes(wire.len() as u64));
    dynamic.bench_function("fcpw-borrowed", |b| {
        b.iter(|| fcpw::BorrowedValue::decode(black_box(&wire)).unwrap())
    });
    dynamic.bench_function("fcpw-owned", |b| {
        b.iter(|| fcpw::from_slice_value(black_box(&wire)).unwrap())
    });
    dynamic.finish();

    let mut output = vec![0u8; wire.len()];
    let mut outputs = c.benchmark_group("encode/output-medium");
    outputs.throughput(Throughput::Bytes(wire.len() as u64));
    outputs.bench_function("to-vec", |b| {
        b.iter(|| fcpw::to_vec(black_box(&value)).unwrap())
    });
    outputs.bench_function("to-slice", |b| {
        b.iter(|| {
            fcpw::to_slice(black_box(&value), black_box(&mut output))
                .unwrap()
                .len()
        })
    });
    outputs.bench_function("serialized-size", |b| {
        b.iter(|| fcpw::serialized_size(black_box(&value)).unwrap())
    });
    outputs.bench_function("to-writer", |b| {
        b.iter(|| fcpw::to_writer(std::io::sink(), black_box(&value)).unwrap())
    });
    outputs.finish();

    let scalar_sequence: Vec<u8> = (0..65_536).map(|value| (value % 24) as u8).collect();
    let mut sequence = c.benchmark_group("sequence/boundaries");
    sequence.throughput(Throughput::Bytes(scalar_sequence.len() as u64));
    sequence.bench_function("fcpw", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for item in fcpw::SequenceDecoder::new(black_box(&scalar_sequence)) {
                total += item.unwrap().as_bytes().len();
            }
            total
        })
    });
    sequence.finish();

    let mut reader_sequence = c.benchmark_group("sequence/reader-scalars");
    reader_sequence.throughput(Throughput::Bytes(scalar_sequence.len() as u64));
    reader_sequence.bench_function("fcpw", |b| {
        b.iter(|| {
            let mut reader = fcpw::ReaderDeserializer::new(black_box(scalar_sequence.as_slice()));
            let mut total = 0u64;
            while let Some(value) = reader.deserialize_next::<u64>().unwrap() {
                total += value;
            }
            total
        })
    });
    reader_sequence.finish();

    #[cfg(feature = "parallel")]
    {
        let item = fcpw::to_vec(&record(16)).unwrap();
        let items = vec![item; 256];
        let slices: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
        let sequence_wire = items.concat();
        let mut parallel = c.benchmark_group("sequence/large-records");
        parallel.throughput(Throughput::Bytes(sequence_wire.len() as u64));
        parallel.bench_function("sequential", |b| {
            b.iter(|| {
                fcpw::SequenceDecoder::new(black_box(&sequence_wire))
                    .map(|item| item.and_then(|raw| fcpw::from_slice::<Record>(raw.as_bytes())))
                    .collect::<fcpw::Result<Vec<_>>>()
                    .unwrap()
            })
        });
        parallel.bench_function("parallel-boundaries", |b| {
            b.iter(|| fcpw::parallel::from_sequence::<Record>(black_box(&sequence_wire)).unwrap())
        });
        parallel.bench_function("parallel-slices", |b| {
            b.iter(|| fcpw::parallel::from_slices::<Record>(black_box(&slices)).unwrap())
        });
        parallel.finish();
    }
}

criterion_group!(
    benches,
    codec_benchmarks,
    validation_benchmarks,
    focused_benchmarks,
    scalar_and_collection_benchmarks,
    dynamic_and_output_benchmarks
);
criterion_main!(benches);
