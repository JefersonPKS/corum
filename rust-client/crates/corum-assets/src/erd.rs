//! Tabelas de recursos `.erd` (`CorumResource.erd`, `DefResource.erd`): id de recurso → caminho.
//!
//! **[confirmado]** Layout: `u32 contagem`, `contagem × (u32 id, u32 tamanho)`, depois os caminhos
//! concatenados, sem terminador, cada um com o seu tamanho. O total fecha exatamente com o arquivo
//! nos dois arquivos do cliente (1.140 e 181 entradas). O byte alto do id é a categoria
//! (`0x0A` = UI, `0x01` = modelos de personagem, ...): **[hipótese]** por causa das amostras.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErdError(String);

impl fmt::Display for ErdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid ERD: {}", self.0)
    }
}

impl std::error::Error for ErdError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceEntry {
    pub id: u32,
    /// Caminho como no original (`.\Data\UI\title.tif`), em bytes crus convertidos sem perda para ASCII.
    pub path: String,
}

impl ResourceEntry {
    /// Categoria do recurso: byte alto do id.
    #[must_use]
    pub fn category(&self) -> u8 {
        self.id.to_be_bytes()[0]
    }
}

pub fn parse(bytes: &[u8]) -> Result<Vec<ResourceEntry>, ErdError> {
    let read_u32 = |offset: usize| -> Result<usize, ErdError> {
        bytes
            .get(offset..offset + 4)
            .map(|raw| u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize)
            .ok_or_else(|| ErdError(format!("truncated at offset {offset}")))
    };
    let count = read_u32(0)?;
    let table_end = count
        .checked_mul(8)
        .and_then(|table| table.checked_add(4))
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| ErdError(format!("{count} entries do not fit in the file")))?;
    let mut position = table_end;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let id = read_u32(4 + index * 8)?;
        let length = read_u32(8 + index * 8)?;
        let raw = bytes
            .get(position..position + length)
            .ok_or_else(|| ErdError(format!("path {index} runs past the end of the file")))?;
        entries.push(ResourceEntry {
            id: id as u32,
            path: String::from_utf8_lossy(raw).into_owned(),
        });
        position += length;
    }
    if position != bytes.len() {
        return Err(ErdError(format!(
            "{} trailing bytes after the last path",
            bytes.len() - position
        )));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut bytes = 2u32.to_le_bytes().to_vec();
        for (id, length) in [(0x0A00_0001u32, 5u32), (0x0100_03E9, 3)] {
            bytes.extend(id.to_le_bytes());
            bytes.extend(length.to_le_bytes());
        }
        bytes.extend(b"a.tgab.m");
        bytes
    }

    #[test]
    fn parses_ids_and_paths() {
        let entries = parse(&sample()).unwrap();
        assert_eq!(entries[0].path, "a.tga");
        assert_eq!(entries[0].category(), 0x0A);
        assert_eq!(entries[1].path, "b.m");
        assert_eq!(entries[1].id, 0x0100_03E9);
    }

    #[test]
    fn rejects_size_mismatches() {
        let mut bytes = sample();
        bytes.push(0);
        assert!(parse(&bytes).is_err());
        bytes.truncate(bytes.len() - 3);
        assert!(parse(&bytes).is_err());
        assert!(parse(&[9, 0, 0, 0]).is_err());
        assert!(parse(&[1]).is_err());
    }
}
