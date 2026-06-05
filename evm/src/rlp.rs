#[derive(Debug, thiserror::Error)]
pub enum RlpError {
    #[error("rlp: unexpected end of input")]
    Eof,
    #[error("rlp: expected a string item, found a list")]
    ExpectedString,
    #[error("rlp: expected a list item, found a string")]
    ExpectedList,
    #[error("rlp: trailing bytes after item")]
    Trailing,
    #[error("rlp: non-canonical length encoding")]
    NonCanonical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rlp {
    Str(Vec<u8>),
    List(Vec<Rlp>),
}

impl Rlp {
    pub fn as_bytes(&self) -> Result<&[u8], RlpError> {
        match self {
            Rlp::Str(b) => Ok(b),
            Rlp::List(_) => Err(RlpError::ExpectedString),
        }
    }

    pub fn as_list(&self) -> Result<&[Rlp], RlpError> {
        match self {
            Rlp::List(items) => Ok(items),
            Rlp::Str(_) => Err(RlpError::ExpectedList),
        }
    }

    pub fn as_u64(&self) -> Result<u64, RlpError> {
        let b = self.as_bytes()?;
        if b.len() > 8 {
            return Err(RlpError::NonCanonical);
        }
        let mut v = 0u64;
        for byte in b {
            v = (v << 8) | (*byte as u64);
        }
        Ok(v)
    }
}

pub fn decode(input: &[u8]) -> Result<Rlp, RlpError> {
    let (item, rest) = decode_item(input)?;
    if !rest.is_empty() {
        return Err(RlpError::Trailing);
    }
    Ok(item)
}

fn decode_item(input: &[u8]) -> Result<(Rlp, &[u8]), RlpError> {
    let first = *input.first().ok_or(RlpError::Eof)?;
    if first <= 0x7f {
        Ok((Rlp::Str(vec![first]), &input[1..]))
    } else if first <= 0xb7 {
        let len = (first - 0x80) as usize;
        let body = input.get(1..1 + len).ok_or(RlpError::Eof)?;
        Ok((Rlp::Str(body.to_vec()), &input[1 + len..]))
    } else if first <= 0xbf {
        let len_of_len = (first - 0xb7) as usize;
        let len_bytes = input.get(1..1 + len_of_len).ok_or(RlpError::Eof)?;
        let len = be_to_usize(len_bytes)?;
        let start = 1 + len_of_len;
        let body = input.get(start..start + len).ok_or(RlpError::Eof)?;
        Ok((Rlp::Str(body.to_vec()), &input[start + len..]))
    } else if first <= 0xf7 {
        let len = (first - 0xc0) as usize;
        let body = input.get(1..1 + len).ok_or(RlpError::Eof)?;
        let items = decode_list_body(body)?;
        Ok((Rlp::List(items), &input[1 + len..]))
    } else {
        let len_of_len = (first - 0xf7) as usize;
        let len_bytes = input.get(1..1 + len_of_len).ok_or(RlpError::Eof)?;
        let len = be_to_usize(len_bytes)?;
        let start = 1 + len_of_len;
        let body = input.get(start..start + len).ok_or(RlpError::Eof)?;
        let items = decode_list_body(body)?;
        Ok((Rlp::List(items), &input[start + len..]))
    }
}

fn decode_list_body(mut body: &[u8]) -> Result<Vec<Rlp>, RlpError> {
    let mut items = Vec::new();
    while !body.is_empty() {
        let (item, rest) = decode_item(body)?;
        items.push(item);
        body = rest;
    }
    Ok(items)
}

fn be_to_usize(bytes: &[u8]) -> Result<usize, RlpError> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(RlpError::NonCanonical);
    }
    let mut v = 0usize;
    for b in bytes {
        v = (v << 8) | (*b as usize);
    }
    Ok(v)
}

fn encode_length(len: usize, offset: u8) -> Vec<u8> {
    if len < 56 {
        vec![offset + len as u8]
    } else {
        let be = len.to_be_bytes();
        let trimmed: Vec<u8> = be.iter().copied().skip_while(|&b| b == 0).collect();
        let mut out = vec![offset + 55 + trimmed.len() as u8];
        out.extend_from_slice(&trimmed);
        out
    }
}

pub fn encode_str(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] <= 0x7f {
        return vec![bytes[0]];
    }
    let mut out = encode_length(bytes.len(), 0x80);
    out.extend_from_slice(bytes);
    out
}

pub fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    for item in items {
        body.extend_from_slice(item);
    }
    let mut out = encode_length(body.len(), 0xc0);
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_short_string() {
        let r = decode(&[0x83, b'd', b'o', b'g']).unwrap();
        assert_eq!(r.as_bytes().unwrap(), b"dog");
    }

    #[test]
    fn decode_single_byte() {
        let r = decode(&[0x0f]).unwrap();
        assert_eq!(r.as_bytes().unwrap(), &[0x0f]);
    }

    #[test]
    fn decode_list_of_strings() {
        let encoded = [0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g'];
        let r = decode(&encoded).unwrap();
        let items = r.as_list().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_bytes().unwrap(), b"cat");
        assert_eq!(items[1].as_bytes().unwrap(), b"dog");
    }

    #[test]
    fn roundtrip_list_encode_decode() {
        let a = encode_str(b"cat");
        let b = encode_str(b"dog");
        let list = encode_list(&[a, b]);
        let decoded = decode(&list).unwrap();
        let items = decoded.as_list().unwrap();
        assert_eq!(items[0].as_bytes().unwrap(), b"cat");
        assert_eq!(items[1].as_bytes().unwrap(), b"dog");
    }

    #[test]
    fn as_u64_parses_be() {
        let r = Rlp::Str(vec![0x01, 0x00]);
        assert_eq!(r.as_u64().unwrap(), 256);
    }
}
