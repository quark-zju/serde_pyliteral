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
}

impl<R: Read> Deserializer<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: PeekRead::from_reader(reader),
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
                    Ok(false)
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
                    Ok(false)
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
}

impl<'de, 'a, R: Read> de::Deserializer<'de> for &'a mut Deserializer<R> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
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
        self.skip_spaces_and_comments()?;
        let mut buf = vec![0; 5];
        let v: V::Value;
        self.peek(&mut buf)?;
        if buf.get(..4) == Some(b"True") {
            v = visitor.visit_bool::<Error>(true)?;
            self.skip(4)?;
        } else if buf.get(..5) == Some(b"False") {
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
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i8>()?;
        visitor.visit_i8(i)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i16>()?;
        visitor.visit_i16(i)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i32>()?;
        visitor.visit_i32(i)
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i64>()?;
        visitor.visit_i64(i)
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u8>()?;
        visitor.visit_u8(i)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u16>()?;
        visitor.visit_u16(i)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_int_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u32>()?;
        visitor.visit_u32(i)
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
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
        let s = self.read_string()?;
        let chars: Vec<char> = s.chars().take(2).collect();
        if chars.len() != 1 {
            Err(Error::TypeMismatch("char", "str".into()))
        } else {
            visitor.visit_char(chars[0])
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_string()?;
        visitor.visit_string(s)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_byte_buf(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let v = self.read_bytes()?;
        visitor.visit_byte_buf(v)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_spaces_and_comments()?;
        let mut buf = vec![0; 4];
        self.peek(&mut buf)?;
        if buf == b"None" {
            self.skip(4)?;
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.read_unit()?;
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        todo!()
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value> {
        todo!()
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        todo!()
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        todo!()
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        todo!()
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        todo!()
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
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
