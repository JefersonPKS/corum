//! Tabelas `.cdb` de `Data\Manager` (dados de jogo cifrados).
//!
//! **[confirmado]** O cliente original as lê em `DecodeCDBData` (`CorumOnlineProject/GameControl.cpp`):
//! o arquivo é `u32 tamanho` seguido de `tamanho` bytes; cada byte `i` é XOR com
//! `(CHAVE[i % 21] + 2) & 0xFF`, com a chave `DECODE_KEY` de `GameControl.h` (bytes crus, não texto).
//! Depois de decifrado, o conteúdo é uma tabela de registros de tamanho fixo (`#pragma pack(1)`), ou,
//! nas tabelas de texto, um "pool" com assinatura `Oops` (`CMessagePool`).

use std::fmt;

/// `DECODE_KEY` de `GameControl.h`, em bytes crus (o original é texto GBK).
pub const DECODE_KEY: [u8; 21] = [
    0xCE, 0xFD, 0xDC, 0xB5, 0xE2, 0xB3, 0xF4, 0xB8, 0xEE, 0xA6, 0x2C, 0xCD, 0xE9, 0xEB, 0xE2, 0xD8,
    0xBF, 0xD8, 0xBF, 0xE1, 0xA8,
];
pub const DECODE_SUBKEY: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdbError(String);

impl CdbError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CdbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid CDB: {}", self.0)
    }
}

impl std::error::Error for CdbError {}

/// Decifra um `.cdb` inteiro e devolve o corpo (sem o `u32` de tamanho).
///
/// O original copia exatamente `tamanho` bytes; se o arquivo trouxer menos, é erro aqui.
pub fn decode(file: &[u8]) -> Result<Vec<u8>, CdbError> {
    let (length, body) = file
        .split_first_chunk::<4>()
        .ok_or_else(|| CdbError::new("file is smaller than the 4-byte length"))?;
    let length = u32::from_le_bytes(*length) as usize;
    if body.len() < length {
        return Err(CdbError::new(format!(
            "declared {length} bytes but only {} follow",
            body.len()
        )));
    }
    Ok(body[..length]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            byte ^ DECODE_KEY[index % DECODE_KEY.len()].wrapping_add(DECODE_SUBKEY)
        })
        .collect())
}

/// O inverso de `decode`: cifra um corpo e antepõe o tamanho (a cifra é XOR, portanto simétrica).
#[must_use]
pub fn encode(body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len()).expect("a CDB body is smaller than 4 GiB");
    let mut file = length.to_le_bytes().to_vec();
    file.extend(body.iter().enumerate().map(|(index, byte)| {
        byte ^ DECODE_KEY[index % DECODE_KEY.len()].wrapping_add(DECODE_SUBKEY)
    }));
    file
}

/// Registro de tamanho fixo de uma tabela (layout `pack(1)` do cliente).
pub trait Record: Sized {
    /// Tamanho em bytes de um registro.
    const SIZE: usize;
    /// Nome do arquivo em `Data\Manager`.
    const FILE: &'static str;

    fn parse(bytes: &[u8]) -> Self;
}

/// Lê todos os registros de um corpo já decifrado; o tamanho tem de ser múltiplo do registro.
pub fn parse_table<T: Record>(decoded: &[u8]) -> Result<Vec<T>, CdbError> {
    if !decoded.len().is_multiple_of(T::SIZE) {
        return Err(CdbError::new(format!(
            "{} bytes is not a multiple of the {}-byte record of {}",
            decoded.len(),
            T::SIZE,
            T::FILE
        )));
    }
    // `as_chunks` exige tamanho const genérico; `T::SIZE` de um trait não serve no Rust estável.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    let rows = decoded.chunks_exact(T::SIZE).map(T::parse).collect();
    Ok(rows)
}

/// Texto de tamanho fixo terminado em NUL (bytes crus: o cliente chinês usa GBK).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FixedText(pub Vec<u8>);

impl FixedText {
    fn from_field(field: &[u8]) -> Self {
        let end = field
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(field.len());
        Self(field[..end].to_vec())
    }

    /// Só é fiel para ASCII; o resto vira `U+FFFD`. Serve para listagens.
    #[must_use]
    pub fn lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    let mut raw = [0; 8];
    raw.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(raw)
}

