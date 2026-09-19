#!/usr/bin/env python3
"""Levanta os objetos posicionados (`GX_OBJECT`) dos mapas e confere se os recursos existem.

Lê os `.map` já extraídos por `tools/survey_maps.py` e compara os recursos (`.MOD` e `.CHR`) com o
conteúdo de `Map_chr.pak`.

Uso:

    python tools/survey_props.py "D:\\Games\\CorumOnline\\Data" target/verification/maps

Estado registrado em 2026-09-19: 10.282 objetos em 158 mapas (7.389 `.MOD`, 2.893 `.CHR`),
36 sem arquivo (`wobj0001..4.mod`), eixo de rotação sempre Y, 118 escalas não uniformes.
"""
import collections
import struct
import sys
from pathlib import Path


def pak_names(pak: Path) -> set[str]:
    blob = pak.read_bytes()
    count = struct.unpack_from("<I", blob, 4)[0]
    offset, names = 92, set()
    for _ in range(count):
        total, _size, name_size, _stored = struct.unpack_from("<4I", blob, offset)
        names.add(blob[offset + 32 : offset + 32 + name_size].decode("latin1").lower())
        offset += total
    return names


def objects_of(script: Path) -> list[tuple]:
    tokens = script.read_text(encoding="latin1").split()
    if "GX_OBJECT" not in tokens:
        return []
    index = tokens.index("GX_OBJECT")
    count = int(tokens[index + 1])
    cursor = index + (3 if tokens[index + 2] == "{" else 2)
    found = []
    for _ in range(count):
        resource, object_id = tokens[cursor], int(tokens[cursor + 1])
        scale = tuple(float(v) for v in tokens[cursor + 2 : cursor + 5])
        position = tuple(float(v) for v in tokens[cursor + 5 : cursor + 8])
        axis = tuple(float(v) for v in tokens[cursor + 8 : cursor + 11])
        angle, flags = float(tokens[cursor + 11]), tokens[cursor + 12]
        found.append((resource, object_id, scale, position, axis, angle, flags))
        cursor += 13
    return found


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    data_dir, work = Path(sys.argv[1]), Path(sys.argv[2])
    names = pak_names(data_dir / "Map_chr" / "Map_chr.pak")
    total: collections.Counter[str] = collections.Counter()
    missing: collections.Counter[str] = collections.Counter()
    flags: collections.Counter[str] = collections.Counter()
    axes: collections.Counter[tuple] = collections.Counter()
    per_map: dict[str, int] = {}
    for script in sorted(work.glob("*.map")):
        objects = objects_of(script)
        if not objects:
            continue
        per_map[script.stem] = len(objects)
        for resource, _id, scale, _pos, axis, _angle, flag in objects:
            key = resource.lower()
            total["objetos"] += 1
            total[key.rsplit(".", 1)[-1]] += 1
            if key not in names:
                missing[key] += 1
            flags[flag] += 1
            axes[tuple(round(v, 2) for v in axis)] += 1
            if max(scale) - min(scale) > 1e-3:
                total["escala_nao_uniforme"] += 1
    print(f"mapas com objetos: {len(per_map)}  {dict(total)}")
    print(f"recursos sem arquivo: {sum(missing.values())} objetos, {len(missing)} nomes: {missing.most_common(8)}")
    print(f"flags: {flags.most_common(10)}")
    print(f"eixos: {axes.most_common(4)}")
    print("maiores:", sorted(per_map.items(), key=lambda kv: -kv[1])[:6])
    return 0


if __name__ == "__main__":
    sys.exit(main())
