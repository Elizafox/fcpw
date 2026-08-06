use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeMap,
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

struct CountingAllocator;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(size, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

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

#[derive(Debug, Deserialize)]
struct KeepOnly {
    keep: Vec<i32>,
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

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("fcpw-decode"));
    let iterations: usize = std::env::args()
        .nth(2)
        .as_deref()
        .unwrap_or("100000")
        .parse()
        .unwrap();
    let value = record(16);
    let integers: Vec<i32> = (0..4096).map(|value| value - 2048).collect();
    let booleans: Vec<bool> = (0..4096).map(|value| value % 3 != 0).collect();
    let signed8: Vec<i8> = (0..4096).map(|value| value as i8).collect();
    let unsigned8: Vec<u8> = (0..4096).map(|value| value as u8).collect();
    let options: Vec<Option<i32>> = (0..4096)
        .map(|value| (value % 4 != 0).then_some(value - 2048))
        .collect();
    let tiny_strings: Vec<String> = (0..4096)
        .map(|value| format!("s{:02}", value % 64))
        .collect();
    let characters: Vec<char> = (0..4096)
        .map(|value| match value % 4 {
            0 => 'a',
            1 => 'ß',
            2 => '水',
            _ => '🦀',
        })
        .collect();
    let long_text = "the quick brown fox jumps over the lazy dog 0123456789 ".repeat(1280);
    let signed16: Vec<i16> = (0..4096).map(|value| value - 2048).collect();
    let unsigned16: Vec<u16> = (0..4096).map(|value| value * 13).collect();
    let unsigned32: Vec<u32> = (0..4096).map(|value| value * 1_000_003).collect();
    let signed64: Vec<i64> = (0..4096)
        .map(|value| (value as i64 - 2048) * 1_000_003)
        .collect();
    let unsigned64: Vec<u64> = (0..4096).map(|value| value as u64 * 1_000_003).collect();
    let byte_string = ByteBuf::from(
        (0usize..65_536)
            .map(|value| value.wrapping_mul(31) as u8)
            .collect::<Vec<_>>(),
    );
    let floats: Vec<f64> = if mode.contains("float-half") {
        (0..4096)
            .map(|value| (value as f64 - 2048.0) * 0.5)
            .collect()
    } else if mode.contains("float32") {
        (0..4096)
            .map(|value| f64::from(value as f32 * 0.1 + 0.03))
            .collect()
    } else if mode.contains("float-special") {
        (0..4096)
            .map(|value| match value % 8 {
                0 => f64::NAN,
                1 => f64::INFINITY,
                2 => f64::NEG_INFINITY,
                3 => 0.0,
                4 => -0.0,
                5 => f64::MIN_POSITIVE,
                6 => f64::MAX,
                _ => f64::from_bits(1),
            })
            .collect()
    } else {
        (0..4096).map(|value| value as f64 * 0.1 + 0.03).collect()
    };
    let map: BTreeMap<String, u64> = (0..1024)
        .map(|value| (format!("key-{value:04}"), value * 1_000_003))
        .collect();
    let mut ignored_map: BTreeMap<String, Vec<i32>> = (0..64)
        .map(|field| {
            (
                format!("extra-{field:02}"),
                (0..64).map(|value| value - 32).collect(),
            )
        })
        .collect();
    ignored_map.insert(String::from("keep"), vec![1, 2, 3, 4]);
    let sequence_item = fcpw::to_vec(&record(1)).unwrap();
    let sequence_items = vec![sequence_item; 1024];
    #[cfg(feature = "parallel")]
    let sequence_slices: Vec<&[u8]> = sequence_items.iter().map(Vec::as_slice).collect();
    let sequence: Vec<u8> = sequence_items.concat();
    let large_sequence_item = fcpw::to_vec(&record(16)).unwrap();
    let large_sequence_items = vec![large_sequence_item; 256];
    #[cfg(feature = "parallel")]
    let large_sequence_slices: Vec<&[u8]> =
        large_sequence_items.iter().map(Vec::as_slice).collect();
    let large_sequence: Vec<u8> = large_sequence_items.concat();
    let scalar_sequence: Vec<u8> = (0..65_536).map(|value| (value % 24) as u8).collect();
    let mut output_storage = vec![0u8; 65_536];
    let bytes = if mode.contains("bool") {
        fcpw::to_vec(&booleans).unwrap()
    } else if mode.contains("unsigned8") {
        fcpw::to_vec(&unsigned8).unwrap()
    } else if mode.contains("signed8") {
        fcpw::to_vec(&signed8).unwrap()
    } else if mode.contains("option") {
        fcpw::to_vec(&options).unwrap()
    } else if mode.contains("tiny-string") {
        fcpw::to_vec(&tiny_strings).unwrap()
    } else if mode.contains("char") {
        fcpw::to_vec(&characters).unwrap()
    } else if mode.contains("text") {
        fcpw::to_vec(&long_text).unwrap()
    } else if mode.contains("ignored") {
        fcpw::to_vec(&ignored_map).unwrap()
    } else if mode.contains("map") {
        fcpw::to_vec(&map).unwrap()
    } else if mode.contains("bytes") {
        fcpw::to_vec(&byte_string).unwrap()
    } else if mode.contains("float") {
        if mode.contains("-decode") {
            fcpw::to_vec_deterministic(&floats).unwrap()
        } else {
            fcpw::to_vec(&floats).unwrap()
        }
    } else if mode.contains("unsigned16") {
        fcpw::to_vec(&unsigned16).unwrap()
    } else if mode.contains("signed16") {
        fcpw::to_vec(&signed16).unwrap()
    } else if mode.contains("unsigned32") {
        fcpw::to_vec(&unsigned32).unwrap()
    } else if mode.contains("unsigned64") {
        fcpw::to_vec(&unsigned64).unwrap()
    } else if mode.contains("signed64") {
        fcpw::to_vec(&signed64).unwrap()
    } else if mode.contains("integer") {
        fcpw::to_vec(&integers).unwrap()
    } else {
        fcpw::to_vec(&value).unwrap()
    };
    let mut reusable_output = Vec::with_capacity(bytes.len());
    let mut reusable_reader_input = Vec::with_capacity(bytes.len().max(8 * 1024));
    let mut reusable_i32_decode = Vec::with_capacity(integers.len());
    let mut reusable_f64_decode = Vec::with_capacity(floats.len());
    let mut deterministic_scratch = fcpw::DeterministicScratch::new();
    if mode == "fcpw-map-deterministic-full-reuse-encode" {
        fcpw::to_vec_deterministic_into_with_scratch(
            &map,
            &mut reusable_output,
            &mut deterministic_scratch,
        )
        .unwrap();
    }

    ALLOCATIONS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    let start = Instant::now();
    for _ in 0..iterations {
        match mode.as_str() {
            "fcpw-decode" => {
                black_box(fcpw::from_slice::<Record>(black_box(&bytes)).unwrap());
            }
            "fcpw-reader-decode" => {
                black_box(fcpw::from_reader::<Record, _>(black_box(bytes.as_slice())).unwrap());
            }
            "fcpw-reader-reuse-decode" => {
                black_box(
                    fcpw::from_reader_with_buffer::<Record, _>(
                        black_box(bytes.as_slice()),
                        black_box(&mut reusable_reader_input),
                    )
                    .unwrap(),
                );
            }
            "fcpw-validate" => {
                fcpw::validate(black_box(&bytes)).unwrap();
            }
            "cbor4ii-decode" => {
                black_box(cbor4ii::serde::from_slice::<Record>(black_box(&bytes)).unwrap());
            }
            "fcpw-borrowed-value-decode" => {
                black_box(fcpw::BorrowedValue::decode(black_box(&bytes)).unwrap());
            }
            "fcpw-owned-value-decode" => {
                black_box(fcpw::from_slice_value(black_box(&bytes)).unwrap());
            }
            "fcpw-encode" => {
                black_box(fcpw::to_vec(black_box(&value)).unwrap());
            }
            "fcpw-reuse-encode" => {
                fcpw::to_vec_into(black_box(&value), &mut reusable_output).unwrap();
                black_box(reusable_output.as_slice());
            }
            "cbor4ii-encode" => {
                black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&value)).unwrap());
            }
            "fcpw-to-slice" => {
                black_box(
                    fcpw::to_slice(black_box(&value), black_box(&mut output_storage)).unwrap(),
                );
            }
            "fcpw-serialized-size" => {
                black_box(fcpw::serialized_size(black_box(&value)).unwrap());
            }
            "fcpw-to-writer" => {
                fcpw::to_writer(std::io::sink(), black_box(&value)).unwrap();
            }
            "fcpw-integer-decode" => {
                black_box(fcpw::from_slice::<Vec<i32>>(black_box(&bytes)).unwrap());
            }
            "fcpw-integer-bulk-decode" => {
                black_box(fcpw::from_slice_i32_array(black_box(&bytes)).unwrap());
            }
            "fcpw-integer-reuse-decode" => {
                fcpw::from_slice_i32_array_into(
                    black_box(&bytes),
                    black_box(&mut reusable_i32_decode),
                )
                .unwrap();
                black_box(reusable_i32_decode.as_slice());
            }
            "cbor4ii-integer-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<i32>>(black_box(&bytes)).unwrap());
            }
            "fcpw-bool-decode" => {
                black_box(fcpw::from_slice::<Vec<bool>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-bool-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<bool>>(black_box(&bytes)).unwrap());
            }
            "fcpw-signed8-decode" => {
                black_box(fcpw::from_slice::<Vec<i8>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-signed8-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<i8>>(black_box(&bytes)).unwrap());
            }
            "fcpw-unsigned8-decode" => {
                black_box(fcpw::from_slice::<Vec<u8>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-unsigned8-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<u8>>(black_box(&bytes)).unwrap());
            }
            "fcpw-option-decode" => {
                black_box(fcpw::from_slice::<Vec<Option<i32>>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-option-decode" => {
                black_box(
                    cbor4ii::serde::from_slice::<Vec<Option<i32>>>(black_box(&bytes)).unwrap(),
                );
            }
            "fcpw-tiny-string-decode" => {
                black_box(fcpw::from_slice::<Vec<&str>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-tiny-string-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<&str>>(black_box(&bytes)).unwrap());
            }
            "fcpw-char-decode" => {
                black_box(fcpw::from_slice::<Vec<char>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-char-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<char>>(black_box(&bytes)).unwrap());
            }
            "fcpw-text-validate" => {
                fcpw::validate(black_box(&bytes)).unwrap();
            }
            "fcpw-text-decode" => {
                black_box(fcpw::from_slice::<&str>(black_box(&bytes)).unwrap());
            }
            "fcpw-ignored-decode" => {
                let decoded = fcpw::from_slice::<KeepOnly>(black_box(&bytes)).unwrap();
                black_box(decoded.keep);
            }
            "cbor4ii-ignored-decode" => {
                let decoded = cbor4ii::serde::from_slice::<KeepOnly>(black_box(&bytes)).unwrap();
                black_box(decoded.keep);
            }
            "fcpw-signed16-decode" => {
                black_box(fcpw::from_slice::<Vec<i16>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-signed16-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<i16>>(black_box(&bytes)).unwrap());
            }
            "fcpw-unsigned16-decode" => {
                black_box(fcpw::from_slice::<Vec<u16>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-unsigned16-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<u16>>(black_box(&bytes)).unwrap());
            }
            "fcpw-unsigned32-decode" => {
                black_box(fcpw::from_slice::<Vec<u32>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-unsigned32-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<u32>>(black_box(&bytes)).unwrap());
            }
            "fcpw-integer-encode" => {
                black_box(fcpw::to_vec(black_box(&integers)).unwrap());
            }
            "fcpw-integer-reuse-encode" => {
                fcpw::to_vec_into(black_box(&integers), &mut reusable_output).unwrap();
                black_box(reusable_output.as_slice());
            }
            "cbor4ii-integer-encode" => {
                black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&integers)).unwrap());
            }
            "fcpw-signed64-encode" => {
                black_box(fcpw::to_vec(black_box(&signed64)).unwrap());
            }
            "cbor4ii-signed64-encode" => {
                black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&signed64)).unwrap());
            }
            "fcpw-unsigned64-encode" => {
                black_box(fcpw::to_vec(black_box(&unsigned64)).unwrap());
            }
            "cbor4ii-unsigned64-encode" => {
                black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&unsigned64)).unwrap());
            }
            "fcpw-signed64-decode" => {
                black_box(fcpw::from_slice::<Vec<i64>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-signed64-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<i64>>(black_box(&bytes)).unwrap());
            }
            "fcpw-unsigned64-decode" => {
                black_box(fcpw::from_slice::<Vec<u64>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-unsigned64-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<u64>>(black_box(&bytes)).unwrap());
            }
            "fcpw-bytes-encode" => {
                black_box(fcpw::to_vec(black_box(&byte_string)).unwrap());
            }
            "cbor4ii-bytes-encode" => {
                black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&byte_string)).unwrap());
            }
            "fcpw-bytes-decode" => {
                black_box(fcpw::from_slice::<ByteBuf>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-bytes-decode" => {
                black_box(cbor4ii::serde::from_slice::<ByteBuf>(black_box(&bytes)).unwrap());
            }
            "fcpw-float-encode"
            | "fcpw-float-half-encode"
            | "fcpw-float32-encode"
            | "fcpw-float-special-encode" => {
                black_box(fcpw::to_vec(black_box(&floats)).unwrap());
            }
            "fcpw-float-reuse-encode" => {
                fcpw::to_vec_into(black_box(&floats), &mut reusable_output).unwrap();
                black_box(reusable_output.as_slice());
            }
            "cbor4ii-float-encode"
            | "cbor4ii-float-half-encode"
            | "cbor4ii-float32-encode"
            | "cbor4ii-float-special-encode" => {
                black_box(cbor4ii::serde::to_vec(Vec::new(), black_box(&floats)).unwrap());
            }
            "fcpw-float-decode"
            | "fcpw-float-half-decode"
            | "fcpw-float32-decode"
            | "fcpw-float-special-decode" => {
                black_box(fcpw::from_slice::<Vec<f64>>(black_box(&bytes)).unwrap());
            }
            "fcpw-float-reuse-decode" => {
                fcpw::from_slice_f64_array_into(
                    black_box(&bytes),
                    black_box(&mut reusable_f64_decode),
                )
                .unwrap();
                black_box(reusable_f64_decode.as_slice());
            }
            "fcpw-float-bulk-decode" => {
                black_box(fcpw::from_slice_f64_array(black_box(&bytes)).unwrap());
            }
            "cbor4ii-float-decode"
            | "cbor4ii-float-half-decode"
            | "cbor4ii-float32-decode"
            | "cbor4ii-float-special-decode" => {
                black_box(cbor4ii::serde::from_slice::<Vec<f64>>(black_box(&bytes)).unwrap());
            }
            "fcpw-map-normal-encode" => {
                black_box(fcpw::to_vec(black_box(&map)).unwrap());
            }
            "fcpw-map-normal-reuse-encode" => {
                fcpw::to_vec_into(black_box(&map), &mut reusable_output).unwrap();
                black_box(reusable_output.as_slice());
            }
            "fcpw-map-deterministic-encode" => {
                black_box(fcpw::to_vec_deterministic(black_box(&map)).unwrap());
            }
            "fcpw-map-deterministic-reuse-encode" => {
                fcpw::to_vec_deterministic_into(black_box(&map), &mut reusable_output).unwrap();
                black_box(reusable_output.as_slice());
            }
            "fcpw-map-deterministic-full-reuse-encode" => {
                fcpw::to_vec_deterministic_into_with_scratch(
                    black_box(&map),
                    &mut reusable_output,
                    &mut deterministic_scratch,
                )
                .unwrap();
                black_box(reusable_output.as_slice());
            }
            "fcpw-map-decode" => {
                black_box(fcpw::from_slice::<BTreeMap<String, u64>>(black_box(&bytes)).unwrap());
            }
            "cbor4ii-map-decode" => {
                black_box(
                    cbor4ii::serde::from_slice::<BTreeMap<String, u64>>(black_box(&bytes)).unwrap(),
                );
            }
            "fcpw-sequence-decode" => {
                let decoded: fcpw::Result<Vec<Record>> =
                    fcpw::SequenceDecoder::new(black_box(&sequence))
                        .map(|item| item.and_then(|raw| fcpw::from_slice(raw.as_bytes())))
                        .collect();
                black_box(decoded.unwrap());
            }
            "fcpw-sequence-boundaries" => {
                let mut total = 0usize;
                for item in fcpw::SequenceDecoder::new(black_box(&scalar_sequence)) {
                    total += item.unwrap().as_bytes().len();
                }
                black_box(total);
            }
            #[cfg(feature = "parallel")]
            "fcpw-parallel-sequence-decode" => {
                black_box(fcpw::parallel::from_sequence::<Record>(black_box(&sequence)).unwrap());
            }
            #[cfg(feature = "parallel")]
            "fcpw-parallel-slices-decode" => {
                black_box(
                    fcpw::parallel::from_slices::<Record>(black_box(&sequence_slices)).unwrap(),
                );
            }
            "fcpw-large-sequence-decode" => {
                let decoded: fcpw::Result<Vec<Record>> =
                    fcpw::SequenceDecoder::new(black_box(&large_sequence))
                        .map(|item| item.and_then(|raw| fcpw::from_slice(raw.as_bytes())))
                        .collect();
                black_box(decoded.unwrap());
            }
            #[cfg(feature = "parallel")]
            "fcpw-parallel-large-sequence-decode" => {
                black_box(
                    fcpw::parallel::from_sequence::<Record>(black_box(&large_sequence)).unwrap(),
                );
            }
            #[cfg(feature = "parallel")]
            "fcpw-parallel-large-slices-decode" => {
                black_box(
                    fcpw::parallel::from_slices::<Record>(black_box(&large_sequence_slices))
                        .unwrap(),
                );
            }
            #[cfg(feature = "parallel")]
            "fcpw-parallel-large-structural" => {
                black_box(
                    fcpw::parallel::from_sequence::<serde::de::IgnoredAny>(black_box(
                        &large_sequence,
                    ))
                    .unwrap(),
                );
            }
            #[cfg(feature = "parallel")]
            "fcpw-parallel-large-structural-slices" => {
                black_box(
                    fcpw::parallel::from_slices::<serde::de::IgnoredAny>(black_box(
                        &large_sequence_slices,
                    ))
                    .unwrap(),
                );
            }
            _ => panic!("unknown mode"),
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{mode}: {:.2} ns/item, {:.2} allocations/item, {:.1} allocated bytes/item",
        elapsed.as_nanos() as f64 / iterations as f64,
        ALLOCATIONS.load(Ordering::Relaxed) as f64 / iterations as f64,
        ALLOCATED_BYTES.load(Ordering::Relaxed) as f64 / iterations as f64,
    );
}
