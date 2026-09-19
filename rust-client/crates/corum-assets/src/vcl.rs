use std::fmt;

/// Per-vertex baked lighting (`.vcl`): one `AARRGGBB` colour for every vertex of the
/// vertex-lit (type 1) objects of the matching `.stm`, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexColors {
    pub colors: Vec<[u8; 4]>,
}

impl VertexColors {
    /// Decodes the file. Each colour is stored as a little-endian `u32` in `AARRGGBB`
    /// order, so the bytes in the file are `B, G, R, A`; the result is normalised to `[R, G, B, A]`.
    pub fn parse(bytes: &[u8]) -> Result<Self, VclError> {
        let (chunks, remainder) = bytes.as_chunks::<4>();
        if !remainder.is_empty() {
            return Err(VclError {
                message: format!(
                    "size {} is not a multiple of 4 ({} stray bytes)",
                    bytes.len(),
                    remainder.len()
                ),
            });
        }
        let colors = chunks
            .iter()
            .map(|[blue, green, red, alpha]| [*red, *green, *blue, *alpha])
            .collect();
        Ok(Self { colors })
    }

    /// Colours of one object, given the sum of the vertex counts of the objects before it.
    #[must_use]
    pub fn slice(&self, first_vertex: usize, vertex_count: usize) -> Option<&[[u8; 4]]> {
        self.colors
            .get(first_vertex..first_vertex.checked_add(vertex_count)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VclError {
    message: String,
}

impl fmt::Display for VclError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid VCL: {}", self.message)
    }
}

impl std::error::Error for VclError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorders_bgra_bytes_into_rgba() {
        let colors = VertexColors::parse(&[0x11, 0x22, 0x33, 0xff, 0x01, 0x02, 0x03, 0x80])
            .expect("two colours should parse");
        assert_eq!(
            colors.colors,
            vec![[0x33, 0x22, 0x11, 0xff], [3, 2, 1, 0x80]]
        );
    }

    #[test]
    fn rejects_sizes_that_are_not_a_multiple_of_four() {
        assert!(VertexColors::parse(&[0; 7]).is_err());
    }

    #[test]
    fn slices_by_object_range() {
        let colors = VertexColors::parse(&[0; 16]).expect("four colours should parse");
        assert_eq!(colors.slice(1, 2).map(<[_]>::len), Some(2));
        assert!(colors.slice(3, 2).is_none());
        assert!(colors.slice(usize::MAX, 2).is_none());
    }
}
