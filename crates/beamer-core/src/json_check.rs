//! The check in debug builds that a value reaches the page as the plugin
//! sent it.
//!
//! JSON has no infinity and no NaN: `serde_json` writes a float that is not
//! finite as `null`, which the page's model types do not expect (a float
//! is `number` there) and which counts as 0 in arithmetic. JavaScript also
//! rounds an integer beyond 2^53. [`unfaithful_number`] finds such a number
//! in a value without writing it, so that debug builds can report it once
//! per sender or event of beamer-webview. [`non_finite_float`] finds the
//! floats alone, for the value a `State` saves: Rust reads a large integer
//! back exactly.

use std::fmt;

use serde::ser::{self, Serialize};

/// The largest integer JavaScript holds exactly: 2^53.
const MAX_EXACT_INTEGER: u128 = 1 << 53;

/// The first number in `value` that the page would not get as it is, with
/// where it sits (`.peak[1]`), or `None` when every number is faithful.
///
/// A value whose `Serialize` fails is not reported here: writing it fails
/// as well, which is reported where it is written.
pub fn unfaithful_number(value: &(impl Serialize + ?Sized)) -> Option<String> {
    inspect(value, true)
}

/// The first float in `value` that is not finite, with where it sits, or
/// `None`. JSON writes it as `null`, which does not read back.
pub fn non_finite_float(value: &(impl Serialize + ?Sized)) -> Option<String> {
    inspect(value, false)
}

/// Report what [`unfaithful_number`] found in a value of `what` (such as
/// `the value sender "meters"`), to the log and to standard error, as the
/// other reports of the queue do.
pub fn report(what: &str, found: &str) {
    warn(&format!(
        "{what} sent {found}. The page gets null for a float that is not finite and a rounded \
         number for an integer beyond 2^53, where its model types promise a number. Send finite \
         numbers: a level as a linear value, not in dB. Reported once."
    ));
}

/// Report what [`non_finite_float`] found in the value a `State` of the
/// type `type_name` saves.
pub fn report_saved(type_name: &str, found: &str) {
    warn(&format!(
        "A `State<{type_name}>` field saved {found}. JSON writes a float that is not finite as \
         null, which does not load back. The page gets null where its model types promise a \
         number. Reported once."
    ));
}

fn warn(message: &str) {
    log::warn!("{message}");
    eprintln!("{message}");
}

fn inspect(value: &(impl Serialize + ?Sized), integers: bool) -> Option<String> {
    match value.serialize(Inspector { integers }) {
        Err(Found::Number { number, mut path }) => {
            if path.is_empty() {
                return Some(number);
            }
            // The segments were collected from the number outwards.
            path.reverse();
            let place: String = path.iter().map(Segment::to_string).collect();
            Some(format!("{number} at {place}"))
        }
        Ok(()) | Err(Found::Other) => None,
    }
}

/// One step on the way to a number.
#[derive(Debug)]
enum Segment {
    /// A field of a struct or a variant: `.name`.
    Field(&'static str),
    /// An element of a sequence or a tuple: `[1]`.
    Index(usize),
    /// The value of a map's key: `["left"]`.
    Key(String),
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Segment::Field(name) => write!(f, ".{name}"),
            Segment::Index(index) => write!(f, "[{index}]"),
            Segment::Key(key) => write!(f, "[{key:?}]"),
        }
    }
}

/// Why the inspection stopped.
#[derive(Debug)]
enum Found {
    /// A number the page would not get as it is, with the segments from it
    /// outwards, gathered while the inspection returns.
    Number { number: String, path: Vec<Segment> },
    /// An error of the value's own `Serialize`.
    Other,
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Found::Number { number, .. } => f.write_str(number),
            Found::Other => f.write_str("the value could not be serialized"),
        }
    }
}

impl std::error::Error for Found {}

impl ser::Error for Found {
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Found::Other
    }
}

/// Add the segment of where a number was found, on the way out. A path is
/// only built for a number found: a faithful value allocates nothing, apart
/// from the buffer of a map's keys.
fn within(result: Result<(), Found>, segment: impl FnOnce() -> Segment) -> Result<(), Found> {
    match result {
        Err(Found::Number { number, mut path }) => {
            path.push(segment());
            Err(Found::Number { number, path })
        }
        result => result,
    }
}

/// A serializer that writes nothing and stops at the first unfaithful
/// number. `integers` also stops at an integer beyond 2^53.
#[derive(Clone, Copy)]
struct Inspector {
    integers: bool,
}

impl Inspector {
    fn found(number: impl fmt::Display) -> Found {
        Found::Number { number: number.to_string(), path: Vec::new() }
    }

