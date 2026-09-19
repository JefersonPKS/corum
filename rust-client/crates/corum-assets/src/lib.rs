#![forbid(unsafe_code)]

pub mod chr;
pub mod dds;
pub mod map_script;
pub mod model;
pub mod motion;
pub mod stm;
pub mod ttb;
pub mod vcl;

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const PAK_HEADER_SIZE: u64 = 92;
pub const ENTRY_HEADER_SIZE: u64 = 32;
const MAX_ENTRY_NAME_SIZE: u32 = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PakHeader {
    pub version: u32,
    pub entry_count: u32,
    pub flags: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PakEntry {
    pub index: u32,
    pub name: String,
    pub record_offset: u64,
    pub data_offset: u64,
    pub size: u64,
    pub total_size: u64,
}

#[derive(Debug, Clone)]
pub struct PakArchive {
    path: PathBuf,
    file_size: u64,
    header: PakHeader,
    entries: Vec<PakEntry>,
}

#[derive(Debug)]
pub enum PakError {
    Io(io::Error),
    InvalidFormat { offset: u64, message: String },
    EntryNotFound(String),
    UnsafeEntryName(String),
}

impl fmt::Display for PakError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::InvalidFormat { offset, message } => {
                write!(formatter, "invalid PAK at offset 0x{offset:X}: {message}")
            }
            Self::EntryNotFound(name) => write!(formatter, "entry not found: {name}"),
            Self::UnsafeEntryName(name) => write!(formatter, "unsafe entry name: {name}"),
        }
    }
}

