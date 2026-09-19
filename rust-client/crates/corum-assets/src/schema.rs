//! Esquemas de tabelas de registros fixos: colunas nomeadas ↔ bytes ↔ TSV.
//!
//! Serve ao "System" editável (como a pasta `System` do Lineage 2): cada tabela `.cdb` decifrada vira
//! um `.tsv` com cabeçalho e volta a bytes idênticos (`from_tsv`), que `cdb::encode` cifra de novo.
//!
//! **Texto:** campos de texto guardam bytes crus (o cliente chinês usa GBK). No TSV cada byte vira um
//! caractere `U+0000..U+00FF` (Latin-1), o que é sem perdas e idêntico ao texto em ASCII; `\t`, `\n`,
//! `\r`, NUL e `\` são escapados como `\t`, `\n`, `\r`, `\0` e `\\`. Só os zeros do fim do campo são
//! preenchimento: bytes velhos depois de um NUL (o editor original reaproveitava campos) aparecem como
//! `\0`, para a ida e volta ser exata.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError(String);

impl SchemaError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "schema error: {}", self.0)
    }
}

impl std::error::Error for SchemaError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    U8,
    U16,
    U32,
    U64,
    I16,
    I32,
    F32,
    /// Texto de tamanho fixo terminado em NUL (bytes crus).
    Text(usize),
    /// Bytes ainda não decifrados, em hexadecimal.
    Hex(usize),
}

impl Kind {
    #[must_use]
    pub fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 => 8,
            Self::Text(size) | Self::Hex(size) => size,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    columns: Vec<Column>,
    record_size: usize,
}

/// Monta um esquema coluna a coluna.
#[derive(Debug, Default, Clone)]
pub struct Builder {
    columns: Vec<Column>,
}

impl Builder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn column(mut self, name: &str, kind: Kind) -> Self {
        self.columns.push(Column {
            name: name.to_owned(),
            kind,
        });
        self
    }

    #[must_use]
    pub fn u8(self, name: &str) -> Self {
        self.column(name, Kind::U8)
    }

    #[must_use]
    pub fn u16(self, name: &str) -> Self {
        self.column(name, Kind::U16)
    }

    #[must_use]
    pub fn u32(self, name: &str) -> Self {
        self.column(name, Kind::U32)
    }

    #[must_use]
    pub fn u64(self, name: &str) -> Self {
        self.column(name, Kind::U64)
    }

    #[must_use]
    pub fn i16(self, name: &str) -> Self {
        self.column(name, Kind::I16)
    }

    #[must_use]
    pub fn i32(self, name: &str) -> Self {
        self.column(name, Kind::I32)
    }

    #[must_use]
    pub fn f32(self, name: &str) -> Self {
        self.column(name, Kind::F32)
    }

    #[must_use]
    pub fn text(self, name: &str, size: usize) -> Self {
        self.column(name, Kind::Text(size))
    }

    #[must_use]
    pub fn hex(self, name: &str, size: usize) -> Self {
        self.column(name, Kind::Hex(size))
    }

    /// `count` colunas do mesmo tipo, chamadas `<prefixo>1`, `<prefixo>2`...
    #[must_use]
    pub fn array(mut self, prefix: &str, count: usize, kind: Kind) -> Self {
        for index in 1..=count {
            self = self.column(&format!("{prefix}{index}"), kind);
        }
        self
    }

    /// Repete um grupo de colunas `count` vezes, com nomes `<prefixo><n>_<coluna>` (n a partir de 1).
    #[must_use]
    pub fn group(mut self, prefix: &str, count: usize, fields: impl Fn(Self) -> Self) -> Self {
        for index in 1..=count {
            for column in fields(Self::new()).columns {
                self.columns.push(Column {
                    name: format!("{prefix}{index}_{}", column.name),
                    kind: column.kind,
                });
            }
        }
        self
    }

    #[must_use]
    pub fn build(self) -> Schema {
        let record_size = self.columns.iter().map(|column| column.kind.size()).sum();
        Schema {
            columns: self.columns,
            record_size,
        }
    }
}

impl Schema {
    #[must_use]
    pub fn record_size(&self) -> usize {
        self.record_size
    }

    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Número de registros de um corpo decifrado, ou erro se o tamanho não fechar.
    pub fn record_count(&self, body: &[u8]) -> Result<usize, SchemaError> {
        if self.record_size == 0 || !body.len().is_multiple_of(self.record_size) {
            return Err(SchemaError::new(format!(
                "{} bytes is not a multiple of the {}-byte record",
                body.len(),
                self.record_size
            )));
        }
        Ok(body.len() / self.record_size)
    }

