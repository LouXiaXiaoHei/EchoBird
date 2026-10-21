//! Devin service wire primitives. All offsets and lengths are checked before decoding.

#[derive(Clone, Debug)]
pub(crate) enum Field<'a> {
    Number(u32, u64),
    Bytes(u32, &'a [u8]),
}
pub(crate) fn fields(mut input: &[u8]) -> Result<Vec<Field<'_>>, String> {
    fn integer(input: &mut &[u8]) -> Result<u64, String> {
        let mut value = 0;
        for shift in (0..70).step_by(7) {
            let (&byte, rest) = input.split_first().ok_or("Truncated Devin integer")?;
            *input = rest;
            if shift == 63 && byte > 1 {
                return Err("Invalid Devin integer".into());
            }
            value |= u64::from(byte & 127) << shift;
            if byte < 128 {
                return Ok(value);
            }
        }
        Err("Invalid Devin integer".into())
    }
    let mut result = Vec::new();
    while !input.is_empty() {
        let tag = integer(&mut input)?;
        let number = u32::try_from(tag >> 3).map_err(|_| "Invalid Devin field")?;
        if number == 0 {
            return Err("Invalid Devin field".into());
        }
        match tag & 7 {
            0 => result.push(Field::Number(number, integer(&mut input)?)),
            2 => {
                let length =
                    usize::try_from(integer(&mut input)?).map_err(|_| "Invalid Devin length")?;
                let bytes = input.get(..length).ok_or("Truncated Devin field")?;
                input = &input[length..];
                result.push(Field::Bytes(number, bytes));
            }
            wire @ (1 | 5) => {
                let length = if wire == 1 { 8 } else { 4 };
                input = input.get(length..).ok_or("Truncated Devin fixed field")?;
            }
            _ => return Err("Unsupported Devin wire type".into()),
        }
    }
    Ok(result)
}
pub(crate) fn string(fields: &[Field<'_>], number: u32) -> Option<String> {
    fields.iter().find_map(|f| match f {
        Field::Bytes(n, bytes) if *n == number => {
            std::str::from_utf8(bytes).ok().map(str::to_owned)
        }
        _ => None,
    })
}
pub(crate) fn number(fields: &[Field<'_>], number: u32) -> Option<u64> {
    fields.iter().find_map(|f| match f {
        Field::Number(n, value) if *n == number => Some(*value),
        _ => None,
    })
}
#[derive(Default)]
pub(crate) struct Message(pub Vec<u8>);
impl Message {
    fn integer(&mut self, mut value: u64) {
        while value >= 128 {
            self.0.push(value as u8 | 128);
            value >>= 7;
        }
        self.0.push(value as u8);
    }
    pub(crate) fn num(mut self, field: u32, value: u64) -> Self {
        self.integer(u64::from(field) << 3);
        self.integer(value);
        self
    }
    pub(crate) fn bytes(mut self, field: u32, bytes: impl AsRef<[u8]>) -> Self {
        let bytes = bytes.as_ref();
        self.integer((u64::from(field) << 3) | 2);
        self.integer(bytes.len() as u64);
        self.0.extend(bytes);
        self
    }
    pub(crate) fn double(mut self, field: u32, value: f64) -> Self {
        self.integer((u64::from(field) << 3) | 1);
        self.0.extend(value.to_le_bytes());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_malformed_lengths() {
        let message = Message::default()
            .num(36, 123)
            .bytes(7, "fixture")
            .double(8, 1.0);
        let parsed = fields(&message.0).unwrap();
        assert_eq!(number(&parsed, 36), Some(123));
        assert_eq!(string(&parsed, 7).as_deref(), Some("fixture"));
        for invalid in [
            vec![0],
            vec![10, 255],
            vec![10, 5, 1],
            vec![255; 12],
            vec![9, 0],
        ] {
            assert!(fields(&invalid).is_err());
        }
    }
}
