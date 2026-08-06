use fcpw::{
    DecodeOptions, Encoder, Error, ErrorKind, Event, Output, Parser, SliceDecoder, SliceOutput,
    Validation, validate, validate_deterministic,
};

fn encoded_unsigned(value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    Encoder::new(&mut bytes).unsigned(value).unwrap();
    bytes
}

#[test]
fn every_unsigned_argument_width_transition_round_trips() {
    let cases: &[(u64, &[u8])] = &[
        (0, &[0x00]),
        (23, &[0x17]),
        (24, &[0x18, 0x18]),
        (255, &[0x18, 0xff]),
        (256, &[0x19, 0x01, 0x00]),
        (65_535, &[0x19, 0xff, 0xff]),
        (65_536, &[0x1a, 0x00, 0x01, 0x00, 0x00]),
        (u32::MAX as u64, &[0x1a, 0xff, 0xff, 0xff, 0xff]),
        (u32::MAX as u64 + 1, &[0x1b, 0, 0, 0, 1, 0, 0, 0, 0]),
        (
            u64::MAX,
            &[0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ),
    ];

    for &(value, expected) in cases {
        let bytes = encoded_unsigned(value);
        assert_eq!(bytes, expected, "encoding {value}");
        let mut decoder = SliceDecoder::new(&bytes);
        assert_eq!(decoder.unsigned().unwrap(), value);
        decoder.finish().unwrap();
        validate_deterministic(&bytes).unwrap();
    }
}

#[test]
fn negative_argument_transitions_and_integer_extremes_round_trip() {
    for value in [
        -1i64,
        -24,
        -25,
        -256,
        -257,
        -65_536,
        -65_537,
        i32::MIN as i64,
        i64::MIN,
    ] {
        let mut bytes = Vec::new();
        Encoder::new(&mut bytes).signed(value).unwrap();
        let mut decoder = SliceDecoder::new(&bytes);
        assert_eq!(decoder.integer().unwrap(), value as i128);
        decoder.finish().unwrap();
        validate_deterministic(&bytes).unwrap();
    }

    let mut bytes = Vec::new();
    Encoder::new(&mut bytes)
        .integer(-(u64::MAX as i128) - 1)
        .unwrap();
    assert_eq!(
        SliceDecoder::new(&bytes).integer().unwrap(),
        -(u64::MAX as i128) - 1
    );

    for value in [u64::MAX as i128 + 1, -(u64::MAX as i128) - 2] {
        let mut output = Vec::new();
        let error = Encoder::new(&mut output).integer(value).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::IntegerOverflow);
        assert!(output.is_empty());
    }
}

#[test]
fn reserved_additional_information_is_rejected_for_every_major_type() {
    for major in 0u8..=7 {
        for additional in 28u8..=30 {
            let byte = major << 5 | additional;
            let error = validate(&[byte]).unwrap_err();
            assert_eq!(
                error.kind(),
                ErrorKind::InvalidAdditionalInfo,
                "initial {byte:#04x}"
            );
            assert_eq!(error.offset(), 0);
        }
    }
}

#[test]
fn truncation_at_every_boundary_reports_eof_without_panicking() {
    // The outer definite array remains incomplete until the final byte, so every
    // proper prefix must be rejected even when an inner prefix is a valid item.
    let complete = [
        0x83, 0x1b, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x64, b'C', b'B', b'O', b'R',
        0x82, 0xf5, 0xf6,
    ];
    for end in 0..complete.len() {
        let error = validate(&complete[..end]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Eof, "prefix length {end}");
        assert!(error.offset() <= end);
    }
    validate(&complete).unwrap();
}

#[test]
fn malformed_indefinite_forms_cover_each_grammar_rule() {
    let cases: &[(&[u8], ErrorKind, usize)] = &[
        (&[0x1f], ErrorKind::InvalidAdditionalInfo, 0),
        (&[0x3f], ErrorKind::InvalidAdditionalInfo, 0),
        (&[0xdf, 0x00], ErrorKind::InvalidAdditionalInfo, 0),
        (&[0x5f], ErrorKind::Eof, 1),
        (&[0x7f, 0x61, b'a'], ErrorKind::Eof, 3),
        (&[0x5f, 0x41, 0, 0x7f, 0xff], ErrorKind::UnexpectedType, 3),
        (&[0x7f, 0x7f, 0xff, 0xff], ErrorKind::UnexpectedType, 1),
        (&[0x9f, 0x01], ErrorKind::Eof, 2),
        (&[0xbf, 0x01, 0xff], ErrorKind::UnexpectedBreak, 2),
        (&[0x81, 0xff], ErrorKind::UnexpectedBreak, 1),
        (&[0xff], ErrorKind::UnexpectedBreak, 0),
    ];
    for &(bytes, kind, offset) in cases {
        let error = validate(bytes).unwrap_err();
        assert_eq!(error.kind(), kind, "{bytes:02x?}");
        assert_eq!(error.offset(), offset, "{bytes:02x?}");
    }
}

