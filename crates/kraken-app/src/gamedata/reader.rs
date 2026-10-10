pub fn fail<T>(message: impl Into<String>) -> Result<T, String> {
    Err(message.into())
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    little: bool,
}

impl<'a> Reader<'a> {
    pub fn big(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            little: false,
        }
    }

    pub fn at(bytes: &'a [u8], pos: usize, little: bool) -> Self {
        Self { bytes, pos, little }
    }

    pub fn set_little(&mut self, little: bool) {
        self.little = little;
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(len).ok_or("overflow")?;
        let slice = self.bytes.get(self.pos..end).ok_or("unexpected end of data")?;
        self.pos = end;
        Ok(slice)
    }

    pub fn skip(&mut self, len: usize) -> Result<(), String> {
        self.take(len).map(|_| ())
    }

    pub fn align(&mut self, to: usize) {
        self.pos = self.pos.div_ceil(to) * to;
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut raw: [u8; N] = self.take(N)?.try_into().map_err(|_| "short read")?;
        if !self.little {
            raw.reverse();
        }
        Ok(raw)
    }

    pub fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_le_bytes)
    }

    pub fn i32(&mut self) -> Result<i32, String> {
        self.array().map(i32::from_le_bytes)
    }

    pub fn i64(&mut self) -> Result<i64, String> {
        self.array().map(i64::from_le_bytes)
    }

    pub fn cstr(&mut self) -> Result<String, String> {
        let rest = self.bytes.get(self.pos..).ok_or("unexpected end of data")?;
        let len = rest.iter().position(|b| *b == 0).ok_or("unterminated string")?;
        let text = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(text)
    }
}
