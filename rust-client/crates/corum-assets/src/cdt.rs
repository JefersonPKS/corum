//! Tabelas `.cdt` de `Data\Cdt`: quadros de efeito e sons de cada movimento de um personagem.
//!
//! **[confirmado]** Lidas por `InitChrInfo` (`CorumOnlineProject/GameControl.cpp`) com a struct `ChrInfo`
//! (`ChrInfo.h`): `u32 animações`, `u32 movimentos`, depois `animações × movimentos` registros de 92 bytes:
//! `u8 quadro_de_efeito[10]`, 2 bytes de padding do compilador, `10 × (u32 quadro, u32 som)`.
//! O cliente indexa `[animação × movimentos + movimento − 1]`; para jogadores a "animação" é o tipo
//! de item na mão (9) e o movimento vai de 1 a 50; monstros e efeitos têm 1 × 15 ou 1 × 1.
//!
//! **Sem cifra.** Dos 219 arquivos, 213 seguem esse formato; `m00011.cdt` (registros de 2 bytes) e
//! `pm01001..pm05001.cdt` (idênticos, 9 × 50 registros de 2 bytes) têm outro layout e nenhum leitor no
//! código do cliente: ficam de fora (`Cdt::parse` os recusa).

use crate::schema::{Builder, Schema};
use std::fmt;

const RECORD_SIZE: usize = 92;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdtError(String);

impl fmt::Display for CdtError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid CDT: {}", self.0)
    }
}

impl std::error::Error for CdtError {}

/// Um arquivo `.cdt` com os registros ainda em bytes (o esquema os converte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cdt {
    pub animation_count: u32,
    pub motion_count: u32,
    records: Vec<u8>,
}

/// Colunas de um registro `ChrInfo`.
#[must_use]
pub fn record_schema() -> Schema {
    Builder::new()
        .array("effect_frame", 10, crate::schema::Kind::U8)
        .hex("padding", 2)
        .group("sound", 10, |sound| sound.u32("frame").u32("id"))
        .build()
}

