use crate::unicode::is_printable_or_space;
use crate::Error;
use crate::Result;
use serde::ser::SerializeMap;
use serde::ser::SerializeSeq;
use serde::ser::SerializeStruct;
use serde::ser::SerializeStructVariant;
use serde::ser::SerializeTuple;
use serde::ser::SerializeTupleStruct;
use serde::ser::SerializeTupleVariant;
use serde::Serialize;
use std::io;
use std::io::Write;

pub fn to_writer<W: io::Write, T: ?Sized + Serialize>(writer: W, value: &T) -> Result<()> {
    let mut ser = Serializer::new(writer);
    value.serialize(&mut ser)
}

pub fn to_vec<T: ?Sized + Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut writer = Vec::with_capacity(128);
    to_writer(&mut writer, value)?;
    Ok(writer)
}

pub fn to_string<T: ?Sized + Serialize>(value: &T) -> Result<String> {
    let vec = to_vec(value)?;
    let string = unsafe {
        // We do not emit invalid UTF-8.
        String::from_utf8_unchecked(vec)
    };
    Ok(string)
}

pub struct Serializer<W> {
    writer: W,
    written_bytes: usize,
    stack: Vec<Frame>,
}

struct Frame {
    count: usize,
    right_bracket: &'static [u8],
}

impl<W: Write> Serializer<W> {
    pub fn new(w: W) -> Self {
        Serializer {
            writer: w,
            written_bytes: 0,
            stack: Vec::new(),
        }
    }
}

impl<'a, W: Write> Serializer<W> {
    fn write_str<V: ToString>(&mut self, v: V) -> Result<()> {
        self.write_raw_bytes(v.to_string().as_bytes())
    }

    fn write_raw_bytes(&mut self, v: &[u8]) -> Result<()> {
        self.write_all(v).map_err(From::from)
    }

    fn push_bracket(
        &mut self,
        left_bracket: &'static [u8],
        right_bracket: &'static [u8],
    ) -> Result<()> {
        self.stack.push(Frame {
            count: 0,
            right_bracket,
        });
        self.write_raw_bytes(left_bracket).map_err(From::from)
    }

    fn pop_bracket(&mut self) -> Result<()> {
        if let Some(frame) = self.stack.pop() {
            self.write_raw_bytes(frame.right_bracket)?;
        }
        Ok(())
    }

    fn write_comma(&mut self) -> Result<()> {
        if let Some(frame) = self.stack.last_mut() {
            frame.count += 1;
            if frame.count > 1 {
                self.write_raw_bytes(b",")?;
            }
        }
        Ok(())
    }

    fn push_enum_variant(&mut self, name: &str) -> Result<()> {
        self.push_bracket(b"{", b"}")?;
        write_escaped_string(name, self).map_err(Error::from)?;
        self.write_raw_bytes(b":")
    }
}

impl<'a, W: Write> Write for Serializer<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.writer.write(buf)?;
        self.written_bytes += n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn unsupported<T>(message: &'static str) -> Result<T> {
    Err(Error::Unsupported(message))
}

