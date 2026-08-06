use fcpw::{Deserializer, ReaderDeserializer, Serializer, Tagged, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Message {
    id: u64,
    body: String,
}

fn main() -> fcpw::Result<()> {
    let message = Message {
        id: 7,
        body: String::from("hello"),
    };

    // Dynamic values use ordinary Serde conversion and preserve CBOR tags.
    let value = fcpw::value::to_value(&Tagged::with_tag(100, &message))?;
    assert!(matches!(value, Value::Tag(100, _)));
    let tagged: Tagged<Message> = fcpw::value::from_value(value)?;
    assert_eq!(tagged.value, message);

    // Stateful slice APIs make consecutive values and offsets observable.
    let mut sequence = fcpw::to_vec(&1_u8)?;
    sequence.extend(fcpw::to_vec(&"next")?);
    let mut deserializer = Deserializer::from_slice(&sequence);
    assert_eq!(u8::deserialize(&mut deserializer)?, 1);
    assert_eq!(deserializer.byte_offset(), 1);
    assert_eq!(String::deserialize(&mut deserializer)?, "next");
    deserializer.end()?;

    // The public serializer writes incrementally to any fcpw::Output.
    let mut output = Vec::new();
    let mut serializer = Serializer::new(&mut output);
    message.serialize(&mut serializer)?;
    assert_eq!(fcpw::from_slice::<Message>(&output)?, message);

    // ReaderDeserializer retains stream offsets across consecutive items.
    let mut reader = ReaderDeserializer::new(sequence.as_slice());
    assert_eq!(reader.deserialize_next::<u8>()?, Some(1));
    assert_eq!(
        reader.deserialize_next::<String>()?.as_deref(),
        Some("next")
    );
    assert_eq!(reader.deserialize_next::<Value>()?, None);

    Ok(())
}
