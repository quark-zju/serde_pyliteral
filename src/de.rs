use crate::error::unsupported;
use crate::peek::PeekRead;
use crate::Error;
use crate::Result;
use serde::de;
use serde::de::Deserializer as _;
use serde::de::IntoDeserializer;
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
    size_hint: Option<usize>,
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

    fn read_number_string(&mut self) -> crate::Result<String> {
        self.skip_spaces_and_comments()?;
        self.read_while(|b, s: &mut String| {
            if (b == b'+' || b == b'-') && (s.is_empty() || s.ends_with('e')) {
                s.push(b as char);
                Ok(true)
            } else if b >= b'0' && b <= b'9' {
                s.push(b as char);
                Ok(true)
            } else if b == b'e' && !s.contains('e') {
                s.push(b as char);
                Ok(true)
            } else if b == b'.' && !s.contains('.') && !s.contains('e') {
                s.push(b as char);
                Ok(true)
            } else if b == b'_' {
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

    fn peek_type(&mut self) -> Result<PeekType> {
        let b = self.peek_byte()?.unwrap_or(0);
        let peek_type = match b {
            0 => PeekType::Eof,
            b'[' => PeekType::List,
            b'{' => PeekType::Map,
            b'(' => PeekType::Tuple,
            b'\'' | b'"' => PeekType::Str,
            b'b' => PeekType::Bytes,
            b'T' | b'F' | b't' | b'f' => PeekType::Bool,
            b'0'..=b'9' | b'+' | b'-' => {
                if self.peek_is_float_or_int()? {
                    PeekType::Float
                } else if b == b'-' {
                    PeekType::SignedInt
                } else {
                    PeekType::UnsignedInt
                }
            }
            b'N' => PeekType::None,
            _ => {
                let mut v = vec![b' '; 10];
                self.peek(&mut v)?;
                PeekType::Unknown(String::from_utf8_lossy(&v).to_string())
            }
        };
        Ok(peek_type)
    }

    /// Check if a number is float or int.
    /// Return `true` for float, `false` for int.
    fn peek_is_float_or_int(&mut self) -> Result<bool> {
        // 32-char is enough to hold u64::MAX.
        let mut v = vec![0u8; 32];
        self.peek(&mut v)?;
        for b in v {
            match b {
                b'e' | b'.' => return Ok(true),
                b'0'..=b'9' | b'_' | b'+' | b'-' => continue,
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// Raise a TypeMismatch error.
    fn type_mismatch<T>(&mut self, expected: &'static str) -> Result<T> {
        let got = self.peek_type()?;
        Err(Error::TypeMismatch(expected, got.to_cow_str()))
    }

    /// Push a frame if bracket matches. Return true if a frame is pushed.
    fn maybe_push_bracket(
        &mut self,
        left_bracket: u8,
        right_bracket: u8,
        size_hint: Option<usize>,
    ) -> crate::Result<bool> {
        let b = self.peek_byte()?;
        if b == Some(left_bracket) {
            self.skip(1)?;
            self.stack.push(Frame {
                right_bracket,
                count: 0,
                size_hint,
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

    /// Force read till the end of a container.
    fn force_end_container(&mut self) -> crate::Result<()> {
        while !self.maybe_pop_bracket()? {
            let b = self.peek_byte()?.unwrap_or(b' ');
            if b == b':' || b == b',' {
                self.skip(1)?;
            }
            self.deserialize_ignored_any(de::IgnoredAny)?;
        }
        Ok(())
    }

    fn reach_size_hint(&self) -> bool {
        if let Some(frame) = self.stack.last() {
            if let Some(size) = frame.size_hint {
                if frame.count == size {
                    return true;
                }
            }
        }
        false
    }

    fn debug(&mut self, label: &'static str) {
        if cfg!(test) && cfg!(debug_assertions) {
            if std::env::var_os("DEBUG").is_some() {
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
                    String::from_utf8_lossy(&buf),
                );
            }
        }
        let _ = label;
    }
}

#[derive(Debug)]
enum PeekType {
    Eof,
    List,
    Map,
    Tuple,
    Str,
    Bytes,
    Bool,
    SignedInt,
    UnsignedInt,
    Float,
    None,
    Unknown(String),
}

impl PeekType {
    fn to_cow_str(&self) -> Cow<'static, str> {
        use PeekType::*;
        match self {
            Eof => "end",
            List => "list",
            Map => "map",
            Tuple => "tuple",
            Str => "str",
            Bytes => "bytes",
            Bool => "bool",
            SignedInt | UnsignedInt => "int",
            Float => "float",
            None => "None",
            Unknown(s) => {
                return format!("unknown type ({:?})", s).into();
            }
        }
        .into()
    }
}

impl<'de, 'a, R: Read> de::Deserializer<'de> for &'a mut Deserializer<R> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_any");
        use PeekType::*;
        match self.peek_type()? {
            List | Tuple => self.deserialize_seq(visitor),
            Map => self.deserialize_map(visitor),
            Str => self.deserialize_str(visitor),
            Bytes => self.deserialize_bytes(visitor),
            Bool => self.deserialize_bool(visitor),
            UnsignedInt => self.deserialize_u64(visitor),
            SignedInt => self.deserialize_i64(visitor),
            Float => self.deserialize_f64(visitor),
            None => self.deserialize_option(visitor),
            Eof => Err(Error::ParseAny(String::new())),
            Unknown(s) => Err(Error::ParseAny(s)),
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
    for t in "i8 i16 i32 i64 u8 u16 u32 u64 f32 f64".split():
        cog.out(f"""
    fn deserialize_{t}<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {{
        self.debug("deserialize_{t}");
        let s = self.read_number_string()?;
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
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i8>()?;
        visitor.visit_i8(i)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i16");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i16>()?;
        visitor.visit_i16(i)
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i32");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i32>()?;
        visitor.visit_i32(i)
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_i64");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<i64>()?;
        visitor.visit_i64(i)
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u8");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u8>()?;
        visitor.visit_u8(i)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u16");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u16>()?;
        visitor.visit_u16(i)
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u32");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u32>()?;
        visitor.visit_u32(i)
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_u64");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<u64>()?;
        visitor.visit_u64(i)
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_f32");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<f32>()?;
        visitor.visit_f32(i)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_f64");
        let s = self.read_number_string()?;
        if s.is_empty() {
            return self.type_mismatch("number");
        }
        let i = s.parse::<f64>()?;
        visitor.visit_f64(i)
    }
    /* [[[end]]] */

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
        self.debug("deserialize_seq");
        if self.maybe_push_bracket(b'[', b']', None)?
            || self.maybe_push_bracket(b'(', b')', None)?
        {
            visitor.visit_seq(&mut self)
        } else {
            self.type_mismatch("list")
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(mut self, len: usize, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_tuple");
        if self.maybe_push_bracket(b'(', b')', Some(len))?
            || self.maybe_push_bracket(b'[', b']', Some(len))?
        {
            visitor.visit_seq(&mut self)
        } else {
            self.type_mismatch("tuple")
        }
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_tuple_struct");
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value> {
        self.debug("deserialize_map");
        if self.maybe_push_bracket(b'{', b'}', None)? {
            visitor.visit_map(&mut self)
        } else {
            self.type_mismatch("map")
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_struct");
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("deserialize_enum");
        if self.maybe_push_bracket(b'{', b'}', None)? {
            // Map variant {'field': value}
            visitor.visit_enum(&mut *self)
        } else {
            let b = self.peek_byte()?;
            if b == Some(b'"') || b == Some(b'\'') {
                // String for unit variant.
                let name = self.read_string()?;
                visitor.visit_enum(name.into_deserializer())
            } else {
                self.type_mismatch("enum")
            }
        }
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
        let v = seed.deserialize(&mut **self)?;
        if self.reach_size_hint() {
            // If size_hint is reached, `next_element_seed` won't be called again.
            // Need to read out the right bracket now.
            self.force_end_container()?;
        }
        Ok(Some(v))
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

impl<'de, 'a, R: Read> de::EnumAccess<'de> for &'a mut Deserializer<R> {
    type Error = Error;
    type Variant = Self;

    fn variant_seed<V: de::DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant)> {
        self.debug("variant_seed");
        let key = seed.deserialize(&mut *self)?;
        if self.peek_byte()? == Some(b':') {
            self.skip(1)?;
            Ok((key, self))
        } else {
            self.type_mismatch("colon")
        }
    }
}

impl<'de, 'a, R: Read> de::VariantAccess<'de> for &'a mut Deserializer<R> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        self.debug("unit_variant");
        self.read_unit()?;
        self.force_end_container()
    }

    fn newtype_variant_seed<T: de::DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        self.debug("newtype_variant_seed");
        let v = seed.deserialize(&mut *self)?;
        self.force_end_container()?;
        Ok(v)
    }

    // { field: (value, value, ...) }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value> {
        self.debug("tuple_variant");
        let v = de::Deserializer::deserialize_tuple(&mut *self, len, visitor)?;
        self.force_end_container()?;
        Ok(v)
    }

    // { field: { field: value, field: value, ...} }
    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.debug("struct_variant");
        let v = de::Deserializer::deserialize_map(&mut *self, visitor)?;
        self.force_end_container()?;
        Ok(v)
    }
}
