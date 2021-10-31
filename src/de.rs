use crate::error::unsupported;
use crate::peek::PeekRead;
use crate::Error;
use crate::Result;
use serde::de;
use serde::de::Visitor;
use std::borrow::Cow;
use std::io;
use std::io::Read;

pub fn from_reader<R: Read, T: de::DeserializeOwned>(reader: R) -> Result<T> {
    let mut de = Deserializer::new(reader);
    de::Deserialize::deserialize(&mut de)
}

pub fn from_slice<T: de::DeserializeOwned>(slice: &[u8]) -> Result<T> {
    from_reader(slice)
}

pub fn from_str<T: de::DeserializeOwned>(s: &str) -> Result<T> {
    from_reader(s.as_bytes())
}

pub struct Deserializer<R> {
    reader: PeekRead<R>,
    stack: Vec<Frame>,
}

struct Frame {
    right_bracket: u8,
    count: usize,
}

impl<R: Read> Deserializer<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: PeekRead::from_reader(reader),
            stack: Vec::new(),
        }
    }
}

// Delegate to reader.
impl<R: Read> Deserializer<R> {
    fn peek(&mut self, out: &mut Vec<u8>) -> io::Result<()> {
        self.reader.peek(out)
    }

    fn read_while<T: Default, E: From<io::Error>>(
        &mut self,
        predicate: impl Fn(u8, &mut T) -> std::result::Result<bool, E>,
    ) -> std::result::Result<T, E> {
        self.reader.read_while(predicate)
    }

    fn skip(&mut self, n: usize) -> io::Result<()> {
        self.reader.skip(n)
    }
}

impl<R: Read> Read for Deserializer<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

// Helper methods.
impl<R: Read> Deserializer<R> {
    fn peek_byte(&mut self) -> crate::Result<Option<u8>> {
        self.skip_spaces_and_comments()?;
        let mut v = vec![0];
        self.peek(&mut v)?;
        Ok(v.into_iter().next())
    }