    /// Corpo decifrado → TSV com cabeçalho.
    pub fn to_tsv(&self, body: &[u8]) -> Result<String, SchemaError> {
        self.record_count(body)?;
        let mut tsv = self
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join("\t");
        tsv.push('\n');
        // `chunks_exact` com tamanho de registro só conhecido em tempo de execução.
        for record in body.chunks_exact(self.record_size) {
            let mut offset = 0;
            for (index, column) in self.columns.iter().enumerate() {
                if index > 0 {
                    tsv.push('\t');
                }
                let field = &record[offset..offset + column.kind.size()];
                format_field(column.kind, field, &mut tsv);
                offset += column.kind.size();
            }
            tsv.push('\n');
        }
        Ok(tsv)
    }

    /// TSV → corpo decifrado (o inverso de `to_tsv`).
    pub fn from_tsv(&self, tsv: &str) -> Result<Vec<u8>, SchemaError> {
        let mut lines = tsv.lines();
        let header = lines.next().ok_or_else(|| SchemaError::new("empty TSV"))?;
        let names: Vec<&str> = header.split('\t').collect();
        let expected: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        if names != expected {
            return Err(SchemaError::new(
                "header does not match the table columns (order and names must be unchanged)",
            ));
        }
        let mut body = Vec::new();
        for (row, line) in lines.enumerate() {
            let cells: Vec<&str> = line.split('\t').collect();
            if cells.len() != self.columns.len() {
                return Err(SchemaError::new(format!(
                    "row {} has {} cells, expected {}",
                    row + 1,
                    cells.len(),
                    self.columns.len()
                )));
            }
            for (column, cell) in self.columns.iter().zip(cells) {
                parse_field(column, cell, &mut body)
                    .map_err(|message| SchemaError::new(format!("row {}: {message}", row + 1)))?;
            }
        }
        Ok(body)
    }
}

fn format_field(kind: Kind, field: &[u8], out: &mut String) {
    use std::fmt::Write;
    let written: fmt::Result = match kind {
        Kind::U8 => write!(out, "{}", field[0]),
        Kind::U16 => write!(out, "{}", u16::from_le_bytes([field[0], field[1]])),
        Kind::I16 => write!(out, "{}", i16::from_le_bytes([field[0], field[1]])),
        Kind::U32 => write!(out, "{}", u32::from_le_bytes(array(field))),
        Kind::I32 => write!(out, "{}", i32::from_le_bytes(array(field))),
        Kind::F32 => write!(out, "{}", f32::from_le_bytes(array(field))),
        Kind::U64 => write!(out, "{}", u64::from_le_bytes(array(field))),
        Kind::Text(_) => {
            // Só os zeros do fim são preenchimento: bytes velhos depois de um NUL (campos que o
            // editor original reaproveitou) ficam visíveis como `\0` para a ida e volta ser exata.
            let end = field
                .iter()
                .rposition(|byte| *byte != 0)
                .map_or(0, |last| last + 1);
            escape_text(&field[..end], out);
            Ok(())
        }
        Kind::Hex(_) => {
            for byte in field {
                let _ = write!(out, "{byte:02x}");
            }
            Ok(())
        }
    };
    debug_assert!(written.is_ok(), "writing to a String cannot fail");
}

/// Bytes → texto do TSV (Latin-1; barra invertida, tab, LF, CR e NUL viram `\\`, `\t`, `\n`, `\r`, `\0`).
pub fn escape_text(bytes: &[u8], out: &mut String) {
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            0 => out.push_str("\\0"),
            other => out.push(char::from(*other)),
        }
    }
}

/// O inverso de `escape_text`; erra com um caractere acima de `U+00FF` ou um escape inválido.
pub fn unescape_text(cell: &str) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::with_capacity(cell.len());
    let mut chars = cell.chars();
    while let Some(character) = chars.next() {
        let byte = if character == '\\' {
            match chars.next() {
                Some('\\') => b'\\',
                Some('t') => b'\t',
                Some('n') => b'\n',
                Some('r') => b'\r',
                Some('0') => 0,
                _ => return Err("bad escape in"),
            }
        } else {
            u8::try_from(u32::from(character)).map_err(|_| "character above U+00FF in")?
        };
        bytes.push(byte);
    }
    Ok(bytes)
}

fn array<const N: usize>(field: &[u8]) -> [u8; N] {
    let mut raw = [0; N];
    raw.copy_from_slice(&field[..N]);
    raw
}