/// `Level.cdb`: experiência necessária por nível do jogador.
///
/// **[confirmado]** 9 bytes: `u8 nível`, `u64 exp` (200 níveis). O `SLEVEL_EXP` do código-fonte
/// (`u8`+`u32` = 5 bytes) não bate com este arquivo; vale o arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelExp {
    pub level: u8,
    pub exp: u64,
}

impl Record for LevelExp {
    const SIZE: usize = 9;
    const FILE: &'static str = "Level.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            level: bytes[0],
            exp: u64_at(bytes, 1),
        }
    }
}

/// `GuardianLevel.cdb` / `GuardianExp.cdb`: experiência por nível do guardião (`SLEVEL_EXP`).
///
/// **[confirmado]** 5 bytes: `u8 nível`, `u32 exp` (200 níveis), como no código-fonte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardianLevelExp {
    pub level: u8,
    pub exp: u32,
}

impl Record for GuardianLevelExp {
    const SIZE: usize = 5;
    const FILE: &'static str = "GuardianLevel.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            level: bytes[0],
            exp: u32_at(bytes, 1),
        }
    }
}

/// `npctable.cdb` (`NPC_TABLE`): nome, tipo de loja e 3 falas de cada NPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcTable {
    pub id: u32,
    pub name: FixedText,
    pub kind: u32,
    pub messages: [FixedText; 3],
}

impl Record for NpcTable {
    const SIZE: usize = 808;
    const FILE: &'static str = "npctable.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: u32_at(bytes, 0),
            name: FixedText::from_field(&bytes[4..36]),
            kind: u32_at(bytes, 36),
            messages: std::array::from_fn(|index| {
                FixedText::from_field(&bytes[40 + index * 256..40 + (index + 1) * 256])
            }),
        }
    }
}

/// `CPTable.cdb` (`CPTable`): habilidades de "CP" (nomes, animações, sons e 5 valores).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpTable {
    pub id: u16,
    pub korean_name: FixedText,
    pub english_name: FixedText,
    pub description: FixedText,
    pub class: u8,
    pub rate: u16,
    pub animation_1: u16,
    pub animation_2: u16,
    pub sound_1: u16,
    pub sound_2: u16,
    pub apply_time: u16,
    pub party_use: u8,
    /// Pares `(id, valor)`.
    pub values: [(u16, u16); 5],
}

impl Record for CpTable {
    const SIZE: usize = 249;
    const FILE: &'static str = "CPTable.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: u16_at(bytes, 0),
            korean_name: FixedText::from_field(&bytes[2..52]),
            english_name: FixedText::from_field(&bytes[52..87]),
            description: FixedText::from_field(&bytes[87..215]),
            class: bytes[215],
            rate: u16_at(bytes, 216),
            animation_1: u16_at(bytes, 218),
            animation_2: u16_at(bytes, 220),
            sound_1: u16_at(bytes, 222),
            sound_2: u16_at(bytes, 224),
            apply_time: u16_at(bytes, 226),
            party_use: bytes[228],
            values: std::array::from_fn(|index| {
                (
                    u16_at(bytes, 229 + index * 4),
                    u16_at(bytes, 231 + index * 4),
                )
            }),
        }
    }
}

/// `ItemStore.cdb` (`ITEM_STORE`): item, tipo e mapa da loja.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStore {
    pub item_id: u16,
    pub kind: u8,
    pub map_id: u16,
}

impl Record for ItemStore {
    const SIZE: usize = 5;
    const FILE: &'static str = "ItemStore.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            item_id: u16_at(bytes, 0),
            kind: bytes[2],
            map_id: u16_at(bytes, 3),
        }
    }
}

/// `ItemResource.cdb` (`SITEM_RESOURCE`): ícone e modelo 3D de cada item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemResource {
    pub id: u16,
    pub icon_file: FixedText,
    pub model_file: FixedText,
    pub icon_start_index: u8,
    pub icon_count: u8,
    pub model_count: u16,
    pub resource_type: u8,
    pub preload: u8,
    pub animation: u8,
}

impl Record for ItemResource {
    const SIZE: usize = 89;
    const FILE: &'static str = "ItemResource.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: u16_at(bytes, 0),
            icon_file: FixedText::from_field(&bytes[2..42]),
            model_file: FixedText::from_field(&bytes[42..82]),
            icon_start_index: bytes[82],
            icon_count: bytes[83],
            model_count: u16_at(bytes, 84),
            resource_type: bytes[86],
            preload: bytes[87],
            animation: bytes[88],
        }
    }
}

