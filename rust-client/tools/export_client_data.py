#!/usr/bin/env python3
"""Exporta tudo que o cliente Rust vai precisar do cliente instalado, numa pasta só.

Resultado (a pasta NÃO vai para o git: são arquivos do jogo, ~1,2 GB):

    <saida>/
      paks/<Pacote>/...        conteúdo de cada .pak (via `corum-assets extract-all`)
      loose/<Pasta>/...        arquivos soltos de Data (Map, Cdt, Sound, Cursor, Manager)
      manager_decoded/*.bin    tabelas .cdb já decifradas (`corum-assets cdb-decode-all`)
      system/*.tsv             tabelas de jogo editáveis, uma coluna por campo (o "System" à la Lineage 2)
      resources/*.tsv         id de recurso -> caminho (`.erd` decodificados, `0xID<TAB>caminho`)
      config/...               .erd, .ini e Paklist.sin da raiz do cliente
      manifest.json            arquivo -> origem, tamanho, sha256; contagem por extensão

Uso (depois de `cargo build -p corum-assets`):

    python tools/export_client_data.py "D:\\Games\\CorumOnline" export

É idempotente: rodar de novo sobrescreve os mesmos arquivos. `--sem-hash` pula o sha256.
"""
import collections
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

LOOSE_DIRECTORIES = ("Map", "Cdt", "Sound", "Cursor", "Manager")
CONFIG_SUFFIXES = {".erd", ".ini", ".sin"}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def run(exe: Path, *arguments: str) -> str:
    result = subprocess.run(
        [str(exe), *arguments], capture_output=True, text=True, encoding="utf-8", errors="replace"
    )
    if result.returncode != 0:
        raise SystemExit(f"falhou: {' '.join(arguments)}\n{result.stderr.strip()}")
    return result.stdout


def run_quiet(exe: Path, *arguments: str) -> None:
    print("  " + run(exe, *arguments).strip().splitlines()[-1])


def main() -> int:
    hashing = "--sem-hash" not in sys.argv
    positional = [argument for argument in sys.argv[1:] if not argument.startswith("--")]
    if len(positional) != 2:
        print(__doc__)
        return 2
    client, out = Path(positional[0]), Path(positional[1])
    data = client / "Data"
    exe = Path(__file__).resolve().parent.parent / "target" / "debug" / "corum-assets.exe"
    if not exe.is_file() or not data.is_dir():
        print(f"Precisa de {exe} (cargo build -p corum-assets) e de {data}")
        return 2

    for pak in sorted(data.glob("*/*.pak")):
        print(f"pacote {pak.name}")
        run_quiet(exe, "extract-all", str(pak), str(out / "paks" / pak.stem))

    for name in LOOSE_DIRECTORIES:
        source = data / name
        if source.is_dir():
            print(f"soltos {name}")
            shutil.copytree(source, out / "loose" / name, dirs_exist_ok=True)

    print("tabelas cdb")
    run_quiet(exe, "cdb-decode-all", str(data / "Manager"), str(out / "manager_decoded"))

    print("system (tabelas editáveis em TSV)")
    system = run(exe, "cdb-export-tsv", str(data / "Manager"), str(out / "system"))
    print("  " + system.strip().splitlines()[-1])
    cdt = run(exe, "cdt-export-tsv", str(data / "Cdt"), str(out / "system" / "Cdt.tsv"))
    print("  " + cdt.strip().splitlines()[-1])

    print("tabelas de recursos (.erd)")
    (out / "resources").mkdir(parents=True, exist_ok=True)
    for erd in sorted(client.glob("*.erd")):
        table = run(exe, "erd-dump", str(erd))
        (out / "resources" / f"{erd.stem}.tsv").write_text(table, encoding="utf-8")
        print(f"  {erd.name}: {len(table.splitlines())} recursos")

    config = out / "config"
    config.mkdir(parents=True, exist_ok=True)
    for file in client.iterdir():
        if file.is_file() and file.suffix.lower() in CONFIG_SUFFIXES:
            shutil.copy2(file, config / file.name)

    files = {}
    extensions: dict[str, collections.Counter[str]] = collections.defaultdict(collections.Counter)
    for path in sorted(p for p in out.rglob("*") if p.is_file() and p.name != "manifest.json"):
        relative = path.relative_to(out).as_posix()
        area = relative.split("/")[0]
        files[relative] = {"size": path.stat().st_size}
        if hashing:
            files[relative]["sha256"] = sha256(path)
        extensions[area][path.suffix.lower() or "(sem extensão)"] += 1

    manifest = {
        "source": str(client),
        "files": files,
        "extensions_by_area": {area: dict(count.most_common()) for area, count in extensions.items()},
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1), encoding="utf-8")
    total = sum(entry["size"] for entry in files.values())
    print(f"{len(files)} arquivos, {total / 1e9:.2f} GB em {out}")
    for area, count in manifest["extensions_by_area"].items():
        print(f"  {area}: {dict(list(count.items())[:8])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