impl Cdt {
    pub fn parse(bytes: &[u8]) -> Result<Self, CdtError> {
        let word = |offset: usize| {
            bytes
                .get(offset..offset + 4)
                .map(|raw| u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
                .ok_or_else(|| CdtError("smaller than the 8-byte header".to_owned()))
        };
        let (animation_count, motion_count) = (word(0)?, word(4)?);
        let expected = (animation_count as usize)
            .checked_mul(motion_count as usize)
            .and_then(|count| count.checked_mul(RECORD_SIZE))
            .and_then(|size| size.checked_add(8));
        if expected != Some(bytes.len()) {
            return Err(CdtError(format!(
                "{} bytes do not fit {animation_count} × {motion_count} records of {RECORD_SIZE} bytes",
                bytes.len()
            )));
        }
        Ok(Self {
            animation_count,
            motion_count,
            records: bytes[8..].to_vec(),
        })
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.animation_count.to_le_bytes().to_vec();
        bytes.extend(self.motion_count.to_le_bytes());
        bytes.extend(&self.records);
        bytes
    }
}

/// Arquivo em montagem: nome, animações, movimentos e os registros `(animação, movimento, bytes)`.
type PendingFile = (String, u32, u32, Vec<(u32, u32, Vec<u8>)>);

const PREFIX: [&str; 3] = ["file", "animation", "motion"];

/// Cabeçalho do TSV combinado: `file`, `animation` (0..), `motion` (1..) e as colunas do registro.
fn header(schema: &Schema) -> String {
    let mut columns: Vec<&str> = PREFIX.to_vec();
    columns.extend(schema.columns().iter().map(|column| column.name.as_str()));
    columns.join("\t")
}

/// Vários `.cdt` (nome sem extensão, arquivo) → um TSV, uma linha por (arquivo, animação, movimento).
pub fn to_tsv(files: &[(String, Cdt)]) -> Result<String, CdtError> {
    let schema = record_schema();
    let mut tsv = header(&schema) + "\n";
    for (name, cdt) in files {
        if name.contains(['\t', '\n', '\r']) {
            return Err(CdtError(format!(
                "file name `{name}` has control characters"
            )));
        }
        // `as_chunks` não serve aqui: o tamanho é uma constante do módulo usada também em tempo de execução.
        #[allow(clippy::chunks_exact_to_as_chunks)]
        let per_record = cdt.records.chunks_exact(RECORD_SIZE);
        for (index, record) in per_record.enumerate() {
            let animation = index / cdt.motion_count as usize;
            let motion = index % cdt.motion_count as usize + 1;
            let line = schema
                .to_tsv(record)
                .map_err(|error| CdtError(error.to_string()))?;
            let row = line.lines().nth(1).unwrap_or_default();
            tsv.push_str(&format!("{name}\t{animation}\t{motion}\t{row}\n"));
        }
    }
    Ok(tsv)
}

/// O inverso de `to_tsv`: agrupa as linhas por arquivo, na ordem em que aparecem.
pub fn from_tsv(tsv: &str) -> Result<Vec<(String, Cdt)>, CdtError> {
    let schema = record_schema();
    let mut lines = tsv.lines();
    if lines.next() != Some(header(&schema).as_str()) {
        return Err(CdtError(
            "header does not match (order and names must be unchanged)".to_owned(),
        ));
    }
    let record_header = header(&schema)
        .splitn(4, '\t')
        .nth(3)
        .unwrap_or_default()
        .to_owned();
    // (nome, maior animação + 1, maior movimento, registros na ordem lida)
    let mut files: Vec<PendingFile> = Vec::new();
    for (row, line) in lines.enumerate() {
        let mut cells = line.splitn(4, '\t');
        let (Some(name), Some(animation), Some(motion), Some(rest)) =
            (cells.next(), cells.next(), cells.next(), cells.next())
        else {
            return Err(CdtError(format!("row {} is missing columns", row + 1)));
        };
        let number = |cell: &str, what: &str| {
            cell.parse::<u32>()
                .map_err(|_| CdtError(format!("row {}: {what} `{cell}` is not a number", row + 1)))
        };
        let (animation, motion) = (number(animation, "animation")?, number(motion, "motion")?);
        if motion == 0 {
            return Err(CdtError(format!("row {}: motions start at 1", row + 1)));
        }
        let record = schema
            .from_tsv(&format!("{record_header}\n{rest}\n"))
            .map_err(|error| CdtError(format!("row {}: {error}", row + 1)))?;
        let slot = match files.iter().position(|(existing, ..)| existing == name) {
            Some(slot) => slot,
            None => {
                files.push((name.to_owned(), 0, 0, Vec::new()));
                files.len() - 1
            }
        };
        let entry = &mut files[slot];
        entry.1 = entry.1.max(animation + 1);
        entry.2 = entry.2.max(motion);
        entry.3.push((animation, motion, record));
    }
    files
        .into_iter()
        .map(|(name, animations, motions, mut records)| {
            if records.len() != animations as usize * motions as usize {
                return Err(CdtError(format!(
                    "{name}: {} rows do not fill {animations} animations × {motions} motions",
                    records.len()
                )));
            }
            records.sort_by_key(|(animation, motion, _)| (*animation, *motion));
            if records
                .windows(2)
                .any(|pair| (pair[0].0, pair[0].1) == (pair[1].0, pair[1].1))
            {
                return Err(CdtError(format!(
                    "{name}: repeated (animation, motion) row"
                )));
            }
            Ok((
                name,
                Cdt {
                    animation_count: animations,
                    motion_count: motions,
                    records: records
                        .into_iter()
                        .flat_map(|(_, _, bytes)| bytes)
                        .collect(),
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(animations: u32, motions: u32, first_frame: u8) -> Vec<u8> {
        let mut bytes = animations.to_le_bytes().to_vec();
        bytes.extend(motions.to_le_bytes());
        for index in 0..animations * motions {
            let mut record = vec![0; RECORD_SIZE];
            record[0] = first_frame + index as u8;
            record[12..16].copy_from_slice(&(index + 3).to_le_bytes());
            record[16..20].copy_from_slice(&7u32.to_le_bytes());
            bytes.extend(record);
        }
        bytes
    }

    /// Com `CORUM_DATA`: os 213 `.cdt` no formato `ChrInfo` voltam idênticos depois de TSV → arquivos.
    #[test]
    fn real_cdt_files_round_trip_through_tsv() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let directory = std::path::Path::new(&data).join("Cdt");
        let mut originals = Vec::new();
        let mut paths: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            let bytes = std::fs::read(&path).unwrap();
            if let Ok(cdt) = Cdt::parse(&bytes) {
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                originals.push((name, bytes, cdt));
            }
        }
        assert_eq!(originals.len(), 213);
        let parsed: Vec<_> = originals
            .iter()
            .map(|(name, _, cdt)| (name.clone(), cdt.clone()))
            .collect();
        let again = from_tsv(&to_tsv(&parsed).unwrap()).unwrap();
        for ((name, bytes, _), (again_name, cdt)) in originals.iter().zip(&again) {
            assert_eq!(name, again_name);
            assert_eq!(&cdt.to_bytes(), bytes, "{name}");
        }
    }

    #[test]
    fn rejects_files_that_do_not_fit_the_header() {
        assert!(Cdt::parse(&file(2, 3, 0)).is_ok());
        assert!(Cdt::parse(&file(2, 3, 0)[..100]).is_err());
        assert!(Cdt::parse(&[1, 0, 0]).is_err());
    }

    #[test]
    fn tsv_round_trips_several_files() {
        let files = vec![
            ("a".to_owned(), Cdt::parse(&file(2, 2, 1)).unwrap()),
            ("b".to_owned(), Cdt::parse(&file(1, 3, 9)).unwrap()),
        ];
        let tsv = to_tsv(&files).unwrap();
        assert_eq!(tsv.lines().count(), 1 + 4 + 3);
        assert!(tsv.lines().nth(2).unwrap().starts_with("a\t0\t2\t2\t"));
        assert_eq!(from_tsv(&tsv).unwrap(), files);
    }

    #[test]
    fn from_tsv_detects_missing_rows() {
        let files = vec![("a".to_owned(), Cdt::parse(&file(2, 2, 1)).unwrap())];
        let tsv = to_tsv(&files).unwrap();
        let cut: String = tsv
            .lines()
            .take(4)
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(from_tsv(&cut).is_err());
    }
}