/// `SkillResource.cdb` (`SSKILL_RESOURCE`): ícones e tipo de cada habilidade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillResource {
    pub id: u16,
    pub skill_type: u8,
    pub icon_file: FixedText,
    pub icon_index: u16,
    pub active_icon_file: FixedText,
    pub active_icon_index: u16,
    pub kind: u8,
    pub kind_index: u8,
    pub kind_position: u8,
}

impl Record for SkillResource {
    const SIZE: usize = 50;
    const FILE: &'static str = "SkillResource.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: u16_at(bytes, 0),
            skill_type: bytes[2],
            icon_file: FixedText::from_field(&bytes[3..23]),
            icon_index: u16_at(bytes, 23),
            active_icon_file: FixedText::from_field(&bytes[25..45]),
            active_icon_index: u16_at(bytes, 45),
            kind: bytes[47],
            kind_index: bytes[48],
            kind_position: bytes[49],
        }
    }
}

/// `ItemOption.cdb` (`ITEM_OPTION`): até 4 linhas de texto de opção por item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOption {
    pub id: u16,
    pub count: u8,
    pub display_flags: [u8; 4],
    pub options: [FixedText; 4],
}

impl Record for ItemOption {
    const SIZE: usize = 263;
    const FILE: &'static str = "ItemOption.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: u16_at(bytes, 0),
            count: bytes[2],
            display_flags: [bytes[3], bytes[4], bytes[5], bytes[6]],
            options: std::array::from_fn(|index| {
                FixedText::from_field(&bytes[7 + index * 64..7 + (index + 1) * 64])
            }),
        }
    }
}

/// `Help.cdb`/`HelpInfo.cdb` (`SHELP_INFO`): dicas de ajuda com posição na tela.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpInfo {
    pub id: u16,
    pub text: FixedText,
    pub left: u16,
    pub top: u16,
    pub link_text_id: u16,
    pub kind: u8,
}

impl Record for HelpInfo {
    const SIZE: usize = 73;
    const FILE: &'static str = "Help.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: u16_at(bytes, 0),
            text: FixedText::from_field(&bytes[2..66]),
            left: u16_at(bytes, 66),
            top: u16_at(bytes, 68),
            link_text_id: u16_at(bytes, 70),
            kind: bytes[72],
        }
    }
}

/// `DungeonProductionItemMinMax.cdb`: faixa de itens produzida por tipo de dungeon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DungeonProductionItemRange {
    pub id: u8,
    pub item_id_min: u16,
    pub item_id_max: u16,
    pub item_id_default: u16,
}

impl Record for DungeonProductionItemRange {
    const SIZE: usize = 7;
    const FILE: &'static str = "DungeonProductionItemMinMax.cdb";

    fn parse(bytes: &[u8]) -> Self {
        Self {
            id: bytes[0],
            item_id_min: u16_at(bytes, 1),
            item_id_max: u16_at(bytes, 3),
            item_id_default: u16_at(bytes, 5),
        }
    }
}

/// `BaseClassInfo.cdb` (`BASE_CLASS_INFO`): máximos base por classe (5 × `i32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseClassInfo {
    pub max_aura: i32,
    pub max_divine: i32,
    pub max_summon: i32,
    pub max_chakra: i32,
    pub max_magic: i32,
}

impl Record for BaseClassInfo {
    const SIZE: usize = 20;
    const FILE: &'static str = "BaseClassInfo.cdb";

    fn parse(bytes: &[u8]) -> Self {
        let value = |index: usize| u32_at(bytes, index * 4).cast_signed();
        Self {
            max_aura: value(0),
            max_divine: value(1),
            max_summon: value(2),
            max_chakra: value(3),
            max_magic: value(4),
        }
    }
}

/// Pool de textos (`message.cdb`, `Cmd_Message.cdb`, `Emoticon.cdb`, filtros): assinatura `Oops`.
///
/// **[confirmado]** corpo decifrado = `u32 (sobra, ignorado)`, cabeçalho `char[4] "Oops"`,
/// `u32 contagem`, `u32 tamanho_dos_textos`, `contagem × (u32 id, u32 posição)`, textos NUL-terminados.
/// O cliente indexa por posição na tabela (`GetMessage(índice)`), não pelo `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPool {
    pub entries: Vec<TextEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEntry {
    pub id: u32,
    pub text: FixedText,
}

