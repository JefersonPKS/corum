#!/usr/bin/env python3
"""Mede quantas malhas `.MOD` os pacotes do cliente já decodificam (regressão do parser).

Extrai os `.mod` de `Character.pak`, `Monster.pak`, `Npc.pak` e `Map_chr.pak` e roda
`corum-assets mod-info` em cada um. Rode depois de mexer em `model.rs`.

Uso (depois de `cargo build -p corum-assets`):

    python tools/survey_models.py "D:\\Games\\CorumOnline\\Data" target/verification/mods

Estado registrado em 2026-09-19 (o total de malhas conta cada registro `F4`):

    Character  1.117 de 1.197 malhas (93%), 831 de 905 modelos completos
    Map_chr    7.000 de 7.051 malhas (99%), 363 de 378 modelos completos
    Monster    1.202 de 1.219 malhas (99%), 176 de 189 modelos completos
    Npc           45 de 45 malhas (100%),    22 de 22 modelos completos

`malhas_com_pele` conta as malhas cujo bloco de influências (osso, peso, offset) foi lido: 167 em
`Character`, 303 em `Monster`, 25 em `Npc` e 5 em `Map_chr`.
"""
import collections
import re
import struct
import subprocess
import sys
from pathlib import Path

PACKAGES = ("Character", "Monster", "Npc", "Map_chr")


def extract_mods(pak: Path, out: Path, prefix: str) -> list[Path]:
    blob = pak.read_bytes()
    count = struct.unpack_from("<I", blob, 4)[0]
    offset = 92
    paths = []
    for _ in range(count):
        total, size, name_size, _stored = struct.unpack_from("<4I", blob, offset)
        name = blob[offset + 32 : offset + 32 + name_size].decode("latin1")
        start = offset + 32 + name_size + 1
        if name.lower().endswith(".mod") and "/" not in name and "\\" not in name:
            path = out / f"{prefix}_{name}"
            path.write_bytes(blob[start : start + size])
            paths.append(path)
        offset += total
    return paths


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    data_dir, out = Path(sys.argv[1]), Path(sys.argv[2])
    exe = Path(__file__).resolve().parent.parent / "target" / "debug" / "corum-assets.exe"
    if not exe.is_file():
        print(f"Compile antes: cargo build -p corum-assets  (esperado em {exe})")
        return 2
    out.mkdir(parents=True, exist_ok=True)

    for package in PACKAGES:
        pak = data_dir / package / f"{package}.pak"
        if not pak.is_file():
            print(f"{package}: pacote não encontrado em {pak}")
            continue
        total: collections.Counter[str] = collections.Counter()
        issues: collections.Counter[str] = collections.Counter()
        for path in extract_mods(pak, out, package):
            run = subprocess.run(
                [str(exe), "mod-info", str(path)],
                capture_output=True, text=True, encoding="utf-8", errors="replace",
            )
            total["modelos"] += 1
            if run.returncode != 0:
                total["modelos_com_erro_de_arquivo"] += 1
                issues[re.sub(r"0x[0-9A-F]+", "0x?", run.stderr.strip()[:80])] += 1
                continue
            meshes = [line for line in run.stdout.splitlines() if "exportable=" in line]
            ok = sum("exportable=true" in line for line in meshes)
            total["malhas"] += len(meshes)
            total["malhas_ok"] += ok
            total["malhas_com_pele"] += sum("skinned=true" in line for line in meshes)
            if meshes and ok == len(meshes):
                total["modelos_completos"] += 1
            for line in meshes:
                if "exportable=false" in line and "issue=" in line:
                    issue = line.split("issue=", 1)[1]
                    issues[re.sub(r"\d+", "N", re.sub(r"0x[0-9A-F]+", "0x?", issue))[:90]] += 1
        share = 100 * total["malhas_ok"] / max(total["malhas"], 1)
        print(f"{package}: {dict(total)}  ({share:.0f}% das malhas)")
        for issue, n in issues.most_common(3):
            print(f"    {n:4d}  {issue}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
