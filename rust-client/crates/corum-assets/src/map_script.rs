use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct MapScript {
    pub bounds_min: Option<[f32; 3]>,
    pub bounds_max: Option<[f32; 3]>,
    pub static_model: Option<String>,
    pub height_field: Option<String>,
    pub objects: Vec<MapObject>,
    pub lights: Vec<MapLight>,
}

/// One `GX_LIGHT` entry: a coloured point light with a radius of influence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapLight {
    /// Colour as written in the script, `AARRGGBB`.
    pub argb: u32,
    pub position: [f32; 3],
    pub radius: f32,
    /// Trailing integer, `1000` in every sample; meaning unknown.
    pub parameter: u32,
}

impl MapLight {
    /// Colour channels as `[r, g, b]` in `0.0..=1.0`.
    #[must_use]
    pub fn rgb(&self) -> [f32; 3] {
        [
            ((self.argb >> 16) & 0xff) as f32 / 255.0,
            ((self.argb >> 8) & 0xff) as f32 / 255.0,
            (self.argb & 0xff) as f32 / 255.0,
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapObject {
    pub resource: String,
    pub id: u32,
    pub scale: [f32; 3],
    pub position: [f32; 3],
    pub axis: [f32; 3],
    pub angle_radians: f32,
    pub flags: String,
}

impl MapScript {
    pub fn parse(bytes: &[u8]) -> Result<Self, MapScriptError> {
        let text = std::str::from_utf8(bytes).map_err(|error| {
            MapScriptError::new(error.valid_up_to(), "script is not UTF-8/ASCII")
        })?;
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let bounds_min = vector_after(&tokens, "BOX_MIN")?;
        let bounds_max = vector_after(&tokens, "BOX_MAX")?;
        let static_model = value_after(&tokens, "STATIC_MODEL").map(str::to_owned);
        let height_field = value_after(&tokens, "HEIGHT_FIELD")
            .filter(|value| !value.eq_ignore_ascii_case("NA"))
            .map(str::to_owned);
        let objects = parse_objects(&tokens)?;
        let lights = parse_lights(&tokens)?;

        Ok(Self {
            bounds_min,
            bounds_max,
            static_model,
            height_field,
            objects,
            lights,
        })
    }
}

fn parse_objects(tokens: &[&str]) -> Result<Vec<MapObject>, MapScriptError> {
    let Some(section) = tokens.iter().position(|token| *token == "GX_OBJECT") else {
        return Ok(Vec::new());
    };
    let count = parse_u32(tokens, section + 1, "GX_OBJECT count")? as usize;
    let mut cursor = section + 2;
    if tokens.get(cursor) == Some(&"{") {
        cursor += 1;
    }

    let mut objects = Vec::with_capacity(count);
    for object_index in 0..count {
        let resource = token(tokens, cursor, "object resource")?.to_owned();
        cursor += 1;
        let id = parse_u32(tokens, cursor, "object id")?;
        cursor += 1;
        let scale = parse_vector(tokens, &mut cursor, "object scale")?;
        let position = parse_vector(tokens, &mut cursor, "object position")?;
        let axis = parse_vector(tokens, &mut cursor, "object rotation axis")?;
        let angle_radians = parse_f32(tokens, cursor, "object rotation angle")?;
        cursor += 1;
        let flags = token(tokens, cursor, "object flags")?.to_owned();
        cursor += 1;
        objects.push(MapObject {
            resource,
            id,
            scale,
            position,
            axis,
            angle_radians,
            flags,
        });

        if tokens.get(cursor) == Some(&"}") && object_index + 1 != count {
            return Err(MapScriptError::new(
                cursor,
                "GX_OBJECT ended before its declared count",
            ));
        }
    }
    Ok(objects)
}

/// Reads `GX_LIGHT`. The declared count includes a zeroed terminator record
/// (`0 0 0 0 0 1000`), which is not a light and is dropped.
fn parse_lights(tokens: &[&str]) -> Result<Vec<MapLight>, MapScriptError> {
    let Some(section) = tokens.iter().position(|token| *token == "GX_LIGHT") else {
        return Ok(Vec::new());
    };
    let count = parse_u32(tokens, section + 1, "GX_LIGHT count")? as usize;
    let mut cursor = section + 2;
    if tokens.get(cursor) == Some(&"{") {
        cursor += 1;
    }

    let mut lights = Vec::new();
    for _ in 0..count {
        let color = token(tokens, cursor, "light colour")?;
        let argb = u32::from_str_radix(color, 16)
            .map_err(|_| MapScriptError::new(cursor, "invalid light colour"))?;
        cursor += 1;
        let position = parse_vector(tokens, &mut cursor, "light position")?;
        let radius = parse_f32(tokens, cursor, "light radius")?;
        cursor += 1;
        let parameter = parse_u32(tokens, cursor, "light parameter")?;
        cursor += 1;
        if argb != 0 || radius != 0.0 {
            lights.push(MapLight {
                argb,
                position,
                radius,
                parameter,
            });
        }
    }
    Ok(lights)
}

fn vector_after(tokens: &[&str], name: &str) -> Result<Option<[f32; 3]>, MapScriptError> {
    let Some(index) = tokens.iter().position(|token| *token == name) else {
        return Ok(None);
    };
    let mut cursor = index + 1;
    parse_vector(tokens, &mut cursor, name).map(Some)
}

fn value_after<'a>(tokens: &'a [&str], name: &str) -> Option<&'a str> {
    let index = tokens.iter().position(|token| *token == name)?;
    tokens.get(index + 1).copied()
}

fn parse_vector(
    tokens: &[&str],
    cursor: &mut usize,
    description: &str,
) -> Result<[f32; 3], MapScriptError> {
    let result = [
        parse_f32(tokens, *cursor, description)?,
        parse_f32(tokens, *cursor + 1, description)?,
        parse_f32(tokens, *cursor + 2, description)?,
    ];
    *cursor += 3;
    Ok(result)
}

fn parse_f32(tokens: &[&str], index: usize, description: &str) -> Result<f32, MapScriptError> {
    token(tokens, index, description)?
        .parse()
        .map_err(|_| MapScriptError::new(index, format!("invalid {description}")))
}

fn parse_u32(tokens: &[&str], index: usize, description: &str) -> Result<u32, MapScriptError> {
    token(tokens, index, description)?
        .parse()
        .map_err(|_| MapScriptError::new(index, format!("invalid {description}")))
}

fn token<'a>(
    tokens: &'a [&str],
    index: usize,
    description: &str,
) -> Result<&'a str, MapScriptError> {
    tokens
        .get(index)
        .copied()
        .ok_or_else(|| MapScriptError::new(index, format!("missing {description}")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapScriptError {
    pub token_index: usize,
    pub message: String,
}

impl MapScriptError {
    fn new(token_index: usize, message: impl Into<String>) -> Self {
        Self {
            token_index,
            message: message.into(),
        }
    }
}

impl fmt::Display for MapScriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid MAP script near token {}: {}",
            self.token_index, self.message
        )
    }
}