#[test]
fn utf8_accepts_scalar_boundaries_and_rejects_invalid_sequences() {
    for text in [
        "\0",
        "\u{7f}",
        "\u{80}",
        "\u{7ff}",
        "\u{800}",
        "\u{ffff}",
        "\u{10000}",
        "\u{10ffff}",
    ] {
        let mut bytes = Vec::new();
        Encoder::new(&mut bytes).text(text).unwrap();
        let mut decoder = SliceDecoder::new(&bytes);
        assert_eq!(decoder.text().unwrap(), text);
        decoder.finish().unwrap();
    }

    for bytes in [
        &[0x61, 0x80][..],               // lone continuation
        &[0x62, 0xc0, 0x80],             // overlong NUL
        &[0x63, 0xed, 0xa0, 0x80],       // surrogate
        &[0x64, 0xf4, 0x90, 0x80, 0x80], // above U+10ffff
        &[0x63, 0xe2, 0x82, 0x20],       // bad continuation
    ] {
        assert_eq!(validate(bytes).unwrap_err().kind(), ErrorKind::InvalidUtf8);
    }
}

#[test]
fn parser_emits_the_complete_indefinite_event_stream() {
    let bytes = [
        0x9f, 0x01, 0x20, 0x5f, 0x42, 1, 2, 0xff, 0x7f, 0x61, b'x', 0xff, 0xbf, 0x01, 0xf5, 0xff,
        0xd8, 42, 0xf6, 0xff,
    ];
    let events: Vec<_> = Parser::new(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(
        events,
        [
            Event::Array(None),
            Event::Unsigned(1),
            Event::Negative(-1),
            Event::IndefiniteBytes,
            Event::Bytes(&[1, 2]),
            Event::Break,
            Event::IndefiniteText,
            Event::Text("x"),
            Event::Break,
            Event::Map(None),
            Event::Unsigned(1),
            Event::Bool(true),
            Event::Break,
            Event::Tag(42),
            Event::Null,
            Event::Break,
        ]
    );
}

#[test]
fn decoder_cursor_and_trailing_policy_are_observable() {
    let mut decoder = SliceDecoder::new(&[0x01, 0x02]);
    assert_eq!(decoder.peek().unwrap(), 1);
    assert_eq!(decoder.unsigned().unwrap(), 1);
    assert_eq!(decoder.position(), 1);
    assert_eq!(decoder.remaining(), &[2]);
    let error = decoder.finish().unwrap_err();
    assert_eq!((error.kind(), error.offset()), (ErrorKind::TrailingData, 1));

    let options = DecodeOptions {
        allow_trailing: true,
        ..DecodeOptions::default()
    };
    let mut decoder = SliceDecoder::with_options(&[0x01, 0x02], options);
    assert_eq!(decoder.unsigned().unwrap(), 1);
    decoder.finish().unwrap();
}

#[test]
fn deterministic_mode_rejects_every_non_minimal_argument_width() {
    for bytes in [
        &[0x18, 23][..],
        &[0x19, 0, 255],
        &[0x1a, 0, 0, 255, 255],
        &[0x1b, 0, 0, 0, 0, 255, 255, 255, 255],
        &[0x38, 23],
        &[0x58, 0],
        &[0x78, 0],
        &[0x98, 0],
        &[0xb8, 0],
        &[0xd8, 23, 0],
    ] {
        assert_eq!(
            validate_deterministic(bytes).unwrap_err().kind(),
            ErrorKind::NonDeterministic
        );
        let strict = DecodeOptions {
            validation: Validation::Strict,
            ..DecodeOptions::default()
        };
        let mut decoder = SliceDecoder::with_options(bytes, strict);
        decoder.skip().unwrap();
        decoder.finish().unwrap();
    }
}

#[test]
fn simple_values_and_float_bit_patterns_are_preserved() {
    for simple in [0, 1, 19, 32, 127, 255] {
        let mut bytes = Vec::new();
        Encoder::new(&mut bytes).simple(simple).unwrap();
        assert_eq!(
            Parser::new(&bytes).next().unwrap().unwrap(),
            Event::Simple(simple)
        );
    }
    for assigned_or_reserved in 20..=31 {
        let error = Encoder::new(Vec::new())
            .simple(assigned_or_reserved)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidAdditionalInfo);
    }

    for bits in [0, 1, 0x8000_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_1234] {
        let value = f32::from_bits(bits);
        let mut bytes = Vec::new();
        Encoder::new(&mut bytes).f32(value).unwrap();
        let decoded = SliceDecoder::new(&bytes).float().unwrap() as f32;
        assert_eq!(decoded.to_bits(), bits);
    }
}

#[test]
fn fixed_output_reports_capacity_failure_and_keeps_completed_prefix() {
    let mut storage = [0u8; 4];
    let mut output = SliceOutput::new(&mut storage);
    let mut encoder = Encoder::new(&mut output);
    encoder.unsigned(24).unwrap();
    let error = encoder.text("abc").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::OutputTooSmall);
    assert_eq!(error.offset(), 3);
    assert_eq!(output.len(), 3);
    assert_eq!(&storage[..3], &[0x18, 0x18, 0x63]);
}

#[derive(Default)]
struct AlwaysFails;

impl Output for AlwaysFails {
    fn write_all(&mut self, _bytes: &[u8]) -> fcpw::Result<()> {
        Err(Error::new(ErrorKind::Message, 99))
    }
}

#[test]
fn encoder_propagates_output_errors_unchanged() {
    let error = Encoder::new(AlwaysFails).unsigned(1).unwrap_err();
    assert_eq!((error.kind(), error.offset()), (ErrorKind::Message, 99));
}