    fn float(self, value: f64) -> Result<(), Found> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(Self::found(value))
        }
    }

    fn integer(self, magnitude: u128, shown: impl fmt::Display) -> Result<(), Found> {
        if !self.integers || magnitude <= MAX_EXACT_INTEGER {
            Ok(())
        } else {
            Err(Self::found(shown))
        }
    }

    fn compound(self, variant: Option<&'static str>) -> Compound {
        Compound { inspector: self, index: 0, key: String::new(), variant }
    }
}

/// The inspector of a compound value: a sequence, a tuple, a map or a
/// struct, which counts its elements for their place.
struct Compound {
    inspector: Inspector,
    index: usize,
    // The current key of a map, as text. One buffer for every key.
    key: String,
    // The variant of a tuple or struct variant.
    variant: Option<&'static str>,
}

impl Compound {
    fn in_variant(&self, result: Result<(), Found>) -> Result<(), Found> {
        match self.variant {
            Some(variant) => within(result, || Segment::Field(variant)),
            None => result,
        }
    }

    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Found> {
        let index = self.index;
        self.index += 1;
        let result = within(value.serialize(self.inspector), || Segment::Index(index));
        self.in_variant(result)
    }

    fn field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Found> {
        let result = within(value.serialize(self.inspector), || Segment::Field(key));
        self.in_variant(result)
    }
}

impl ser::Serializer for Inspector {
    type Ok = ();
    type Error = Found;
    type SerializeSeq = Compound;
    type SerializeTuple = Compound;
    type SerializeTupleStruct = Compound;
    type SerializeTupleVariant = Compound;
    type SerializeMap = Compound;
    type SerializeStruct = Compound;
    type SerializeStructVariant = Compound;

    fn serialize_bool(self, _: bool) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_i8(self, _: i8) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_i16(self, _: i16) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_i32(self, _: i32) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_i64(self, value: i64) -> Result<(), Found> {
        self.integer(u128::from(value.unsigned_abs()), value)
    }

    fn serialize_i128(self, value: i128) -> Result<(), Found> {
        self.integer(value.unsigned_abs(), value)
    }

    fn serialize_u8(self, _: u8) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_u16(self, _: u16) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_u32(self, _: u32) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_u64(self, value: u64) -> Result<(), Found> {
        self.integer(u128::from(value), value)
    }

    fn serialize_u128(self, value: u128) -> Result<(), Found> {
        self.integer(value, value)
    }