impl std::error::Error for MapScriptError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_map_metadata_and_objects() {
        let script = br#"
            GX_METADATA { BOX_MAX 10 20 30 BOX_MIN -1 -2 -3 }
            GX_MAP { STATIC_MODEL 1.stm HEIGHT_FIELD NA }
            GX_OBJECT 1 {
                lamp.mod 42 1 2 3 100 200 300 0 1 0 1.57 100000A
            }
        "#;
        let map = MapScript::parse(script).expect("synthetic MAP should parse");
        assert_eq!(map.static_model.as_deref(), Some("1.stm"));
        assert_eq!(map.height_field, None);
        assert_eq!(map.bounds_min, Some([-1.0, -2.0, -3.0]));
        assert_eq!(map.objects.len(), 1);
        assert_eq!(map.objects[0].position, [100.0, 200.0, 300.0]);
        assert_eq!(map.objects[0].flags, "100000A");
    }

    #[test]
    fn parses_lights_and_drops_the_zero_terminator() {
        let script = br#"
            GX_LIGHT 3
            {
                FF323296 1772.8 200.0 2936.56 2000.0 1000
                FFDC50FF 2449.82 120.0 3073.49 300.0 1000
                0 0.0 0.0 0.0 0.0 1000
            }
            GX_TRIGGER 0 { }
        "#;
        let map = MapScript::parse(script).expect("synthetic lights should parse");
        assert_eq!(map.lights.len(), 2);
        assert_eq!(map.lights[0].position, [1772.8, 200.0, 2936.56]);
        assert_eq!(map.lights[0].radius, 2000.0);
        let [red, green, blue] = map.lights[0].rgb();
        assert!((red - 50.0 / 255.0).abs() < 1e-6);
        assert!((green - 50.0 / 255.0).abs() < 1e-6);
        assert!((blue - 150.0 / 255.0).abs() < 1e-6);
    }
}
