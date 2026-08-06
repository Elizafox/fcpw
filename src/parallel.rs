//! Parallel decoding for independent CBOR items.

use alloc::vec::Vec;
use rayon::prelude::*;
use serde::de::DeserializeOwned;
use std::sync::mpsc;

use crate::{RawValue, Result, SliceDecoder, from_slice};

#[derive(Clone, Copy, Debug)]
/// Thresholds controlling when parallel decoding begins.
pub struct ParallelOptions {
    /// Inputs below this total byte count are decoded sequentially.
    pub min_bytes: usize,
    /// Batches below this item count are decoded sequentially.
    pub min_items: usize,
}

impl Default for ParallelOptions {
    fn default() -> Self {
        Self {
            min_bytes: 1024 * 1024,
            min_items: 32,
        }
    }
}

/// Decodes every item in a CBOR sequence, using default parallel thresholds.
pub fn from_sequence<T: DeserializeOwned + Send>(input: &[u8]) -> Result<Vec<T>> {
    from_sequence_with_options(input, ParallelOptions::default())
}

/// Decodes every item in a CBOR sequence with explicit parallel thresholds.
///
/// Results retain input order. Errors include the zero-based sequence item index.
pub fn from_sequence_with_options<T: DeserializeOwned + Send>(
    input: &[u8],
    options: ParallelOptions,
) -> Result<Vec<T>> {
    let mut decoder = SliceDecoder::new(input);
    let mut items = Vec::new();
    // Establish the item threshold before starting work with observable
    // deserializer side effects.
    while decoder.position() != input.len()
        && (input.len() < options.min_bytes || items.len() < options.min_items)
    {
        let index = items.len();
        items.push(
            decoder
                .raw_structural()
                .map_err(|error| error.with_item(index))?,
        );
    }
    if input.len() < options.min_bytes || items.len() < options.min_items {
        decode_raw(&items, input.len(), options)
    } else {
        decode_raw_pipelined(decoder, items)
    }
}

/// Decodes independent CBOR slices, using default parallel thresholds.
pub fn from_slices<T: DeserializeOwned + Send>(inputs: &[&[u8]]) -> Result<Vec<T>> {
    from_slices_with_options(inputs, ParallelOptions::default())
}

/// Decodes independent CBOR slices with explicit parallel thresholds.
///
/// Results retain input order. Errors include the zero-based input index.
pub fn from_slices_with_options<T: DeserializeOwned + Send>(
    inputs: &[&[u8]],
    options: ParallelOptions,
) -> Result<Vec<T>> {
    let total: usize = inputs.iter().map(|input| input.len()).sum();
    if inputs.len() < options.min_items || total < options.min_bytes {
        inputs
            .iter()
            .enumerate()
            .map(|(index, input)| from_slice(input).map_err(|error| error.with_item(index)))
            .collect()
    } else {
        inputs
            .par_iter()
            .enumerate()
            .map(|(index, input)| from_slice(input).map_err(|error| error.with_item(index)))
            .collect()
    }
}

fn decode_raw<T: DeserializeOwned + Send>(
    items: &[RawValue<'_>],
    total: usize,
    options: ParallelOptions,
) -> Result<Vec<T>> {
    if items.len() < options.min_items || total < options.min_bytes {
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                from_slice(item.as_bytes()).map_err(|error| error.with_item(index))
            })
            .collect()
    } else {
        items
            .par_iter()
            .enumerate()
            .map(|(index, item)| {
                from_slice(item.as_bytes()).map_err(|error| error.with_item(index))
            })
            .collect()
    }
}

fn decode_raw_pipelined<T: DeserializeOwned + Send>(
    mut decoder: SliceDecoder<'_>,
    initial: Vec<RawValue<'_>>,
) -> Result<Vec<T>> {
    const CHUNK_ITEMS: usize = 8;

    let mut initial = initial.into_iter();
    let mut item_count = 0usize;
    let mut boundary_error = None;
    let (sender, receiver) = mpsc::channel::<(usize, Result<Vec<T>>)>();

    rayon::scope_fifo(|scope| {
        loop {
            let start = item_count;
            let mut chunk = Vec::with_capacity(CHUNK_ITEMS);
            while chunk.len() < CHUNK_ITEMS {
                if let Some(item) = initial.next() {
                    chunk.push(item);
                    item_count += 1;
                    continue;
                }
                if decoder.remaining().is_empty() {
                    break;
                }
                let index = item_count;
                match decoder
                    .raw_structural()
                    .map_err(|error| error.with_item(index))
                {
                    Ok(item) => {
                        chunk.push(item);
                        item_count += 1;
                    }
                    Err(error) => {
                        boundary_error = Some(error);
                        break;
                    }
                }
            }
            if boundary_error.is_some() || chunk.is_empty() {
                break;
            }
            let sender = sender.clone();
            scope.spawn_fifo(move |_| {
                let values = chunk
                    .iter()
                    .enumerate()
                    .map(|(offset, item)| {
                        from_slice(item.as_bytes()).map_err(|error| error.with_item(start + offset))
                    })
                    .collect();
                let _ = sender.send((start, values));
            });
        }
    });
    drop(sender);

    // Preserve the boundary-first error precedence of the non-pipelined path.
    if let Some(error) = boundary_error {
        return Err(error);
    }
    let mut chunks: Vec<_> = receiver.into_iter().collect();
    // FIFO scheduling is only a preference; restore exact wire order here.
    chunks.sort_unstable_by_key(|(start, _)| *start);
    let mut output = Vec::with_capacity(item_count);
    for (_, values) in chunks {
        output.append(&mut values?);
    }
    Ok(output)
}