    fn read_int_string(&mut self) -> crate::Result<String> {
        self.skip_spaces_and_comments()?;
        self.read_while(|b, s: &mut String| {
            if s.is_empty() && (b == b'+' || b == b'-') {
                s.push(b as char);
                Ok(true)
            } else if b >= b'0' && b <= b'9' {
                s.push(b as char);
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }

    fn read_string(&mut self) -> crate::Result<String> {
        self.skip_spaces_and_comments()?;

        struct State {
            parsing: ParsingState,
            out: Vec<u8>,
            quote: u8,
        }
        enum ParsingState {
            None,
            Parsing,
            ParsingSlash,
            ParsingUnicode4 { value: u32, count: usize },
            Closed,
        }
        impl Default for State {
            fn default() -> Self {
                State {
                    parsing: ParsingState::None,
                    out: Vec::new(),
                    quote: 0,
                }
            }
        }

        let state = self.read_while(|b, s: &mut State| match s.parsing {
            ParsingState::None => {
                if b == b'"' || b == b'\'' {
                    s.quote = b;
                    s.parsing = ParsingState::Parsing;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            ParsingState::Parsing => match b {
                b'\\' => {
                    s.parsing = ParsingState::ParsingSlash;
                    Ok(true)
                }
                b if b == s.quote => {
                    s.parsing = ParsingState::Closed;
                    Ok(true)
                }
                _ => {
                    s.out.push(b);
                    Ok(true)
                }
            },
            ParsingState::ParsingSlash => {
                let escape = match b {
                    b'0' => 0,
                    b'\\' => b'\\',
                    b'"' => b'"',
                    b'\'' => b'\'',
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => {
                        s.parsing = ParsingState::ParsingUnicode4 { count: 0, value: 0 };
                        return Ok(true);
                    }
                    _ => {
                        return Err(Error::ParseString(
                            format!("unknown escape: \\{}", b as char).into(),
                        ))
                    }
                };
                s.out.push(escape);
                s.parsing = ParsingState::Parsing;
                Ok(true)
            }
            ParsingState::ParsingUnicode4 {
                ref mut count,
                ref mut value,
            } => {
                let v = hex_to_u4(b).ok_or_else(|| {
                    Error::ParseString(format!("unknown hex: \\{}", b as char).into())
                })?;
                *value = ((*value as u32) << 4) | (v as u32);
                *count += 1;
                if *count == 4 {
                    let ch = match char::from_u32(*value) {
                        None => {
                            return Err(Error::ParseString(
                                format!("not utf8 char: {}", *value).into(),
                            ))
                        }
                        Some(ch) => ch,
                    };
                    s.out.extend_from_slice(ch.to_string().as_bytes());
                    s.parsing = ParsingState::Parsing;
                }
                Ok(true)
            }
            ParsingState::Closed => Ok(false),
        })?;
        match state.parsing {
            ParsingState::Closed => {
                let out = String::from_utf8(state.out)
                    .map_err(|e| Error::ParseString(format!("not utf8: {}", e).into()))?;
                Ok(out)
            }
            ParsingState::None => self.type_mismatch("str"),
            _ => Err(Error::ParseString("incomplete str".into())),
        }
    }

    fn read_bytes(&mut self) -> crate::Result<Vec<u8>> {
        self.skip_spaces_and_comments()?;

        struct State {
            parsing: ParsingState,
            out: Vec<u8>,
            quote: u8,
        }
        enum ParsingState {
            None,
            BPrefix,
            Parsing,
            ParsingSlash,
            ParsingHex { value: u8, count: usize },
            Closed,
        }
        impl Default for State {
            fn default() -> Self {
                State {
                    parsing: ParsingState::None,
                    out: Vec::new(),
                    quote: 0,
                }
            }
        }
        let state = self.read_while(|b, s: &mut State| match s.parsing {
            ParsingState::None => {
                if b == b'b' {
                    s.parsing = ParsingState::BPrefix;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            ParsingState::BPrefix => {
                if b == b'"' || b == b'\'' {
                    s.quote = b;
                    s.parsing = ParsingState::Parsing;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            ParsingState::Parsing => match b {
                b'\\' => {
                    s.parsing = ParsingState::ParsingSlash;
                    Ok(true)
                }
                b if b == s.quote => {
                    s.parsing = ParsingState::Closed;
                    Ok(true)
                }
                _ => {
                    s.out.push(b);
                    Ok(true)
                }
            },
            ParsingState::ParsingSlash => {
                let escape = match b {
                    b'0' => 0,
                    b'\\' => b'\\',
                    b'"' => b'"',
                    b'\'' => b'\'',
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'x' => {
                        s.parsing = ParsingState::ParsingHex { count: 0, value: 0 };
                        return Ok(true);
                    }
                    _ => {
                        return Err(Error::ParseBytes(
                            format!("unknown escape: \\{}", b as char).into(),
                        ))
                    }
                };
                s.out.push(escape);
                s.parsing = ParsingState::Parsing;
                Ok(true)
            }
            ParsingState::ParsingHex {
                ref mut count,
                ref mut value,
            } => {
                let v = hex_to_u4(b).ok_or_else(|| {
                    Error::ParseString(format!("unknown hex: \\{}", b as char).into())
                })?;
                *value = (*value << 4) | v;
                *count += 1;
                if *count == 2 {
                    s.out.push(*value);
                    s.parsing = ParsingState::Parsing;
                }
                Ok(true)
            }
            ParsingState::Closed => Ok(false),
        })?;
        match state.parsing {
            ParsingState::Closed => Ok(state.out),
            ParsingState::None => self.type_mismatch("bytes"),
            _ => Err(Error::ParseString("incomplete str".into())),
        }
    }

    fn read_unit(&mut self) -> crate::Result<()> {
        self.skip_spaces_and_comments()?;
        let mut buf = vec![0; 2];
        self.peek(&mut buf)?;
        if buf != b"()" {
            return self.type_mismatch("()");
        } else {
            self.skip(2)?;
        }
        Ok(())
    }

    fn skip_spaces_and_comments(&mut self) -> io::Result<()> {
        self.read_while(|b, in_comment: &mut bool| {
            let need_skip = match (b, *in_comment) {
                (b'#', false) => {
                    *in_comment = true;
                    true
                }
                (_, false) => (b as char).is_ascii_whitespace(),
                (b'\n', true) => {
                    *in_comment = false;
                    true
                }
                (_, true) => true,
            };
            Ok::<_, io::Error>(need_skip)
        })?;
        Ok(())
    }

    /// Raise a TypeMismatch error.
    fn type_mismatch<T>(&mut self, expected: &'static str) -> Result<T> {
        let got: Cow<str> = match self.peek_byte()?.unwrap_or(0) {
            0 => "eof".into(),
            b'[' => "list".into(),
            b'{' => "map".into(),
            b'(' => "tuple".into(),
            b'\'' | b'"' => "str".into(),
            b'b' => "bytes".into(),
            b'T' | b'F' => "bool".into(),
            b'0'..=b'9' | b'+' | b'-' => "number".into(),
            b'N' => "None".into(),
            b => format!("unknown type ({})", b as char).into(),
        };
        Err(Error::TypeMismatch(expected, got))
    }

    /// Push a frame if bracket matches. Return true if a frame is pushed.
    fn maybe_push_bracket(&mut self, left_bracket: u8, right_bracket: u8) -> crate::Result<bool> {
        let b = self.peek_byte()?;
        if b == Some(left_bracket) {
            self.skip(1)?;
            self.stack.push(Frame {
                right_bracket,
                count: 0,
            });
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Pop a frame if bracket matches. Return true if the right
    /// bracket matches.
    fn maybe_pop_bracket(&mut self) -> crate::Result<bool> {
        if let Some(frame) = self.stack.last() {
            let right_bracket = frame.right_bracket;
            if let Some(b) = self.peek_byte()? {
                if b == right_bracket {
                    self.stack.pop();
                    self.skip(1)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Read "," except for the first item.
    /// Call this before reading an item, after maybe_pop_bracket.
    fn maybe_read_comma(&mut self) -> crate::Result<()> {
        if let Some(frame) = self.stack.last_mut() {
            let first = frame.count == 0;
            frame.count += 1;
            // Comma is needed for non-first
            if !first {
                let b = self.peek_byte()?;
                dbg!(b.unwrap_or(b' ') as char);
                if b != Some(b',') {
                    return self.type_mismatch("comma");
                } else {
                    self.skip(1)?;
                }
            }
        }
        Ok(())
    }

    /// Check if we reach the end of a container. Used to implement
    /// seq or map acess. Return true if end is reached, and the
    /// callsite should return `None`.
    ///
    /// The function consumes ',' and the right-side bracket.
    fn check_end_of_container(&mut self) -> crate::Result<bool> {
        if self.maybe_pop_bracket()? {
            return Ok(true);
        }
        self.maybe_read_comma()?;
        // Check again after tailing comma.
        self.maybe_pop_bracket()
    }

    fn debug(&mut self, label: &'static str) {
        if cfg!(test) && cfg!(debug_assertions) {
            let brackets = self
                .stack
                .iter()
                .map(|f| f.right_bracket)
                .collect::<Vec<u8>>();
            let mut buf = vec![b' '; 10];
            self.peek(&mut buf).unwrap();
            eprintln!(
                "{:22} STACK: '{}' PEEK: '{}'",
                label,
                String::from_utf8(brackets).unwrap(),
                String::from_utf8(buf).unwrap(),
            );
        }
        let _ = label;
    }
}

impl<'de, 'a, R: Read> de::Deserializer<'de> for &'a mut Deserializer<R> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_any");
        match self.peek_byte()?.unwrap_or(b' ') {
            b'[' => self.deserialize_seq(visitor),
            b'{' => self.deserialize_map(visitor),
            b'(' => self.deserialize_tuple(0, visitor),
            b'\'' | b'"' => self.deserialize_str(visitor),
            b'b' => self.deserialize_bytes(visitor),
            b'T' | b'F' => self.deserialize_bool(visitor),
            b'0'..=b'9' => self.deserialize_u64(visitor),
            b'-' => self.deserialize_i64(visitor),
            b'N' => self.deserialize_option(visitor),
            b => Err(Error::ParseAny(b as char)),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_bool");
        self.skip_spaces_and_comments()?;
        let mut buf = vec![0; 5];
        let v: V::Value;
        self.peek(&mut buf)?;
        if let Some(b"True") | Some(b"true") = buf.get(..4) {
            v = visitor.visit_bool::<Error>(true)?;
            self.skip(4)?;
        } else if let Some(b"False") | Some(b"false") = buf.get(..5) {
            v = visitor.visit_bool::<Error>(false)?;
            self.skip(5)?;
        } else if buf.get(0) == Some(&b'1') {
            v = visitor.visit_bool::<Error>(true)?;
            self.skip(1)?;
        } else if buf.get(0) == Some(&b'0') {
            v = visitor.visit_bool::<Error>(false)?;
            self.skip(1)?;
        } else {
            return self.type_mismatch("bool");
        }
        Ok(v)
    }

    /* [[[cog
    import cog
    for t in "i8 i16 i32 i64 u8 u16 u32 u64".split():
        cog.out(f"""
    fn deserialize_{t}<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {{
        self.debug("deserialize_{t}");
        let s = self.read_int_string()?;
        if s.is_empty() {{
            return self.type_mismatch("number");
        }}
        let i = s.parse::<{t}>()?;
        visitor.visit_{t}(i)
    }}
    """)
    ]]] */

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i8");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i8>()?;
        visitor.visit_i8(i)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i16");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i16>()?;
        visitor.visit_i16(i)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i32");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i32>()?;
        visitor.visit_i32(i)
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i64");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i64>()?;
        visitor.visit_i64(i)
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u8");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u8>()?;
        visitor.visit_u8(i)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u16");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u16>()?;
        visitor.visit_u16(i)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u32");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u32>()?;
        visitor.visit_u32(i)
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u64");
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u64>()?;
        visitor.visit_u64(i)
    }
    /* [[[end]]] */

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        unsupported("deserialize_f32")
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        unsupported("deserialize_f64")
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_char");
        let s = self.read_string()?;
        let chars: Vec<char> = s.chars().take(2).collect();
        if chars.len() != 1 {
            Err(Error::TypeMismatch("char", "str".into()))
        } else {
            visitor.visit_char(chars[0])
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_str");
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_string");
        let s = self.read_string()?;
        visitor.visit_string(s)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_bytes");
        self.deserialize_byte_buf(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_byte_buf");
        let v = self.read_bytes()?;
        visitor.visit_byte_buf(v)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_option");
        self.skip_spaces_and_comments()?;
        let mut buf = vec![0; 4];
        self.peek(&mut buf)?;
        if buf == b"None" || buf == b"null" {
            self.skip(4)?;
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_unit");
        self.read_unit()?;
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_unit_struct");
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_newtype_struct");
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value> {
        if self.maybe_push_bracket(b'[', b']')? || self.maybe_push_bracket(b'(', b')')? {
            visitor.visit_seq(&mut self)
        } else {
            self.type_mismatch("list")
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(mut self, len: usize, visitor: V) -> Result<V::Value> {
        if self.maybe_push_bracket(b'(', b')')? || self.maybe_push_bracket(b'[', b']')? {
            visitor.visit_seq(&mut self)
        } else {
            self.type_mismatch("tuple")
        }
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_tuple_struct");
        todo!()
    }

    fn deserialize_map<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value> {
        if self.maybe_push_bracket(b'{', b'}')? {
            visitor.visit_map(&mut self)
        } else {
            self.type_mismatch("map")
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_struct");
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_enum");
        todo!()
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_identifier");
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_ignored_any");
        self.deserialize_any(visitor)
    }
}

fn hex_to_u4(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

impl<'de, 'a, R: Read> de::SeqAccess<'de> for &'a mut Deserializer<R> {
    type Error = Error;

    fn next_element_seed<T: de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>> {
        self.debug("next_element_seed");
        if self.check_end_of_container()? {
            return Ok(None);
        }
        seed.deserialize(&mut **self).map(Some)
    }
}

impl<'de, 'a, R: Read> de::MapAccess<'de> for &'a mut Deserializer<R> {
    type Error = Error;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        self.debug("next_key_seed");
        if self.check_end_of_container()? {
            return Ok(None);
        }
        seed.deserialize(&mut **self).map(Some)
    }

    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        self.debug("next_value_seed");
        if self.peek_byte()? == Some(b':') {
            self.skip(1)?;
        } else {
            return self.type_mismatch("colon");
        }
        seed.deserialize(&mut **self)
    }
}
