use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChrManifest {
    pub model_file: String,
    pub motions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChrError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ChrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid CHR at line {}: {}",
            self.line, self.message
        )
    }
}

impl std::error::Error for ChrError {}

impl ChrManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ChrError> {
        let text = String::from_utf8_lossy(bytes);
        let lines: Vec<(usize, &str)> = text
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty())
            .collect();

        let (model_line, model) = directive(&lines, 0, "*MOD_FILE_NAME")?;
        if model.is_empty() {
            return Err(error(model_line, "model filename is empty"));
        }

        let (motion_line, motion_count) = directive(&lines, 1, "*MOTION_NUM")?;
        let declared_count = motion_count.parse::<usize>().map_err(|_| {
            error(
                motion_line,
                format!("invalid motion count '{motion_count}'"),
            )
        })?;
        let motions: Vec<String> = lines
            .iter()
            .skip(2)
            .map(|(_, line)| (*line).to_owned())
            .collect();

        if motions.len() != declared_count {
            return Err(error(
                motion_line,
                format!(
                    "declares {declared_count} motions but contains {} filenames",
                    motions.len()
                ),
            ));
        }

        Ok(Self {
            model_file: model.to_owned(),
            motions,
        })
    }
}

fn directive<'a>(
    lines: &[(usize, &'a str)],
    index: usize,
    expected: &str,
) -> Result<(usize, &'a str), ChrError> {
    let Some(&(line_number, line)) = lines.get(index) else {
        return Err(error(index + 1, format!("missing {expected} directive")));
    };
    let mut fields = line.split_whitespace();
    let directive = fields.next().unwrap_or_default();
    if !directive.eq_ignore_ascii_case(expected) {
        return Err(error(
            line_number,
            format!("expected {expected}, found '{directive}'"),
        ));
    }
    let value = fields
        .next()
        .ok_or_else(|| error(line_number, format!("{expected} has no value")))?;
    if fields.next().is_some() {
        return Err(error(
            line_number,
            format!("{expected} has unexpected extra fields"),
        ));
    }
    Ok((line_number, value))
}

fn error(line: usize, message: impl Into<String>) -> ChrError {
    ChrError {
        line,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_manifest() {
        let manifest = ChrManifest::parse(
            b"*MOD_FILE_NAME\tdfymiss.mod\r\n*MOTION_NUM\t\t1\r\ndfmiss.anm\r\n",
        )
        .unwrap();
        assert_eq!(manifest.model_file, "dfymiss.mod");
        assert_eq!(manifest.motions, ["dfmiss.anm"]);
    }

    #[test]
    fn rejects_a_wrong_motion_count() {
        let result = ChrManifest::parse(b"*MOD_FILE_NAME a.mod\n*MOTION_NUM 2\na.anm\n");
        assert!(result.is_err());
    }
}