fn parse_field(column: &Column, cell: &str, body: &mut Vec<u8>) -> Result<(), String> {
    let bad = |what: &str| format!("column `{}`: {what} `{cell}`", column.name);
    match column.kind {
        Kind::U8 => body.push(cell.parse::<u8>().map_err(|_| bad("not a u8:"))?),
        Kind::U16 => body.extend(
            cell.parse::<u16>()
                .map_err(|_| bad("not a u16:"))?
                .to_le_bytes(),
        ),
        Kind::I16 => body.extend(
            cell.parse::<i16>()
                .map_err(|_| bad("not an i16:"))?
                .to_le_bytes(),
        ),
        Kind::U32 => body.extend(
            cell.parse::<u32>()
                .map_err(|_| bad("not a u32:"))?
                .to_le_bytes(),
        ),
        Kind::I32 => body.extend(
            cell.parse::<i32>()
                .map_err(|_| bad("not an i32:"))?
                .to_le_bytes(),
        ),
        Kind::F32 => body.extend(
            cell.parse::<f32>()
                .map_err(|_| bad("not a number:"))?
                .to_le_bytes(),
        ),
        Kind::U64 => body.extend(
            cell.parse::<u64>()
                .map_err(|_| bad("not a u64:"))?
                .to_le_bytes(),
        ),
        Kind::Text(size) => {
            let mut bytes = unescape_text(cell).map_err(&bad)?;
            // O campo pode estar cheio (sem NUL); o cliente original trata o último byte como terminador.
            if bytes.len() > size {
                return Err(bad(&format!("text longer than {size} bytes:")));
            }
            bytes.resize(size, 0);
            body.extend(bytes);
        }
        Kind::Hex(size) => {
            if cell.len() != size * 2 || !cell.is_ascii() {
                return Err(bad("expected exactly the hexadecimal bytes:"));
            }
            for pair in cell.as_bytes().chunks(2) {
                let text = std::str::from_utf8(pair).map_err(|_| bad("not hex:"))?;
                body.push(u8::from_str_radix(text, 16).map_err(|_| bad("not hex:"))?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Schema {
        Builder::new()
            .u16("id")
            .text("name", 6)
            .group("slot", 2, |slot| slot.u8("kind").i16("delta"))
            .hex("rest", 2)
            .u64("big")
            .build()
    }

    #[test]
    fn group_columns_are_numbered_and_sized() {
        let schema = sample();
        let names: Vec<_> = schema.columns().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "id",
                "name",
                "slot1_kind",
                "slot1_delta",
                "slot2_kind",
                "slot2_delta",
                "rest",
                "big"
            ]
        );
        assert_eq!(schema.record_size(), 2 + 6 + 2 * 3 + 2 + 8);
    }

    #[test]
    fn tsv_round_trips_including_escapes_and_high_bytes() {
        let schema = sample();
        let mut body = 7u16.to_le_bytes().to_vec();
        body.extend(b"a\tb\\\xe9\0");
        for (kind, delta) in [(1u8, -5i16), (2, 300)] {
            body.push(kind);
            body.extend(delta.to_le_bytes());
        }
        body.extend([0xde, 0xad]);
        body.extend(u64::MAX.to_le_bytes());
        let tsv = schema.to_tsv(&body).unwrap();
        assert_eq!(
            tsv.lines().nth(1).unwrap(),
            "7\ta\\tb\\\\\u{e9}\t1\t-5\t2\t300\tdead\t18446744073709551615"
        );
        assert_eq!(schema.from_tsv(&tsv).unwrap(), body);
    }

    #[test]
    fn from_tsv_rejects_bad_input() {
        let schema = Builder::new().u8("a").text("t", 3).build();
        assert!(schema.from_tsv("b\tt\n1\tx\n").is_err(), "header");
        assert!(schema.from_tsv("a\tt\n300\tx\n").is_err(), "overflow");
        assert!(schema.from_tsv("a\tt\n1\tabcd\n").is_err(), "text too long");
        assert!(schema.from_tsv("a\tt\n1\tabc\n").is_ok(), "full field");
        assert!(schema.from_tsv("a\tt\n1\n").is_err(), "cells");
        assert!(
            schema.from_tsv("a\tt\n1\t\u{4e2d}\n").is_err(),
            "non latin1"
        );
        assert_eq!(schema.from_tsv("a\tt\n").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn record_count_requires_exact_multiples() {
        let schema = Builder::new().u32("a").build();
        assert_eq!(schema.record_count(&[0; 8]).unwrap(), 2);
        assert!(schema.record_count(&[0; 6]).is_err());
    }
}
