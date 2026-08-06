use std::{hint::black_box, time::Duration};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    id: u64,
    timestamp: i64,
    name: String,
    active: bool,
    scores: Vec<i32>,
    metadata: Vec<(String, String)>,
    payload: Vec<u8>,
}

fn record(scale: usize) -> Record {
    Record {
        id: 9_007_199_254_740_991,
        timestamp: -1_725_000_000,
        name: "variation-tolerant CBOR benchmark record".repeat(scale),
        active: true,
        scores: (0..scale * 32).map(|value| value as i32 - 50).collect(),
        metadata: (0..scale * 4)
            .map(|value| (format!("key-{value:03}"), format!("value-{value:03}")))
            .collect(),
        payload: (0..scale * 256)
            .map(|value| value.wrapping_mul(31) as u8)
            .collect(),
    }
}

fn benchmarks(c: &mut Criterion) {
    let small = record(1);
    let small_wire = fcpw::to_vec(&small).unwrap();
    let medium = record(16);
    let medium_wire = fcpw::to_vec(&medium).unwrap();
    let integers: Vec<i32> = (0..4096).map(|value| value - 2048).collect();
    let integer_wire = fcpw::to_vec(&integers).unwrap();

    let mut encode = c.benchmark_group("encode");
    encode.bench_function("small-record", |b| {
        b.iter(|| fcpw::to_vec(black_box(&small)).unwrap())
    });
    encode.bench_function("medium-record", |b| {
        b.iter(|| fcpw::to_vec(black_box(&medium)).unwrap())
    });
    encode.bench_function("integer-array", |b| {
        b.iter(|| fcpw::to_vec(black_box(&integers)).unwrap())
    });
    encode.finish();

    let mut decode = c.benchmark_group("decode");
    decode.throughput(Throughput::Bytes(medium_wire.len() as u64));
    decode.bench_function("medium-record", |b| {
        b.iter(|| fcpw::from_slice::<Record>(black_box(&medium_wire)).unwrap())
    });
    decode.finish();

    let mut integers_decode = c.benchmark_group("decode-integer-array");
    integers_decode.throughput(Throughput::Bytes(integer_wire.len() as u64));
    integers_decode.bench_function("serde", |b| {
        b.iter(|| fcpw::from_slice::<Vec<i32>>(black_box(&integer_wire)).unwrap())
    });
    integers_decode.bench_function("bulk", |b| {
        b.iter(|| fcpw::from_slice_i32_array(black_box(&integer_wire)).unwrap())
    });
    integers_decode.finish();

    let mut validate = c.benchmark_group("validate");
    validate.throughput(Throughput::Bytes(small_wire.len() as u64));
    validate.bench_function("small-record", |b| {
        b.iter(|| fcpw::validate(black_box(&small_wire)).unwrap())
    });
    validate.finish();
}

fn configuration() -> Criterion {
    Criterion::default()
        .sample_size(60)
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .confidence_level(0.95)
        .significance_level(0.05)
        // Hosted runners are noisy. Gate only on a large change backed by
        // Criterion's statistical comparison, not on small timing movement.
        .noise_threshold(0.15)
}

criterion_group! {
    name = performance;
    config = configuration();
    targets = benchmarks
}
criterion_main!(performance);
