//! Codificação dos pacotes de sessão do Corum Online.
//!
//! Os servidores antigos serializam structs C++ com `#pragma pack(1)`. Este crate mantém a
//! mesma forma na rede, mas lê e escreve cada campo explicitamente em little-endian, sem
//! referências `packed` e sem `unsafe`. A camada TCP/framing será adicionada acima dele.

use std::fmt;

pub const STATUS_LOGIN: u8 = 1;
pub const STATUS_CHARACTER_SELECT: u8 = 3;

pub const CMD_LOGIN: u8 = 0;
pub const CMD_LOGIN_FAIL: u8 = 1;
pub const CMD_LOGIN_SUCCESS: u8 = 2;
pub const CMD_ENCRYPTION_KEY: u8 = 13;

pub const CMD_CHARACTER_SELECT: u8 = 0;
pub const CMD_CREATE_NEW_CHARACTER: u8 = 1;
pub const CMD_CREATE_CHARACTER_SUCCESS: u8 = 2;
pub const CMD_CREATE_CHARACTER_FAIL: u8 = 4;
pub const CMD_WORLD_USER_INFO: u8 = 5;
pub const CMD_CHARACTER_SELECT_FAIL: u8 = 9;
pub const CMD_CONNECT_WORLD_SERVER: u8 = 10;
pub const CMD_WORLD_LOGIN: u8 = 11;
pub const CMD_WORLD_LOGIN_FAIL: u8 = 12;

pub const NATIONAL_CODE_KOREA: u8 = 0;
pub const NATIONAL_CODE_JAPAN: u8 = 1;
pub const NATIONAL_CODE_CHINA: u8 = 2;
pub const NATIONAL_CODE_TAIWAN: u8 = 3;

pub const MAX_ID_LENGTH: usize = 20;
pub const MAX_PASSWORD_LENGTH: usize = 20;
pub const MAX_CHARACTER_NAME_LENGTH: usize = 20;
pub const ENCRYPTION_KEY_LENGTH: usize = 10;
pub const CHARACTER_SUMMARY_SIZE: usize = 82;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Truncated {
        needed: usize,
        available: usize,
    },
    UnexpectedLength {
        expected: String,
        actual: usize,
    },
    UnexpectedHeader {
        status: u8,
        command: u8,
    },
    InvalidSignature([u8; 4]),
    InvalidCount {
        field: &'static str,
        value: usize,
        maximum: usize,
    },
    StringTooLong {
        field: &'static str,
        maximum: usize,
        actual: usize,
    },
    InvalidUtf8 {
        field: &'static str,
    },
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    formatter,
                    "packet truncated: need {needed} bytes, have {available}"
                )
            }
            Self::UnexpectedLength { expected, actual } => {
                write!(
                    formatter,
                    "unexpected packet length {actual}, expected {expected}"
                )
            }
            Self::UnexpectedHeader { status, command } => {
                write!(
                    formatter,
                    "unexpected packet header status={status} command={command}"
                )
            }
            Self::InvalidSignature(signature) => {
                write!(formatter, "invalid login signature: {:?}", signature)
            }
            Self::InvalidCount {
                field,
                value,
                maximum,
            } => {
                write!(
                    formatter,
                    "invalid {field} count {value}; maximum is {maximum}"
                )
            }
            Self::StringTooLong {
                field,
                maximum,
                actual,
            } => {
                write!(formatter, "{field} is {actual} bytes; maximum is {maximum}")
            }
            Self::InvalidUtf8 { field } => write!(formatter, "{field} is not valid UTF-8"),
        }
    }
}

impl std::error::Error for WireError {}

fn require(bytes: &[u8], needed: usize) -> Result<(), WireError> {
    if bytes.len() < needed {
        return Err(WireError::Truncated {
            needed,
            available: bytes.len(),
        });
    }
    Ok(())
}

fn header(bytes: &[u8], status: u8, command: u8) -> Result<(), WireError> {
    require(bytes, 2)?;
    if bytes[0] != status || bytes[1] != command {
        return Err(WireError::UnexpectedHeader {
            status: bytes[0],
            command: bytes[1],
        });
    }
    Ok(())
}

fn fixed_bytes(field: &'static str, value: &str, width: usize) -> Result<Vec<u8>, WireError> {
    let bytes = value.as_bytes();
    if bytes.len() >= width {
        return Err(WireError::StringTooLong {
            field,
            maximum: width - 1,
            actual: bytes.len(),
        });
    }
    let mut output = vec![0; width];
    output[..bytes.len()].copy_from_slice(bytes);
    Ok(output)
}

fn fixed_string(field: &'static str, bytes: &[u8]) -> Result<String, WireError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).map_err(|_| WireError::InvalidUtf8 { field })
}