impl<'a, W: Write> serde::Serializer for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_unit(self) -> Result<()> {
        self.write_raw_bytes(b"()")
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<()> {
        self.serialize_unit()
    }

    fn serialize_bool(self, v: bool) -> Result<()> {
        self.write_raw_bytes(if v { b"True" } else { b"False" })
    }

    fn serialize_u8(self, v: u8) -> Result<()> {
        self.write_str(v)
    }

    fn serialize_u16(self, v: u16) -> Result<()> {
        self.write_str(v)
    }

    fn serialize_u32(self, v: u32) -> Result<()> {
        self.write_str(v)
    }

    fn serialize_u64(self, v: u64) -> Result<()> {
        self.write_str(v)
    }

    #[inline]
    fn serialize_i8(self, v: i8) -> Result<()> {
        self.write_str(v)
    }

    #[inline]
    fn serialize_i16(self, v: i16) -> Result<()> {
        self.write_str(v)
    }

    #[inline]
    fn serialize_i32(self, v: i32) -> Result<()> {
        self.write_str(v)
    }

    #[inline]
    fn serialize_i64(self, v: i64) -> Result<()> {
        self.write_str(v)
    }

    #[inline]
    fn serialize_f32(self, _v: f32) -> Result<()> {
        unsupported("serialize_f32")
    }

    #[inline]
    fn serialize_f64(self, _v: f64) -> Result<()> {
        unsupported("serialize_f64")
    }

    #[inline]
    fn serialize_str(self, v: &str) -> Result<()> {
        write_escaped_string(v, self).map_err(From::from)
    }

    #[inline]
    fn serialize_char(self, c: char) -> Result<()> {
        write_escaped_string(&c.to_string(), self).map_err(From::from)
    }

    #[inline]
    fn serialize_bytes(self, v: &[u8]) -> Result<()> {
        write_escaped_bytes(v, self).map_err(From::from)
    }

    #[inline]
    fn serialize_none(self) -> Result<()> {
        self.write_raw_bytes(b"None")
    }

    #[inline]
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<()> {
        v.serialize(self)
    }

    #[inline]
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
        self.push_bracket(b"[", b"]")?;
        Ok(self)
    }

    #[inline]
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple> {
        self.push_bracket(b"(", b")")?;
        Ok(self)
    }

    #[inline]
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        self.push_bracket(b"(", b")")?;
        Ok(self)
    }

    #[inline]
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        self.push_enum_variant(variant)?;
        self.push_bracket(b"(", b")")?;
        Ok(self)
    }

    #[inline]
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
        self.push_bracket(b"{", b"}")?;
        Ok(self)
    }

    #[inline]
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeStruct> {
        self.push_bracket(b"{", b"}")?;
        Ok(self)
    }

    #[inline]
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        self.push_enum_variant(variant)?;
        self.push_bracket(b"{", b"}")?;
        Ok(self)
    }

    #[inline]
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<()> {
        value.serialize(self)
    }

    #[inline]
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<()> {
        self.push_enum_variant(variant)?;
        value.serialize(&mut *self)?;
        self.pop_bracket()
    }

    #[inline]
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<()> {
        self.push_enum_variant(variant)?;
        self.serialize_unit()?;
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeSeq for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<V: ?Sized + Serialize>(&mut self, value: &V) -> Result<()> {
        self.write_comma()?;
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeTuple for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<V: ?Sized + Serialize>(&mut self, value: &V) -> Result<()> {
        self.write_comma()?;
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeTupleStruct for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<V: ?Sized + Serialize>(&mut self, value: &V) -> Result<()> {
        self.write_comma()?;
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeTupleVariant for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<V: ?Sized + Serialize>(&mut self, value: &V) -> Result<()> {
        self.write_comma()?;
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()?;
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeMap for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<K: ?Sized + Serialize>(&mut self, key: &K) -> Result<()> {
        self.write_comma()?;
        key.serialize(&mut **self)?;
        self.write_raw_bytes(b":")
    }

    fn serialize_value<V: ?Sized + Serialize>(&mut self, value: &V) -> Result<()> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeStruct for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<V: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &V,
    ) -> Result<()> {
        self.write_comma()?;
        key.serialize(&mut **self)?;
        self.write_raw_bytes(b":")?;
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()
    }
}

impl<'a, W: Write> SerializeStructVariant for &'a mut Serializer<W> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<V: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &V,
    ) -> Result<()> {
        self.write_comma()?;
        key.serialize(&mut **self)?;
        self.write_raw_bytes(b":")?;
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        self.pop_bracket()?;
        self.pop_bracket()
    }
}

fn to_hex_char(b: u8) -> u8 {
    assert!(b < 16);
    b"0123456789abcdef"[b as usize]
}

fn to_hex_string(bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(bytes.len() * 2);
    const HEX: &[u8] = b"0123456789abcdef";
    for &b in bytes {
        v.push(HEX[(b >> 4) as usize]);
        v.push(HEX[(b & 15) as usize]);
    }
    v
}

// See unicode_repr in cpython and
// https://docs.python.org/3/reference/lexical_analysis.html#string-and-bytes-literals
fn write_escaped_string(value: &str, out: &mut impl io::Write) -> io::Result<()> {
    let quote = if value.contains('\"') && !value.contains('\'') {
        b'\''
    } else {
        b'"'
    };
    out.write_all(&[quote])?;

    let mut state = WriteBytesState::from_value(value.as_bytes());
    let mut skipping = false;
    for (i, ch) in value.char_indices() {
        if skipping {
            state.skip_to(i);
            skipping = false;
        }
        let escape: &[u8] = match ch {
            '\0' => br"\0",
            '"' if quote == b'"' => br#"\""#,
            '\'' if quote == b'\'' => br"'",
            '\\' => br"\\",
            '\n' => br"\n",
            '\r' => br"\r",
            '\t' => br"\t",
            _ => {
                if !is_printable_or_space(ch) {
                    // Use \uxxxx or \Uxxxxxxxx to escape.
                    out.write_all(state.pending(i))?;
                    let v = ch as u32;
                    if v <= u16::MAX as u32 {
                        let v = v as u16;
                        out.write_all(br"\u")?;
                        out.write_all(&to_hex_string(&v.to_be_bytes()))?;
                    } else {
                        out.write_all(br"\U")?;
                        out.write_all(&to_hex_string(&v.to_be_bytes()))?;
                    }
                    skipping = true;
                }
                continue;
            }
        };
        out.write_all(state.pending(i))?;
        out.write_all(escape)?;
        skipping = true;
    }
    if !skipping {
        out.write_all(state.pending(value.as_bytes().len()))?;
    }
    out.write_all(&[quote])
}

fn write_escaped_bytes(value: &[u8], out: &mut impl io::Write) -> io::Result<()> {
    out.write_all(b"b\"")?;
    let mut state = WriteBytesState::from_value(value);
    let mut skipping = false;
    for (i, &b) in value.iter().enumerate() {
        if skipping {
            state.skip_to(i);
            skipping = false;
        }
        let escape = match b {
            0 => br"\0",
            b'"' => br#"\""#,
            b'\\' => br"\\",
            b'\n' => br"\n",
            b'\r' => br"\r",
            b'\t' => br"\t",
            _ => {
                if b >= b' ' && b < 0x7f {
                    // No need to escape. Flush later.
                    continue;
                } else {
                    // Use \xxx to escape.
                    out.write_all(state.pending(i))?;
                    out.write_all(b"\\x")?;
                    let low = b & 15;
                    let high = b >> 4;
                    out.write_all(&[to_hex_char(high), to_hex_char(low)])?;
                    skipping = true;
                    continue;
                }
            }
        };
        out.write_all(state.pending(i))?;
        out.write_all(escape)?;
        skipping = true;
    }
    if !skipping {
        out.write_all(state.pending(value.len()))?;
    }
    out.write_all(b"\"")
}

// Used to reduce small "write" calls if no escape is needed.
struct WriteBytesState<'a> {
    value: &'a [u8],
    start: usize,
}
impl<'a> WriteBytesState<'a> {
    fn from_value(value: &'a [u8]) -> Self {
        Self { value, start: 0 }
    }

    fn pending(&mut self, pos: usize) -> &[u8] {
        let start = self.start;
        debug_assert!(pos >= start);
        self.start = pos;
        &self.value[start..pos]
    }

    fn skip_to(&mut self, pos: usize) {
        self.start = pos;
    }
}
