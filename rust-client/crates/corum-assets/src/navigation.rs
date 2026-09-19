//! Busca de caminho sobre a grade de tiles (`.ttb`): o "clique para andar".
//!
//! O cliente original chama `g_pSw->FindShortestWay` (um módulo de busca em DLL) e anda entre os
//! pontos de curva devolvidos (`A_STAR` em `GameControl.h`, `DungeonProcess.cpp`). Aqui é
//! reimplementado: A* em 8 direções sobre os tiles caminháveis, sem cortar quinas, seguido de um
//! alisamento por linha de visão para sobrar só as curvas. **[hipótese]** o resultado é equivalente em
//! espírito, mas não idêntico ao do módulo original (não há como comparar sem rodar o cliente).
//!
//! Coordenadas em **unidades de tile**: o tile `(i, j)` cobre `[i, i+1) × [j, j+1)`.

use crate::ttb::TileMap;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Distância máxima, em tiles, para "grudar" um clique num tile bloqueado ao tile andável mais perto.
const SNAP_RADIUS: i32 = 3;
/// Passo da amostragem de linha de visão, em tiles.
const SIGHT_STEP: f32 = 0.1;

fn tile_of(map: &TileMap, point: [f32; 2]) -> Option<(u32, u32)> {
    let (x, z) = (point[0].floor(), point[1].floor());
    (x >= 0.0 && z >= 0.0 && x < map.width as f32 && z < map.height as f32)
        .then_some((x as u32, z as u32))
}

fn walkable(map: &TileMap, x: i32, z: i32) -> bool {
    x >= 0 && z >= 0 && map.is_walkable(x as u32, z as u32)
}