impl TextPool {
    pub fn parse(decoded: &[u8]) -> Result<Self, CdbError> {
        const HEADER: usize = 4 + 12;
        if decoded.len() < HEADER || &decoded[4..8] != b"Oops" {
            return Err(CdbError::new("missing the `Oops` text-pool signature"));
        }
        let count = u32_at(decoded, 8) as usize;
        let text_size = u32_at(decoded, 12) as usize;
        let table_end = count
            .checked_mul(8)
            .and_then(|table| table.checked_add(HEADER))
            .ok_or_else(|| CdbError::new("entry count overflows"))?;
        if decoded.len() != table_end + text_size {
            return Err(CdbError::new(format!(
                "size {} does not match header + {count} entries + {text_size} text bytes",
                decoded.len()
            )));
        }
        let texts = &decoded[table_end..];
        let entries = (0..count)
            .map(|index| {
                let row = HEADER + index * 8;
                let position = u32_at(decoded, row + 4) as usize;
                if position > texts.len() {
                    return Err(CdbError::new(format!(
                        "entry {index} points past the text block"
                    )));
                }
                Ok(TextEntry {
                    id: u32_at(decoded, row),
                    text: FixedText::from_field(&texts[position..]),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_round_trips_across_key_boundary() {
        let body: Vec<u8> = (0..100).collect();
        assert_eq!(decode(&encode(&body)).unwrap(), body);
    }

    #[test]
    fn decode_rejects_truncated_files() {
        let mut file = encode(&[1, 2, 3, 4]);
        file.pop();
        assert!(decode(&file).is_err());
        assert!(decode(&[1, 2]).is_err());
    }

    #[test]
    fn level_table_parses_nine_byte_records() {
        let body = [1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0x50, 0, 0, 0, 0, 0, 0, 0];
        let levels = parse_table::<LevelExp>(&body).unwrap();
        assert_eq!(
            levels,
            [
                LevelExp { level: 1, exp: 0 },
                LevelExp { level: 2, exp: 80 }
            ]
        );
        assert!(parse_table::<LevelExp>(&body[..10]).is_err());
    }

    /// Confere os tamanhos de registro contra os arquivos reais; pula sem `CORUM_DATA`
    /// (pasta `Data` do cliente, a mesma das variáveis do sandbox).
    #[test]
    fn typed_tables_match_the_real_client_files() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let manager = std::path::Path::new(&data).join("Manager");
        fn count<T: Record>(manager: &std::path::Path, file: &str) -> usize {
            let bytes = std::fs::read(manager.join(file)).unwrap();
            parse_table::<T>(&decode(&bytes).unwrap()).unwrap().len()
        }
        assert_eq!(count::<LevelExp>(&manager, "Level.cdb"), 200);
        assert_eq!(
            count::<GuardianLevelExp>(&manager, "GuardianLevel.cdb"),
            200
        );
        assert_eq!(count::<ItemResource>(&manager, "ItemResource.cdb"), 3058);
        assert_eq!(count::<SkillResource>(&manager, "SkillResource.cdb"), 110);
        assert_eq!(count::<ItemOption>(&manager, "ItemOption.cdb"), 1117);
        assert_eq!(count::<ItemStore>(&manager, "ItemStore.cdb"), 1660);
        assert_eq!(count::<NpcTable>(&manager, "npctable.cdb"), 112);
        assert_eq!(count::<CpTable>(&manager, "CPTable.cdb"), 31);
        assert_eq!(count::<HelpInfo>(&manager, "Help.cdb"), 732);
        let bytes = std::fs::read(manager.join("message.cdb")).unwrap();
        let pool = TextPool::parse(&decode(&bytes).unwrap()).unwrap();
        assert_eq!(pool.entries.len(), 1823);
    }

    #[test]
    fn text_pool_reads_entries_by_position() {
        let mut body = vec![0; 4];
        body.extend(b"Oops");
        body.extend(2u32.to_le_bytes());
        body.extend(7u32.to_le_bytes());
        for (id, position) in [(10u32, 0u32), (11, 3)] {
            body.extend(id.to_le_bytes());
            body.extend(position.to_le_bytes());
        }
        body.extend(b"ab\0cd\0\0");
        let pool = TextPool::parse(&body).unwrap();
        assert_eq!(pool.entries[0].id, 10);
        assert_eq!(pool.entries[1].text.lossy(), "cd");
        body.push(0);
        assert!(TextPool::parse(&body).is_err());
    }
}