impl std::error::Error for PakError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PakError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl PakArchive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PakError> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let file_size = file.metadata()?.len();
        let (header, entries) = parse(&mut file, file_size)?;

        Ok(Self {
            path,
            file_size,
            header,
            entries,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn header(&self) -> &PakHeader {
        &self.header
    }

    pub fn entries(&self) -> &[PakEntry] {
        &self.entries
    }

    pub fn entry(&self, name: &str) -> Option<&PakEntry> {
        self.entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
    }

    pub fn extract_entry(
        &self,
        entry_name: &str,
        output_directory: impl AsRef<Path>,
    ) -> Result<PathBuf, PakError> {
        let entry = self
            .entry(entry_name)
            .ok_or_else(|| PakError::EntryNotFound(entry_name.to_owned()))?;
        self.extract(entry, output_directory.as_ref())
    }

    pub fn read_entry(&self, entry_name: &str) -> Result<Vec<u8>, PakError> {
        let entry = self
            .entry(entry_name)
            .ok_or_else(|| PakError::EntryNotFound(entry_name.to_owned()))?;
        let size = usize::try_from(entry.size)
            .map_err(|_| invalid(entry.data_offset, "entry does not fit in memory"))?;
        let mut source = File::open(&self.path)?;
        source.seek(SeekFrom::Start(entry.data_offset))?;
        let mut bytes = vec![0_u8; size];
        source.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    pub fn extract_all(&self, output_directory: impl AsRef<Path>) -> Result<u32, PakError> {
        let output_directory = output_directory.as_ref();
        for entry in &self.entries {
            self.extract(entry, output_directory)?;
        }
        Ok(self.entries.len() as u32)
    }

    fn extract(&self, entry: &PakEntry, output_directory: &Path) -> Result<PathBuf, PakError> {
        let relative_path = safe_relative_path(&entry.name)?;
        let output_path = output_directory.join(relative_path);

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut source = File::open(&self.path)?;
        source.seek(SeekFrom::Start(entry.data_offset))?;
        let mut limited = source.take(entry.size);
        let mut output = File::create(&output_path)?;
        let copied = io::copy(&mut limited, &mut output)?;
        if copied != entry.size {
            return Err(invalid(
                entry.data_offset,
                format!(
                    "entry '{}' ended early: expected {} bytes, copied {copied}",
                    entry.name, entry.size
                ),
            ));
        }

        Ok(output_path)
    }
}

fn parse<R: Read + Seek>(
    reader: &mut R,
    file_size: u64,
) -> Result<(PakHeader, Vec<PakEntry>), PakError> {
    if file_size < PAK_HEADER_SIZE {
        return Err(invalid(0, "file is smaller than the 92-byte PAK header"));
    }

    reader.seek(SeekFrom::Start(0))?;
    let version = read_u32(reader)?;
    let entry_count = read_u32(reader)?;
    let flags = read_u32(reader)?;
    let pack_name_size = read_u32(reader)?;

    if version != 1 {
        return Err(invalid(0, format!("unsupported PAK version {version}")));
    }
    if pack_name_size > 76 {
        return Err(invalid(
            12,
            format!("pack name length {pack_name_size} exceeds its 76-byte field"),
        ));
    }

    let mut pack_name_bytes = [0_u8; 76];
    reader.read_exact(&mut pack_name_bytes)?;
    let pack_name = decode_name(&pack_name_bytes[..pack_name_size as usize]);

    let header = PakHeader {
        version,
        entry_count,
        flags,
        name: pack_name,
    };

    let maximum_possible_entries = (file_size - PAK_HEADER_SIZE) / (ENTRY_HEADER_SIZE + 1);
    if u64::from(entry_count) > maximum_possible_entries {
        return Err(invalid(
            4,
            format!("entry count {entry_count} cannot fit in a {file_size}-byte archive"),
        ));
    }

    let capacity = usize::try_from(entry_count).map_err(|_| {
        invalid(
            4,
            format!("entry count {entry_count} does not fit in memory"),
        )
    })?;
    let mut entries = Vec::with_capacity(capacity);
    let mut position = PAK_HEADER_SIZE;

    for index in 0..entry_count {
        if position
            .checked_add(ENTRY_HEADER_SIZE)
            .is_none_or(|end| end > file_size)
        {
            return Err(invalid(
                position,
                format!("truncated entry header #{index}"),
            ));
        }

        reader.seek(SeekFrom::Start(position))?;
        let total_size = u64::from(read_u32(reader)?);
        let data_size = u64::from(read_u32(reader)?);
        let name_size = read_u32(reader)?;
        let stored_position = u64::from(read_u32(reader)?);
        let mut reserved = [0_u8; 16];
        reader.read_exact(&mut reserved)?;

        if name_size > MAX_ENTRY_NAME_SIZE {
            return Err(invalid(
                position + 8,
                format!("entry #{index} name is unreasonably large: {name_size} bytes"),
            ));
        }
        if stored_position != position {
            return Err(invalid(
                position + 12,
                format!("entry #{index} stores offset {stored_position}, expected {position}"),
            ));
        }

        let expected_total = ENTRY_HEADER_SIZE
            .checked_add(u64::from(name_size))
            .and_then(|value| value.checked_add(1))
            .and_then(|value| value.checked_add(data_size))
            .ok_or_else(|| invalid(position, format!("entry #{index} size overflow")))?;

        if total_size != expected_total {
            return Err(invalid(
                position,
                format!("entry #{index} has total size {total_size}, expected {expected_total}"),
            ));
        }

        let entry_end = position
            .checked_add(total_size)
            .ok_or_else(|| invalid(position, format!("entry #{index} offset overflow")))?;
        if entry_end > file_size {
            return Err(invalid(
                position,
                format!("entry #{index} extends beyond the archive"),
            ));
        }

        let name_size_usize = usize::try_from(name_size)
            .map_err(|_| invalid(position + 8, "entry name length does not fit in memory"))?;
        let mut name_bytes = vec![0_u8; name_size_usize];
        reader.read_exact(&mut name_bytes)?;
        let terminator = read_byte(reader)?;
        if terminator != 0 {
            return Err(invalid(
                position + ENTRY_HEADER_SIZE + u64::from(name_size),
                format!("entry #{index} name is not NUL-terminated"),
            ));
        }

        let name = decode_name(&name_bytes);
        if name.is_empty() {
            return Err(invalid(
                position + ENTRY_HEADER_SIZE,
                format!("entry #{index} has an empty name"),
            ));
        }

        entries.push(PakEntry {
            index,
            name,
            record_offset: position,
            data_offset: position + ENTRY_HEADER_SIZE + u64::from(name_size) + 1,
            size: data_size,
            total_size,
        });
        position = entry_end;
    }

    if position != file_size {
        return Err(invalid(
            position,
            format!(
                "{} trailing bytes after the final entry",
                file_size - position
            ),
        ));
    }

    Ok((header, entries))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, PakError> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_byte(reader: &mut impl Read) -> Result<u8, PakError> {
    let mut byte = [0_u8; 1];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn decode_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn safe_relative_path(name: &str) -> Result<PathBuf, PakError> {
    let normalized = name.replace('\\', "/");
    let mut output = PathBuf::new();

    for component in normalized.split('/') {
        match component {
            "" | "." => continue,
            ".." => return Err(PakError::UnsafeEntryName(name.to_owned())),
            value if value.contains(':') || value.contains('\0') => {
                return Err(PakError::UnsafeEntryName(name.to_owned()));
            }
            value => output.push(value),
        }
    }

    if output.as_os_str().is_empty() || output.is_absolute() {
        return Err(PakError::UnsafeEntryName(name.to_owned()));
    }

    Ok(output)
}

fn invalid(offset: u64, message: impl Into<String>) -> PakError {
    PakError::InvalidFormat {
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_and_extracts_a_synthetic_archive() {
        let bytes = archive(&[("hello.txt", b"corum"), ("nested/data.bin", &[1, 2, 3])]);
        let temp = unique_temp_directory();
        fs::create_dir_all(&temp).unwrap();
        let pak_path = temp.join("test.pak");
        fs::write(&pak_path, bytes).unwrap();

        let pak = PakArchive::open(&pak_path).unwrap();
        assert_eq!(pak.header().version, 1);
        assert_eq!(pak.entries().len(), 2);
        assert_eq!(pak.entry("HELLO.TXT").unwrap().size, 5);

        let output = temp.join("out");
        assert_eq!(pak.extract_all(&output).unwrap(), 2);
        assert_eq!(fs::read(output.join("hello.txt")).unwrap(), b"corum");
        assert_eq!(fs::read(output.join("nested/data.bin")).unwrap(), [1, 2, 3]);

        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn rejects_a_mismatched_stored_offset() {
        let mut bytes = archive(&[("hello.txt", b"corum")]);
        bytes[(PAK_HEADER_SIZE as usize + 12)..(PAK_HEADER_SIZE as usize + 16)]
            .copy_from_slice(&123_u32.to_le_bytes());
        let mut cursor = io::Cursor::new(bytes);
        let length = cursor.get_ref().len() as u64;

        assert!(matches!(
            parse(&mut cursor, length),
            Err(PakError::InvalidFormat { .. })
        ));
    }

    #[test]
    fn rejects_parent_directory_during_extraction() {
        assert!(matches!(
            safe_relative_path("../outside.txt"),
            Err(PakError::UnsafeEntryName(_))
        ));
        assert!(matches!(
            safe_relative_path("C:\\outside.txt"),
            Err(PakError::UnsafeEntryName(_))
        ));
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&[0_u8; 76]);

        for (name, data) in entries {
            let position = bytes.len() as u32;
            let name_size = name.len() as u32;
            let total_size = ENTRY_HEADER_SIZE as u32 + name_size + 1 + data.len() as u32;
            bytes.extend_from_slice(&total_size.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&name_size.to_le_bytes());
            bytes.extend_from_slice(&position.to_le_bytes());
            bytes.extend_from_slice(&[0_u8; 16]);
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(0);
            bytes.write_all(data).unwrap();
        }

        bytes
    }

    fn unique_temp_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("corum-assets-{}-{nonce}", std::process::id()))
    }
}