/// Um corpo de raio `radius` centrado em `point` cabe (os quatro cantos estão em tiles andáveis).
#[must_use]
pub fn can_stand(map: &TileMap, point: [f32; 2], radius: f32) -> bool {
    [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .all(|(dx, dz)| {
            walkable(
                map,
                (point[0] + dx * radius).floor() as i32,
                (point[1] + dz * radius).floor() as i32,
            )
        })
}

/// O segmento `a → b` pode ser percorrido por um corpo de raio `radius` sem bater em tile bloqueado.
#[must_use]
pub fn line_is_clear(map: &TileMap, a: [f32; 2], b: [f32; 2], radius: f32) -> bool {
    let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let steps = (length / SIGHT_STEP).ceil().max(1.0) as u32;
    (0..=steps).all(|step| {
        let t = step as f32 / steps as f32;
        can_stand(
            map,
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            radius,
        )
    })
}

/// Tile andável mais próximo de `point` (o próprio tile, se andável), até `SNAP_RADIUS` tiles.
fn nearest_walkable(map: &TileMap, point: [f32; 2]) -> Option<(i32, i32)> {
    let (tx, tz) = (point[0].floor() as i32, point[1].floor() as i32);
    let mut best: Option<(f32, (i32, i32))> = None;
    for dz in -SNAP_RADIUS..=SNAP_RADIUS {
        for dx in -SNAP_RADIUS..=SNAP_RADIUS {
            let (x, z) = (tx + dx, tz + dz);
            if !walkable(map, x, z) {
                continue;
            }
            let distance =
                (x as f32 + 0.5 - point[0]).powi(2) + (z as f32 + 0.5 - point[1]).powi(2);
            if best.is_none_or(|(known, _)| distance < known) {
                best = Some((distance, (x, z)));
            }
        }
    }
    best.map(|(_, tile)| tile)
}

/// Caminho de `from` até `to` (ambos em tiles), como uma lista de pontos a seguir em linha reta, que
/// termina em `to` (ou no tile andável mais próximo, se `to` cair em um bloqueado). `None` se o
/// destino é inalcançável ou está fora do mapa. `radius` é o raio do corpo, em tiles.
#[must_use]
pub fn find_path(
    map: &TileMap,
    from: [f32; 2],
    to: [f32; 2],
    radius: f32,
) -> Option<Vec<[f32; 2]>> {
    let start = tile_of(map, from)?;
    tile_of(map, to)?;
    let goal = nearest_walkable(map, to)?;
    let goal_point = if walkable(map, to[0].floor() as i32, to[1].floor() as i32)
        && can_stand(map, to, radius)
    {
        to
    } else {
        [goal.0 as f32 + 0.5, goal.1 as f32 + 0.5]
    };
    if line_is_clear(map, from, goal_point, radius) {
        return Some(vec![goal_point]);
    }
    let tiles = astar(map, (start.0 as i32, start.1 as i32), goal)?;
    // Alisamento: de cada âncora, vai ao ponto mais distante ainda visível.
    let mut points: Vec<[f32; 2]> = tiles
        .iter()
        .skip(1)
        .map(|(x, z)| [*x as f32 + 0.5, *z as f32 + 0.5])
        .collect();
    if let Some(last) = points.last_mut() {
        *last = goal_point;
    }
    let mut route = Vec::new();
    let mut anchor = from;
    let mut index = 0;
    while index < points.len() {
        let mut farthest = index;
        for candidate in (index..points.len()).rev() {
            if line_is_clear(map, anchor, points[candidate], radius) {
                farthest = candidate;
                break;
            }
        }
        anchor = points[farthest];
        route.push(anchor);
        index = farthest + 1;
    }
    Some(route)
}

/// A* de 8 direções entre dois tiles; devolve os tiles do início ao fim.
fn astar(map: &TileMap, start: (i32, i32), goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    let width = map.width as i32;
    let index = |(x, z): (i32, i32)| (z * width + x) as usize;
    let count = map.width as usize * map.height as usize;
    let mut cost = vec![u32::MAX; count];
    let mut parent = vec![usize::MAX; count];
    // Custos em milésimos de tile: reto 1000, diagonal 1414 (heurística octile).
    let heuristic = |(x, z): (i32, i32)| {
        let (dx, dz) = ((x - goal.0).unsigned_abs(), (z - goal.1).unsigned_abs());
        1000 * dx.max(dz) + 414 * dx.min(dz)
    };
    let mut open = BinaryHeap::new();
    cost[index(start)] = 0;
    open.push(Reverse((heuristic(start), start)));
    while let Some(Reverse((_, current))) = open.pop() {
        if current == goal {
            let mut tiles = vec![current];
            let mut at = index(current);
            while parent[at] != usize::MAX {
                at = parent[at];
                tiles.push(((at as i32) % width, (at as i32) / width));
            }
            tiles.reverse();
            return Some(tiles);
        }
        for (dx, dz) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let next = (current.0 + dx, current.1 + dz);
            if !walkable(map, next.0, next.1) {
                continue;
            }
            let diagonal = dx != 0 && dz != 0;
            // Sem cortar quinas: as duas vizinhas ortogonais também precisam ser andáveis.
            if diagonal
                && !(walkable(map, current.0 + dx, current.1)
                    && walkable(map, current.0, current.1 + dz))
            {
                continue;
            }
            let step = if diagonal { 1414 } else { 1000 };
            let through = cost[index(current)] + step;
            if through < cost[index(next)] {
                cost[index(next)] = through;
                parent[index(next)] = index(current);
                open.push(Reverse((through + heuristic(next), next)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ttb::TileAttribute;

    /// Mapa de texto: `#` bloqueado, `.` livre.
    fn map(rows: &[&str]) -> TileMap {
        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        TileMap {
            width,
            height,
            tile_size: 100,
            declared_object_count: 0,
            tiles: rows
                .iter()
                .flat_map(|row| row.bytes())
                .map(|cell| TileAttribute::from_raw(u16::from(cell == b'#')))
                .collect(),
            declared_section_count: None,
            trailing_bytes: 0,
        }
    }

    /// Com `CORUM_DATA`: no mapa real `1100`, todo caminho achado entre tiles andáveis tem os trechos
    /// livres e termina no destino; e ao menos metade dos pares sorteados tem caminho.
    #[test]
    fn real_map_paths_are_valid() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let bytes =
            std::fs::read(std::path::Path::new(&data).join("Map").join("1100.ttb")).unwrap();
        let map = TileMap::parse(&bytes).unwrap();
        let walkable_tiles: Vec<(u32, u32)> = (0..map.height)
            .flat_map(|z| (0..map.width).map(move |x| (x, z)))
            .filter(|(x, z)| map.is_walkable(*x, *z))
            .collect();
        assert!(walkable_tiles.len() > 50);
        let (mut found, mut tried) = (0, 0);
        for index in 0..200usize {
            // Pares determinísticos espalhados pela lista de tiles andáveis.
            let a = walkable_tiles[(index * 37) % walkable_tiles.len()];
            let b = walkable_tiles[(index * 91 + 13) % walkable_tiles.len()];
            let (from, to) = (
                [a.0 as f32 + 0.5, a.1 as f32 + 0.5],
                [b.0 as f32 + 0.5, b.1 as f32 + 0.5],
            );
            tried += 1;
            let Some(path) = find_path(&map, from, to, 0.2) else {
                continue;
            };
            found += 1;
            let mut at = from;
            for point in &path {
                // O tile de partida pode encostar em parede: a checagem vale para o resto do trecho.
                assert!(
                    line_is_clear(&map, at, *point, 0.2) || at == from,
                    "{a:?} -> {b:?}: {at:?} -> {point:?}"
                );
                at = *point;
            }
            assert_eq!(at[0].floor() as i32, b.0 as i32, "{a:?} -> {b:?}");
            assert_eq!(at[1].floor() as i32, b.1 as i32, "{a:?} -> {b:?}");
        }
        assert!(found * 2 >= tried, "{found} of {tried}");
    }

    #[test]
    fn open_field_is_a_single_straight_segment() {
        let field = map(&["........", "........", "........", "........"]);
        let path = find_path(&field, [0.5, 0.5], [6.5, 3.5], 0.2).unwrap();
        assert_eq!(path, vec![[6.5, 3.5]]);
    }

    #[test]
    fn a_wall_forces_a_detour_that_never_touches_blocked_tiles() {
        let field = map(&[
            "..........",
            "....#.....",
            "....#.....",
            "....#.....",
            "....#.....",
            "..........",
        ]);
        let path = find_path(&field, [1.5, 2.5], [8.5, 2.5], 0.2).unwrap();
        assert!(path.len() >= 2, "{path:?}");
        assert_eq!(*path.last().unwrap(), [8.5, 2.5]);
        let mut at = [1.5, 2.5];
        for point in &path {
            assert!(
                line_is_clear(&field, at, *point, 0.2),
                "{at:?} -> {point:?}"
            );
            at = *point;
        }
    }

    #[test]
    fn unreachable_or_outside_goals_give_no_path() {
        let field = map(&[".....", ".###.", ".#.#.", ".###.", "....."]);
        assert!(
            find_path(&field, [0.5, 0.5], [2.5, 2.5], 0.2).is_none(),
            "enclosed tile"
        );
        assert!(
            find_path(&field, [0.5, 0.5], [9.5, 0.5], 0.2).is_none(),
            "outside the map"
        );
    }

    #[test]
    fn a_click_on_a_wall_snaps_to_the_nearest_free_tile() {
        let field = map(&["......", "..##..", "..##..", "......"]);
        let path = find_path(&field, [0.5, 0.5], [2.6, 1.6], 0.2).unwrap();
        let end = *path.last().unwrap();
        assert!(walkable(&field, end[0] as i32, end[1] as i32), "{end:?}");
        assert!((end[0] - 2.6).abs() < 1.6 && (end[1] - 1.6).abs() < 1.6);
    }

    #[test]
    fn diagonals_do_not_cut_corners() {
        // A única passagem é a diagonal entre dois tiles bloqueados: não vale atravessá-la.
        let field = map(&["..#.", "..#.", "##..", "..#."]);
        let path = find_path(&field, [0.5, 0.5], [3.5, 0.5], 0.2);
        if let Some(path) = path {
            let mut at = [0.5, 0.5];
            for point in &path {
                assert!(line_is_clear(&field, at, *point, 0.2));
                at = *point;
            }
        }
    }
}
