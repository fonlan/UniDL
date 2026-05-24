use std::{error::Error, fs, ops::Range};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFileEntry {
    pub index: i64,
    pub path: String,
    pub length: i64,
}

#[derive(Debug)]
enum BValue {
    Integer(i64),
    Bytes(Vec<u8>),
    List(Vec<BValue>),
    Dict(Vec<(Vec<u8>, BValue)>),
}

pub fn read_torrent_files(source: &str) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    let bytes = fs::read(source)?;
    torrent_files(&bytes)
}

pub fn read_torrent_info_hash(source: &str) -> Result<String, Box<dyn Error>> {
    let bytes = fs::read(source)?;
    torrent_info_hash(&bytes)
}

fn torrent_files(bytes: &[u8]) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    let mut parser = Parser::new(bytes);
    let root = parser.parse_value(0)?;
    let BValue::Dict(root) = root else {
        return Err("torrent root must be a dictionary".into());
    };
    let info = dict_value(&root, b"info").ok_or("torrent info is missing")?;
    let BValue::Dict(info) = info else {
        return Err("torrent info must be a dictionary".into());
    };

    if let Some(files) = dict_value(info, b"files") {
        let BValue::List(files) = files else {
            return Err("torrent files must be a list".into());
        };
        return files
            .iter()
            .enumerate()
            .map(|(index, file)| torrent_file_entry(index + 1, file))
            .collect();
    }

    let name = bytes_to_string(required_bytes(info, b"name")?);
    let length = required_integer(info, b"length")?;
    Ok(vec![TorrentFileEntry {
        index: 1,
        path: name,
        length,
    }])
}

fn torrent_info_hash(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let mut parser = Parser::new(bytes);
    parser.parse_value(0)?;
    let range = parser.info_range.ok_or("torrent info is missing")?;
    Ok(sha1_hex(&bytes[range]))
}

fn torrent_file_entry(index: usize, file: &BValue) -> Result<TorrentFileEntry, Box<dyn Error>> {
    let BValue::Dict(file) = file else {
        return Err("torrent file entry must be a dictionary".into());
    };
    let length = required_integer(file, b"length")?;
    let path = required_path(file)?;
    Ok(TorrentFileEntry {
        index: i64::try_from(index)?,
        path,
        length,
    })
}

fn required_path(dict: &[(Vec<u8>, BValue)]) -> Result<String, Box<dyn Error>> {
    let path = dict_value(dict, b"path").ok_or("torrent file path is missing")?;
    let BValue::List(parts) = path else {
        return Err("torrent file path must be a list".into());
    };
    let parts = parts
        .iter()
        .map(|part| match part {
            BValue::Bytes(bytes) => Ok(bytes_to_string(bytes)),
            _ => Err("torrent file path part must be bytes".into()),
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    Ok(parts.join("/"))
}

fn required_integer(dict: &[(Vec<u8>, BValue)], key: &[u8]) -> Result<i64, Box<dyn Error>> {
    match dict_value(dict, key) {
        Some(BValue::Integer(value)) => Ok(*value),
        Some(_) => Err(format!("torrent {} must be an integer", bytes_to_string(key)).into()),
        None => Err(format!("torrent {} is missing", bytes_to_string(key)).into()),
    }
}

fn required_bytes<'a>(dict: &'a [(Vec<u8>, BValue)], key: &[u8]) -> Result<&'a [u8], Box<dyn Error>> {
    match dict_value(dict, key) {
        Some(BValue::Bytes(value)) => Ok(value),
        Some(_) => Err(format!("torrent {} must be bytes", bytes_to_string(key)).into()),
        None => Err(format!("torrent {} is missing", bytes_to_string(key)).into()),
    }
}

fn dict_value<'a>(dict: &'a [(Vec<u8>, BValue)], key: &[u8]) -> Option<&'a BValue> {
    dict.iter()
        .find(|(item_key, _)| item_key.as_slice() == key)
        .map(|(_, value)| value)
}

fn bytes_to_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn sha1_hex(bytes: &[u8]) -> String {
    let digest = sha1_smol::Sha1::from(bytes).digest().bytes();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    info_range: Option<Range<usize>>,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            info_range: None,
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<BValue, Box<dyn Error>> {
        match self.peek()? {
            b'i' => self.parse_integer(),
            b'l' => self.parse_list(depth),
            b'd' => self.parse_dict(depth),
            b'0'..=b'9' => self.parse_bytes().map(BValue::Bytes),
            byte => Err(format!("invalid bencode byte: {byte}").into()),
        }
    }

    fn parse_integer(&mut self) -> Result<BValue, Box<dyn Error>> {
        self.expect(b'i')?;
        let start = self.pos;
        while self.peek()? != b'e' {
            self.pos += 1;
        }
        let value = std::str::from_utf8(&self.bytes[start..self.pos])?.parse::<i64>()?;
        self.expect(b'e')?;
        Ok(BValue::Integer(value))
    }

    fn parse_list(&mut self, depth: usize) -> Result<BValue, Box<dyn Error>> {
        self.expect(b'l')?;
        let mut values = Vec::new();
        while self.peek()? != b'e' {
            values.push(self.parse_value(depth + 1)?);
        }
        self.expect(b'e')?;
        Ok(BValue::List(values))
    }

    fn parse_dict(&mut self, depth: usize) -> Result<BValue, Box<dyn Error>> {
        self.expect(b'd')?;
        let mut values = Vec::new();
        while self.peek()? != b'e' {
            let key = self.parse_bytes()?;
            let value_start = self.pos;
            let value = self.parse_value(depth + 1)?;
            if depth == 0 && key.as_slice() == b"info" {
                self.info_range = Some(value_start..self.pos);
            }
            values.push((key, value));
        }
        self.expect(b'e')?;
        Ok(BValue::Dict(values))
    }

    fn parse_bytes(&mut self) -> Result<Vec<u8>, Box<dyn Error>> {
        let start = self.pos;
        while self.peek()? != b':' {
            let byte = self.peek()?;
            if !byte.is_ascii_digit() {
                return Err("bencode byte string length must be numeric".into());
            }
            self.pos += 1;
        }
        let length = std::str::from_utf8(&self.bytes[start..self.pos])?.parse::<usize>()?;
        self.expect(b':')?;
        let end = self.pos.checked_add(length).ok_or("bencode byte string is too long")?;
        if end > self.bytes.len() {
            return Err("bencode byte string exceeds input".into());
        }
        let value = self.bytes[self.pos..end].to_vec();
        self.pos = end;
        Ok(value)
    }

    fn peek(&self) -> Result<u8, Box<dyn Error>> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| "unexpected end of bencode input".into())
    }

    fn expect(&mut self, byte: u8) -> Result<(), Box<dyn Error>> {
        if self.peek()? != byte {
            return Err(format!("expected bencode byte: {byte}").into());
        }
        self.pos += 1;
        Ok(())
    }
}