fn read_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, WireError> {
    require(&bytes[*offset..], 2)?;
    let value = u16::from_le_bytes([bytes[*offset], bytes[*offset + 1]]);
    *offset += 2;
    Ok(value)
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, WireError> {
    require(&bytes[*offset..], 4)?;
    let value = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    Ok(value)
}

fn read_fixed<const N: usize>(bytes: &[u8], offset: &mut usize) -> Result<[u8; N], WireError> {
    require(&bytes[*offset..], N)?;
    let value = bytes[*offset..*offset + N].try_into().unwrap();
    *offset += N;
    Ok(value)
}

/// `CTWS_LOGIN` (51 bytes without packet encryption, 61 with the optional client key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRequest {
    pub nation_code: u8,
    pub version: u32,
    pub id: String,
    pub password: String,
    pub client_key: Option<[u8; ENCRYPTION_KEY_LENGTH]>,
}

impl LoginRequest {
    #[must_use]
    pub fn new(id: impl Into<String>, password: impl Into<String>, version: u32) -> Self {
        Self {
            nation_code: NATIONAL_CODE_KOREA,
            version,
            id: id.into(),
            password: password.into(),
            client_key: None,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let id = fixed_bytes("login id", &self.id, MAX_ID_LENGTH)?;
        let password = fixed_bytes("password", &self.password, MAX_PASSWORD_LENGTH)?;
        let mut output = Vec::with_capacity(if self.client_key.is_some() { 61 } else { 51 });
        output.extend([STATUS_LOGIN, CMD_LOGIN]);
        output.extend(b"@SAD");
        output.push(self.nation_code);
        output.extend(self.version.to_le_bytes());
        output.extend(id);
        output.extend(password);
        if let Some(key) = self.client_key {
            output.extend(key);
        }
        Ok(output)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != 51 && bytes.len() != 61 {
            return Err(WireError::UnexpectedLength {
                expected: "51 or 61".to_owned(),
                actual: bytes.len(),
            });
        }
        header(bytes, STATUS_LOGIN, CMD_LOGIN)?;
        let signature: [u8; 4] = bytes[2..6].try_into().unwrap();
        if signature != *b"@SAD" {
            return Err(WireError::InvalidSignature(signature));
        }
        let mut offset = 6;
        let nation_code = bytes[offset];
        offset += 1;
        let version = read_u32(bytes, &mut offset)?;
        let id = fixed_string(
            "login id",
            &read_fixed::<MAX_ID_LENGTH>(bytes, &mut offset)?,
        )?;
        let password = fixed_string(
            "password",
            &read_fixed::<MAX_PASSWORD_LENGTH>(bytes, &mut offset)?,
        )?;
        let client_key = if bytes.len() == 61 {
            Some(read_fixed::<ENCRYPTION_KEY_LENGTH>(bytes, &mut offset)?)
        } else {
            None
        };
        Ok(Self {
            nation_code,
            version,
            id,
            password,
            client_key,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginFailure {
    pub result: u8,
    pub extra_data: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncryptionKey {
    pub server_key: [u8; ENCRYPTION_KEY_LENGTH],
}

impl EncryptionKey {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        header(bytes, STATUS_LOGIN, CMD_ENCRYPTION_KEY)?;
        if bytes.len() != 2 + ENCRYPTION_KEY_LENGTH {
            return Err(WireError::UnexpectedLength {
                expected: (2 + ENCRYPTION_KEY_LENGTH).to_string(),
                actual: bytes.len(),
            });
        }
        Ok(Self {
            server_key: bytes[2..].try_into().unwrap(),
        })
    }
}

impl LoginFailure {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        header(bytes, STATUS_LOGIN, CMD_LOGIN_FAIL)?;
        if bytes.len() != 7 {
            return Err(WireError::UnexpectedLength {
                expected: "7".to_owned(),
                actual: bytes.len(),
            });
        }
        Ok(Self {
            result: bytes[2],
            extra_data: u32::from_le_bytes(bytes[3..7].try_into().unwrap()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterSummary {
    pub character_index: u32,
    pub name: String,
    pub character_slot: u16,
    pub head: u16,
    pub class: u16,
    pub grade: u16,
    pub level: u32,
    pub experience: u32,
    pub ego: u32,
    pub strength: u32,
    pub vitality: u32,
    pub dexterity: u32,
    pub intelligence: u32,
    pub honor: u32,
    pub luck: u32,
    pub recent_world_map: u32,
    pub left_hand: u16,
    pub right_hand: u16,
    pub helmet: u16,
    pub mantle: u16,
    pub mail: u16,
}

impl CharacterSummary {
    fn decode(bytes: &[u8], offset: &mut usize) -> Result<Self, WireError> {
        let character_index = read_u32(bytes, offset)?;
        let name = fixed_string(
            "character name",
            &read_fixed::<MAX_CHARACTER_NAME_LENGTH>(bytes, offset)?,
        )?;
        Ok(Self {
            character_index,
            name,
            character_slot: read_u16(bytes, offset)?,
            head: read_u16(bytes, offset)?,
            class: read_u16(bytes, offset)?,
            grade: read_u16(bytes, offset)?,
            level: read_u32(bytes, offset)?,
            experience: read_u32(bytes, offset)?,
            ego: read_u32(bytes, offset)?,
            strength: read_u32(bytes, offset)?,
            vitality: read_u32(bytes, offset)?,
            dexterity: read_u32(bytes, offset)?,
            intelligence: read_u32(bytes, offset)?,
            honor: read_u32(bytes, offset)?,
            luck: read_u32(bytes, offset)?,
            recent_world_map: read_u32(bytes, offset)?,
            left_hand: read_u16(bytes, offset)?,
            right_hand: read_u16(bytes, offset)?,
            helmet: read_u16(bytes, offset)?,
            mantle: read_u16(bytes, offset)?,
            mail: read_u16(bytes, offset)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSuccess {
    pub adult_mode: u8,
    pub characters: Vec<CharacterSummary>,
}

impl LoginSuccess {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        header(bytes, STATUS_LOGIN, CMD_LOGIN_SUCCESS)?;
        require(bytes, 4)?;
        let count = usize::from(bytes[3]);
        if count > 4 {
            return Err(WireError::InvalidCount {
                field: "characters",
                value: count,
                maximum: 4,
            });
        }
        let expected = 4 + count * CHARACTER_SUMMARY_SIZE;
        if bytes.len() != expected {
            return Err(WireError::UnexpectedLength {
                expected: expected.to_string(),
                actual: bytes.len(),
            });
        }
        let mut offset = 4;
        let mut characters = Vec::with_capacity(count);
        for _ in 0..count {
            characters.push(CharacterSummary::decode(bytes, &mut offset)?);
        }
        Ok(Self {
            adult_mode: bytes[2],
            characters,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterSelectRequest {
    pub character_index: u8,
}

impl CharacterSelectRequest {
    #[must_use]
    pub const fn new(character_index: u8) -> Self {
        Self { character_index }
    }

    #[must_use]
    pub const fn encode(self) -> [u8; 3] {
        [
            STATUS_CHARACTER_SELECT,
            CMD_CHARACTER_SELECT,
            self.character_index,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCharacterRequest {
    pub character_slot: u8,
    pub class: u8,
    pub head: u16,
    pub start_map: u16,
    pub name: String,
}

impl CreateCharacterRequest {
    pub fn encode(&self) -> Result<[u8; 28], WireError> {
        let name = fixed_bytes("character name", &self.name, MAX_CHARACTER_NAME_LENGTH)?;
        let mut output = [0_u8; 28];
        output[..2].copy_from_slice(&[STATUS_CHARACTER_SELECT, CMD_CREATE_NEW_CHARACTER]);
        output[2] = self.character_slot;
        output[3] = self.class;
        output[4..6].copy_from_slice(&self.head.to_le_bytes());
        output[6..8].copy_from_slice(&self.start_map.to_le_bytes());
        output[8..].copy_from_slice(&name);
        Ok(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateCharacterSuccess {
    pub strength: u32,
    pub vitality: u32,
    pub dexterity: u32,
    pub intelligence: u32,
    pub ego: u32,
}

impl CreateCharacterSuccess {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        header(bytes, STATUS_CHARACTER_SELECT, CMD_CREATE_CHARACTER_SUCCESS)?;
        if bytes.len() != 22 {
            return Err(WireError::UnexpectedLength {
                expected: "22".to_owned(),
                actual: bytes.len(),
            });
        }
        let mut offset = 2;
        Ok(Self {
            strength: read_u32(bytes, &mut offset)?,
            vitality: read_u32(bytes, &mut offset)?,
            dexterity: read_u32(bytes, &mut offset)?,
            intelligence: read_u32(bytes, &mut offset)?,
            ego: read_u32(bytes, &mut offset)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterSelectFailure {
    pub error_code: u8,
}

/// `ASTC_CONNECT_WORLD_SERVER` (21 bytes), sent by LoginAgent after the selected character is
/// accepted by the WorldServer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectWorldServer {
    pub ip: u32,
    pub port: u16,
    pub property_id: u32,
    pub character_index: u32,
    pub serial_code: u32,
    pub event_flag: u8,
}

impl ConnectWorldServer {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        header(bytes, STATUS_CHARACTER_SELECT, CMD_CONNECT_WORLD_SERVER)?;
        if bytes.len() != 21 {
            return Err(WireError::UnexpectedLength {
                expected: "21".to_owned(),
                actual: bytes.len(),
            });
        }
        let mut offset = 2;
        Ok(Self {
            ip: read_u32(bytes, &mut offset)?,
            port: read_u16(bytes, &mut offset)?,
            property_id: read_u32(bytes, &mut offset)?,
            character_index: read_u32(bytes, &mut offset)?,
            serial_code: read_u32(bytes, &mut offset)?,
            event_flag: bytes[offset],
        })
    }
}

impl CharacterSelectFailure {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        header(bytes, STATUS_CHARACTER_SELECT, CMD_CHARACTER_SELECT_FAIL)?;
        if bytes.len() != 3 {
            return Err(WireError::UnexpectedLength {
                expected: "3".to_owned(),
                actual: bytes.len(),
            });
        }
        Ok(Self {
            error_code: bytes[2],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_without_encryption_matches_the_cxx_size() {
        let packet = LoginRequest::new("tester", "secret", 0x0703_2101);
        let encoded = packet.encode().unwrap();
        assert_eq!(encoded.len(), 51);
        assert_eq!(&encoded[..6], &[1, 0, b'@', b'S', b'A', b'D']);
        assert_eq!(LoginRequest::decode(&encoded).unwrap(), packet);
    }

    #[test]
    fn login_with_client_key_is_the_encrypted_build_size() {
        let mut packet = LoginRequest::new("id", "pw", 1);
        packet.client_key = Some([0xA5; ENCRYPTION_KEY_LENGTH]);
        let encoded = packet.encode().unwrap();
        assert_eq!(encoded.len(), 61);
        assert_eq!(LoginRequest::decode(&encoded).unwrap(), packet);
    }

    #[test]
    fn character_packets_match_the_size_reporter() {
        assert_eq!(CharacterSelectRequest::new(2).encode(), [3, 0, 2]);
        let packet = CreateCharacterRequest {
            character_slot: 1,
            class: 4,
            head: 1003,
            start_map: 1100,
            name: "Aria".to_owned(),
        };
        assert_eq!(packet.encode().unwrap().len(), 28);
    }

    #[test]
    fn connect_world_server_decodes_the_loginagent_handoff() {
        let bytes = [
            STATUS_CHARACTER_SELECT,
            CMD_CONNECT_WORLD_SERVER,
            127,
            0,
            0,
            1,
            0x71,
            0x32,
            0x01,
            0x00,
            0x00,
            0x00,
            0x02,
            0x00,
            0x00,
            0x00,
            0x03,
            0x00,
            0x00,
            0x00,
            0x04,
        ];
        let packet = ConnectWorldServer::decode(&bytes).unwrap();
        assert_eq!(packet.ip, 0x0100_007F);
        assert_eq!(packet.port, 0x3271);
        assert_eq!(packet.property_id, 1);
        assert_eq!(packet.character_index, 2);
        assert_eq!(packet.serial_code, 3);
        assert_eq!(packet.event_flag, 4);
    }

    #[test]
    fn login_success_decodes_four_packed_character_summaries() {
        let mut bytes = vec![STATUS_LOGIN, CMD_LOGIN_SUCCESS, 0, 1];
        bytes.extend(7_u32.to_le_bytes());
        bytes.extend(b"Knight\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(1001_u16.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(0_u16.to_le_bytes());
        for value in 1_u32..=10 {
            bytes.extend(value.to_le_bytes());
        }
        for value in [2400_u16, 1, 2000, 0, 2200] {
            bytes.extend(value.to_le_bytes());
        }
        assert_eq!(bytes.len(), 4 + CHARACTER_SUMMARY_SIZE);
        let result = LoginSuccess::decode(&bytes).unwrap();
        assert_eq!(result.characters[0].name, "Knight");
        assert_eq!(result.characters[0].recent_world_map, 10);
        assert_eq!(result.characters[0].mail, 2200);
    }

    #[test]
    fn malformed_packets_are_rejected_before_field_access() {
        assert!(matches!(
            LoginRequest::decode(&[STATUS_LOGIN, CMD_LOGIN]),
            Err(WireError::UnexpectedLength { .. })
        ));
        assert!(matches!(
            LoginSuccess::decode(&[STATUS_LOGIN, CMD_LOGIN_SUCCESS, 0, 5]),
            Err(WireError::InvalidCount { .. })
        ));
        assert!(matches!(
            CreateCharacterRequest {
                character_slot: 0,
                class: 1,
                head: 1001,
                start_map: 1,
                name: "12345678901234567890".to_owned(),
            }
            .encode(),
            Err(WireError::StringTooLong { .. })
        ));
    }
}
