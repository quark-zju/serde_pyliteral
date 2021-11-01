use std::collections::VecDeque;
use std::io;
use std::io::Read;

pub struct PeekRead<R> {
    reader: R,
    peek: VecDeque<u8>,
}

impl<R: Read> PeekRead<R> {
    pub fn from_reader(reader: R) -> Self {
        Self {
            reader,
            peek: VecDeque::new(),
        }
    }
}

impl<R: Read> PeekRead<R> {
    /// Peek multiple bytes. Truncate the `out` on EOF.
    pub fn peek(&mut self, out: &mut Vec<u8>) -> io::Result<()> {
        for (o, b) in out.iter_mut().zip(self.peek.iter()) {
            *o = *b;
        }
        let mut peek_len = self.peek.len();
        if peek_len < out.len() {
            let mut n = 1;
            while n > 0 {
                n = self.reader.read(&mut out[peek_len..])?;
                for i in 0..n {
                    self.peek.push_back(out[peek_len + i]);
                }
                peek_len += n;
            }
        }
        out.truncate(peek_len);
        Ok(())
    }

    /// Read while `predicate` returns `true`. `predicate` takes the next
    /// byte, and the current state to decide whether to accept the byte
    /// or not. `predicate` should mutate `T` in place if it decides to
    /// accept the byte.
    pub fn read_while<T: Default, E: From<io::Error>>(
        &mut self,
        predicate: impl Fn(u8, &mut T) -> Result<bool, E>,
    ) -> Result<T, E> {
        let mut result = T::default();
        let mut buf = vec![0; 32];
        'a: loop {
            self.peek(&mut buf)?;
            if buf.is_empty() {
                break;
            }
            for &b in buf.iter() {
                if predicate(b, &mut result)? {
                    // predicate accepts the byte.
                    self.skip(1)?;
                } else {
                    break 'a;
                }
            }
        }
        Ok(result)
    }

    /// Skip `n` bytes.
    pub fn skip(&mut self, mut n: usize) -> io::Result<()> {
        let mut buf = [0u8; 32];
        while n > 0 {
            let size = n.min(buf.len());
            self.read_exact(&mut buf[..size])?;
            n -= size;
        }
        Ok(())
    }
}

impl<R: Read> Read for PeekRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut n = buf.len().min(self.peek.len());
        for (i, b) in self.peek.drain(..n).enumerate() {
            buf[i] = b;
        }
        if n < buf.len() {
            n += self.reader.read(&mut buf[n..])?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peek_read() {
        let mut v = PeekRead::from_reader(&b"123456"[..]);
        assert_eq!(peek(2, &mut v), b"12");
        assert_eq!(peek(1, &mut v), b"1");
        assert_eq!(peek(3, &mut v), b"123");

        assert_eq!(read(1, &mut v), b"1");
        assert_eq!(peek(2, &mut v), b"23");
        assert_eq!(read(4, &mut v), b"2345");

        assert_eq!(read(3, &mut v), b"6..");
        assert_eq!(read(3, &mut v), b"...");
        assert_eq!(peek(2, &mut v), b"");
    }

    fn peek(n: usize, peek: &mut PeekRead<&[u8]>) -> Vec<u8> {
        let mut buf = vec![b'.'; n];
        peek.peek(&mut buf).unwrap();
        buf
    }

    fn read(n: usize, peek: &mut PeekRead<&[u8]>) -> Vec<u8> {
        let mut buf = vec![b'.'; n];
        peek.read(&mut buf).unwrap();
        buf
    }
}
