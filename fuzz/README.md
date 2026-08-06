# Fuzzing the encoders

Install `cargo-fuzz` and a nightly Rust toolchain, then run either target from
the repository root:

```console
cargo +nightly fuzz run low_level_encoder
cargo +nightly fuzz run serde_encoder
```

For a finite local or CI smoke run:

```console
cargo +nightly fuzz run low_level_encoder -- -runs=10000
cargo +nightly fuzz run serde_encoder -- -runs=10000
```

`low_level_encoder` generates bounded recursive CBOR values and checks dynamic
round trips, validation, fixed-slice output, undersized-output errors, and
preferred float encoding. `serde_encoder` checks all public Serde output modes,
size calculation, deterministic encoding, and semantic cross-decoding with
`serde_cbor`.

Crashes and minimized reproducers are written below `fuzz/artifacts/`. Generated
corpora and artifacts are intentionally ignored by Git.
