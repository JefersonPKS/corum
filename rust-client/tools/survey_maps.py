#!/usr/bin/env python3
"""Confere os parsers de mapa contra TODOS os mapas empacotados do cliente.

Extrai os `.stm`, `.map`, `.vcl`, `.lm` e `.ttb` de `Map_stm.pak`, `Map_light.pak` e `Map_tif.pak`
(mais os arquivos soltos de `Data\\Map`) para uma pasta de trabalho e roda `corum-assets` em cada
mapa. Serve como regressão: rode depois de mexer em `stm.rs`, `vcl.rs`, `lightmap.rs` ou
`map_script.rs`.

Uso (depois de `cargo build -p corum-assets`):

    python tools/survey_maps.py "D:\\Games\\CorumOnline\\Data" target/verification/maps

Verificações por mapa:
  * `stm-info`: o arquivo é lido até o fim (sem `unread_objects`) e sem erro de parse;
  * `vcl-info`: nº de cores do `.vcl` == vértices dos objetos tipo 0 e 1;
  * `lm-info`: cada objeto tipo 3 casa com o cabeçalho do seu registro do `.lm`;
  * `map-info`: o script `.map` é lido.

Estado registrado em 2026-09-19 (196 mapas): 0 erros de parse, `.vcl` exato em 195, `.lm` sem
divergência em 196, `.map` lido em 196. A exceção do `.vcl` é o mapa `1` (ver README do corum-assets).
"""
import collections
import os
import struct
import subprocess
import sys
from pathlib import Path

EXTENSIONS = {"stm", "map", "vcl", "lm", "ttb"}
PACKAGES = ("Map_stm", "Map_light", "Map_tif")


def extract(data_dir: Path, work: Path) -> int:
    """Extrai os arquivos de mapa dos pacotes (formato PAK versão 1) e da pasta `Map`."""
    work.mkdir(parents=True, exist_ok=True)
    written = 0
    for package in PACKAGES:
        pak = data_dir / package / f"{package}.pak"
        if not pak.is_file():
            continue
        blob = pak.read_bytes()
        count = struct.unpack_from("<I", blob, 4)[0]
        offset = 92
        for _ in range(count):
            total, size, name_size, _stored = struct.unpack_from("<4I", blob, offset)
            name = blob[offset + 32 : offset + 32 + name_size].decode("latin1")
            start = offset + 32 + name_size + 1
            if name.rsplit(".", 1)[-1].lower() in EXTENSIONS and "/" not in name and "\\" not in name:
                (work / name).write_bytes(blob[start : start + size])
                written += 1
            offset += total
    for path in (data_dir / "Map").glob("*"):
        if path.suffix.lstrip(".").lower() in EXTENSIONS and not (work / path.name).exists():
            (work / path.name).write_bytes(path.read_bytes())
            written += 1
    return written


def run(exe: Path, *args: str) -> tuple[int, str, str]:
    process = subprocess.run(
        [str(exe), *args], capture_output=True, text=True, encoding="utf-8", errors="replace"
    )
    return process.returncode, process.stdout, process.stderr


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    data_dir, work = Path(sys.argv[1]), Path(sys.argv[2])
    exe = Path(__file__).resolve().parent.parent / "target" / "debug" / "corum-assets.exe"
    if not exe.is_file():
        print(f"Compile antes: cargo build -p corum-assets  (esperado em {exe})")
        return 2
    print(f"extraídos {extract(data_dir, work)} arquivos para {work}")

    names = sorted(
        {p.stem for p in work.glob("*.stm")}, key=lambda n: (0, int(n)) if n.isdigit() else (1, n)
    )
    summary: collections.Counter[str] = collections.Counter()
    problems: list[tuple[str, str, str]] = []
    for name in names:
        summary["mapas"] += 1
        stm = work / f"{name}.stm"
        code, out, err = run(exe, "stm-info", str(stm))
        if code != 0:
            problems.append((name, "STM erro", err.strip()[:140]))
            summary["stm_erro"] += 1
            continue
        info = dict(line.split(": ", 1) for line in out.splitlines() if ": " in line and line[0] != " ")
        if int(info.get("unread_objects", "0")):
            problems.append((name, "STM objetos não lidos", info["unread_objects"]))
            summary["stm_nao_lidos"] += 1
        if int(info.get("skipped_objects", "0")):
            summary["mapas_com_tipos_pulados"] += 1  # tipo 48 (billboard) e outros: informativo

        for extension, command, label in (("vcl", "vcl-info", "VCL"), ("lm", "lm-info", "LM")):
            path = work / f"{name}.{extension}"
            if not path.exists():
                continue
            code, out, err = run(exe, command, str(path), str(stm))
            bad = code != 0 or "match: NO" in out or "MISMATCH" in out or "missing" in out
            if bad:
                problems.append((name, f"{label} diverge", (out + err).replace("\n", " | ").strip()[:140]))
                summary[f"{extension}_diverge"] += 1
            else:
                summary[f"{extension}_ok"] += 1

        script = work / f"{name}.map"
        if script.exists():
            code, _out, err = run(exe, "map-info", str(script))
            if code != 0:
                problems.append((name, "MAP erro", err.strip()[:140]))
                summary["map_erro"] += 1
            else:
                summary["map_ok"] += 1

    print(dict(summary))
    for problem in problems:
        print(problem)
    print(f"problemas: {len(problems)}")
    return 1 if any(kind not in ("STM objetos não lidos",) and kind.endswith("erro") for _, kind, _ in problems) else 0


if __name__ == "__main__":
    sys.exit(main())