    fn serialize_f32(self, value: f32) -> Result<(), Found> {
        self.float(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> Result<(), Found> {
        self.float(value)
    }

    fn serialize_char(self, _: char) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_str(self, _: &str) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_none(self) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Found> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<(), Found> {
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<(), Found> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Found> {
        within(value.serialize(self), || Segment::Field(variant))
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Compound, Found> {
        Ok(self.compound(None))
    }

    fn serialize_tuple(self, _: usize) -> Result<Compound, Found> {
        Ok(self.compound(None))
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Compound, Found> {
        Ok(self.compound(None))
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound, Found> {
        Ok(self.compound(Some(variant)))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Compound, Found> {
        Ok(self.compound(None))
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Compound, Found> {
        Ok(self.compound(None))
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound, Found> {
        Ok(self.compound(Some(variant)))
    }
}

impl ser::SerializeSeq for Compound {
    type Ok = ();
    type Error = Found;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Found> {
        self.element(value)
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeTuple for Compound {
    type Ok = ();
    type Error = Found;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Found> {
        self.element(value)
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeTupleStruct for Compound {
    type Ok = ();
    type Error = Found;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Found> {
        self.element(value)
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeTupleVariant for Compound {
    type Ok = ();
    type Error = Found;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Found> {
        self.element(value)
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeMap for Compound {
    type Ok = ();
    type Error = Found;

    // Keys are text in JSON. A key is kept as text for the place of its
    // value. A key of another kind leaves it empty: JSON cannot write such
    // a map at all.
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Found> {
        self.key.clear();
        let _ = key.serialize(KeyText(&mut self.key));
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Found> {
        let result = value.serialize(self.inspector);
        within(result, || Segment::Key(self.key.clone()))
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeStruct for Compound {
    type Ok = ();
    type Error = Found;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Found> {
        self.field(key, value)
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

impl ser::SerializeStructVariant for Compound {
    type Ok = ();
    type Error = Found;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Found> {
        self.field(key, value)
    }

    fn end(self) -> Result<(), Found> {
        Ok(())
    }
}

/// A serializer that writes a map's key as JSON has it, into a buffer:
/// text as it is, a number or a bool as its digits or word, the name of a
/// unit variant. Any other key is refused.
struct KeyText<'a>(&'a mut String);

impl KeyText<'_> {
    fn write(self, text: impl fmt::Display) -> Result<(), fmt::Error> {
        use std::fmt::Write as _;
        write!(self.0, "{text}")
    }
}

/// The error of a key that is no text.
#[derive(Debug)]
struct NoText;

impl fmt::Display for NoText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the key is no text")
    }
}

impl std::error::Error for NoText {}

impl ser::Error for NoText {
    fn custom<T: fmt::Display>(_message: T) -> Self {
        NoText
    }
}

impl From<fmt::Error> for NoText {
    fn from(_: fmt::Error) -> Self {
        NoText
    }
}

macro_rules! key_as_text {
    ($($method:ident: $type:ty),*) => {
        $(fn $method(self, value: $type) -> Result<(), NoText> {
            Ok(self.write(value)?)
        })*
    };
}

impl ser::Serializer for KeyText<'_> {
    type Ok = ();
    type Error = NoText;
    type SerializeSeq = ser::Impossible<(), NoText>;
    type SerializeTuple = ser::Impossible<(), NoText>;
    type SerializeTupleStruct = ser::Impossible<(), NoText>;
    type SerializeTupleVariant = ser::Impossible<(), NoText>;
    type SerializeMap = ser::Impossible<(), NoText>;
    type SerializeStruct = ser::Impossible<(), NoText>;
    type SerializeStructVariant = ser::Impossible<(), NoText>;

    key_as_text!(
        serialize_bool: bool, serialize_i8: i8, serialize_i16: i16, serialize_i32: i32,
        serialize_i64: i64, serialize_i128: i128, serialize_u8: u8, serialize_u16: u16,
        serialize_u32: u32, serialize_u64: u64, serialize_u128: u128, serialize_f32: f32,
        serialize_f64: f64, serialize_char: char, serialize_str: &str
    );

    fn serialize_bytes(self, _: &[u8]) -> Result<(), NoText> {
        Err(NoText)
    }

    fn serialize_none(self) -> Result<(), NoText> {
        Err(NoText)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), NoText> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), NoText> {
        Err(NoText)
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), NoText> {
        Err(NoText)
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<(), NoText> {
        Ok(self.write(variant)?)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<(), NoText> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), NoText> {
        Err(NoText)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, NoText> {
        Err(NoText)
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, NoText> {
        Err(NoText)
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, NoText> {
        Err(NoText)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, NoText> {
        Err(NoText)
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, NoText> {
        Err(NoText)
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, NoText> {
        Err(NoText)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, NoText> {
        Err(NoText)
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use std::collections::BTreeMap;

    use super::{non_finite_float, unfaithful_number};

    #[derive(Serialize)]
    struct Levels {
        peak: [f32; 2],
        count: u64,
    }

    #[derive(Serialize)]
    enum Event {
        Note { velocity: f64 },
    }

    #[test]
    fn test_finite_numbers_are_faithful() {
        assert_eq!(unfaithful_number(&Levels { peak: [0.5, -60.0], count: 1 << 53 }), None);
        assert_eq!(unfaithful_number(&vec![Some(1.0f32), None]), None);
        assert_eq!(unfaithful_number("text"), None);
    }

    #[test]
    fn test_a_float_that_is_not_finite_is_found_with_its_place() {
        assert_eq!(
            unfaithful_number(&Levels { peak: [0.5, f32::NEG_INFINITY], count: 0 }).as_deref(),
            Some("-inf at .peak[1]")
        );
        assert_eq!(unfaithful_number(&f64::NAN).as_deref(), Some("NaN"));
        assert_eq!(
            unfaithful_number(&vec![Event::Note { velocity: f64::INFINITY }]).as_deref(),
            Some("inf at [0].Note.velocity")
        );
    }

    #[test]
    fn test_an_integer_beyond_2_to_the_53_is_found() {
        assert_eq!(
            unfaithful_number(&Levels { peak: [0.0; 2], count: (1 << 53) + 1 }).as_deref(),
            Some("9007199254740993 at .count")
        );
        assert_eq!(unfaithful_number(&-(1i64 << 60)).as_deref(), Some("-1152921504606846976"));
        assert!(unfaithful_number(&u128::MAX).is_some());
    }

    #[test]
    fn test_a_value_in_a_map_is_found_at_its_key() {
        let map = BTreeMap::from([("left", 0.5), ("right", f64::NAN)]);
        assert_eq!(unfaithful_number(&map).as_deref(), Some("NaN at [\"right\"]"));
        let by_number = BTreeMap::from([(3u8, vec![f32::INFINITY])]);
        assert_eq!(unfaithful_number(&by_number).as_deref(), Some("inf at [\"3\"][0]"));
    }

    #[test]
    fn test_saving_finds_floats_alone() {
        assert_eq!(non_finite_float(&u64::MAX), None);
        assert_eq!(
            non_finite_float(&Levels { peak: [f32::NAN, 0.0], count: u64::MAX }).as_deref(),
            Some("NaN at .peak[0]")
        );
    }
}
