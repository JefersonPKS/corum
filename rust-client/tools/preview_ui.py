#!/usr/bin/env python3
"""Pré-visualiza as janelas da interface original a partir das tabelas do "System".

Junta `InterfaceFrameInfo` (janelas), `InterfaceComponentInfo` (o que há em cada janela),
`InterfaceSpriteManager` (sprite -> recurso) e `InterfaceResourceInfo` (arquivo e recorte) e desenha
cada janela com Pillow. Serve para confirmar como as tabelas devem ser lidas antes do renderizador
Rust do sandbox.

Uso (depois de `python tools/export_client_data.py <cliente> export`):

    python tools/preview_ui.py export target/verification/ui 5 2 3

Cada argumento depois da pasta de saída é o id da janela (`InterfaceFrameInfo.frame_id`).
"""
import csv
import struct
import sys
from pathlib import Path

from PIL import Image


def rows(system: Path, name: str) -> list[dict]:
    lines = [
        line
        for line in (system / f"{name}.tsv").read_text(encoding="utf-8").split("\n")
        if line and not line.startswith("#")
    ]
    return list(csv.DictReader(lines, delimiter="\t", quoting=csv.QUOTE_NONE))


def frame(record: dict) -> dict:
    return {
        "name": record["name"],
        "width": int(record["width"]),
        "height": int(record["height"]),
        "left": int(record["left"]),
        "top": int(record["top"]),
    }


def load_image(ui: Path, name: str, cache: dict) -> Image.Image | None:
    key = name.lower()
    if key not in cache:
        found = next((p for p in ui.iterdir() if p.name.lower() == key), None)
        if found is None:
            stem = Path(name).stem.lower()
            found = next((p for p in ui.iterdir() if p.stem.lower() == stem), None)
        cache[key] = Image.open(found).convert("RGBA") if found else None
    return cache[key]


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    export, out = Path(sys.argv[1]), Path(sys.argv[2])
    windows = sys.argv[3:]
    system, ui = export / "system", export / "paks" / "UI"
    out.mkdir(parents=True, exist_ok=True)
    frames = {int(r["frame_id"]): frame(r) for r in rows(system, "InterfaceFrameInfo")}
    resources = {r["id"]: r for r in rows(system, "InterfaceResourceInfo")}
    sprites = {r["id"]: r["resource_id"] for r in rows(system, "InterfaceSpriteManager")}
    components = rows(system, "InterfaceComponentInfo")
    cache: dict = {}
    for window in windows:
        info = frames[int(window)]
        width, height = max(info["width"], 1), max(info["height"], 1)
        canvas = Image.new("RGBA", (width, height), (40, 0, 40, 255))
        drawn = set()
        sprite_parts = [
            c for c in components if c["interface_id"] == window and c["resource_type"] == "3"
        ]
        sprite_parts.sort(key=lambda c: (int(c["order"]), int(c["callback_id"])))
        for part in sprite_parts:
            position = (int(part["left"]), int(part["top"]))
            # Vários componentes no mesmo lugar são estados do mesmo botão: só o primeiro entra.
            if (int(part["order"]), position) in drawn:
                continue
            drawn.add((int(part["order"]), position))
            resource = resources.get(sprites.get(part["resource_id"], ""))
            if resource is None:
                continue
            image = load_image(ui, resource["file_name"], cache)
            if image is None:
                print(f"  janela {window}: sem imagem {resource['file_name']}")
                continue
            if resource["type"] == "0":
                piece = image
            else:
                x, y, w, h = (int(resource[k]) for k in ("x", "y", "width", "height"))
                piece = image.crop((x, y, x + w, y + h))
            canvas.alpha_composite(piece, dest=position) if (
                position[0] + piece.width <= width and position[1] + piece.height <= height
            ) else canvas.paste(piece, position)
        target = out / f"window_{int(window):02d}_{info['name']}.png"
        canvas.save(target)
        print(f"{target} ({width}x{height}, {len(sprite_parts)} sprites)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
