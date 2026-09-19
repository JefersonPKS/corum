# corum-assets

Leitor seguro dos contêineres `.pak` do cliente Corum Online.

O formato foi validado nos 13 pacotes encontrados em `D:\Games\CorumOnline`: 10.990 registros foram percorridos e todos terminaram exatamente no último byte de seus arquivos.

## Formato PAK versão 1

Todos os inteiros são `u32` little-endian.

### Cabeçalho — 92 bytes

| Offset | Tamanho | Campo |
|---:|---:|---|
| `0x00` | 4 | versão (`1`) |
| `0x04` | 4 | quantidade de registros |
| `0x08` | 4 | flags |
| `0x0c` | 4 | tamanho do nome do pacote |
| `0x10` | 76 | nome do pacote/reservado |

### Registro — 32 bytes + nome + dados

| Offset relativo | Tamanho | Campo |
|---:|---:|---|
| `0x00` | 4 | tamanho total do registro |
| `0x04` | 4 | tamanho dos dados |
| `0x08` | 4 | tamanho do nome, sem NUL |
| `0x0c` | 4 | offset absoluto do registro |
| `0x10` | 16 | reservado |
| `0x20` | variável | nome + terminador NUL |
| seguinte | variável | dados do arquivo |

Invariante observada:

```text
tamanho_total = 32 + tamanho_nome + 1 + tamanho_dados
```

Não há compressão adicional no nível do contêiner.

## Formatos de personagem

### CHR

O `.chr` é um manifesto textual. Ele contém `*MOD_FILE_NAME`, `*MOTION_NUM` e, em seguida, os nomes dos arquivos `.anm`. O parser valida a contagem declarada. Dos 617 manifestos do pacote `Character`, 616 são consistentes; `pm1245_005.chr` declara 450 animações, mas contém 500 nomes.

### MOD versão 1

O contêiner estrutural do modelo foi identificado:

| Offset | Campo observado |
|---:|---|
| `0x00` | versão (`1`) |
| `0x04` | quantidade total de nós |
| `0x08` | quantidade de materiais |
| `0x0c` | quantidade de registros de malha |
| `0x18` | quantidade de ossos |
| `0x1c` | primeiro registro de material |

Os materiais usam a tag `0x00F00000`. Os registros de cena encontrados usam `0xF4000000` para malhas, `0xF5000000` para ossos/nós e, em poucos modelos, `0xF1000000` para um tipo especial ainda não interpretado. Cada registro contém seu tamanho, permitindo percorrer e validar o arquivo sem depender de ponteiros gravados pela ferramenta antiga.

No payload `F4`, foram confirmados:

- nome do objeto;
- índice do pai e flags;
- contagens de vértices, vértices de textura e costuras;
- pivot;
- posições e UVs no layout estático;
- grupos de faces, material e índices dos triângulos.

O exportador OBJ atual cobre malhas estáticas nas quais vértices e UVs têm correspondência direta e não existem costuras adicionais. No pacote `Character`, todos os 905 arquivos MOD passam pela leitura estrutural: são 1.197 malhas, das quais 53 já são diretamente exportáveis. As outras 1.144 precisam da tabela de remapeamento de costuras e dos pesos de skinning.

### ANM versão 1

O cabeçalho de 160 bytes contém versão, ticks por frame, primeiro/último frame, velocidade, duração e nome. Em seguida aparecem registros com tag `0x0000F000` e tamanho explícito.

Três tipos de keyframe foram separados por tamanho:

| Track provisória | Bytes por keyframe | Conteúdo observado |
|---|---:|---|
| `track_24` | 24 | tick, índice e quatro `f32` |
| `track_20` | 20 | tick, índice e três `f32` |
| `track_36` | 36 | tick, índice e sete `f32` |

As três semânticas finais ainda serão confirmadas contra o motor, mas a divisão binária é consistente: 3.875 registros sem morph do pacote `Effect` obedecem exatamente a essa equação, sem divergências. Todos os 197 ANM de `Effect` e os 259 de `Character` são aceitos. A quinta track é morph por vértice e tem tamanho dependente da malha; por enquanto o parser preserva sua contagem e extensão sem interpretá-la.

## Texturas DDS

`dds::DecodedImage::from_dds` decodifica o maior nível de mip de DXT1, DXT3, DXT5 e RGB/RGBA de 24/32 bits para RGBA8. As 51 texturas únicas do mapa `1100` (todas DXT1, de 32x256 a 256x256) vivem em `Map_dds.pak` e são lidas com `PakArchive::read_entry`, sem extrair para disco. O material do `.stm` referencia `nome.tga`, mas o pacote guarda `nome.dds`.

## Formatos de mapa

Um mapa `N` é composto por arquivos em `Data\Map` (soltos ou dentro de `Map_stm.pak`, `Map_light.pak` etc.):

| Arquivo | Conteúdo | Parser |
|---|---|---|
| `N.ttb` | grade de colisão/atributos de tiles | `ttb::TileMap` |
| `N.map` | script textual: limites, referência ao `.stm`, objetos e luzes | `map_script::MapScript` (luzes ainda não) |
| `N.stm` | geometria estática do cenário (posições, UVs, materiais, faces) | `stm::StaticModelFile` |
| `N.vcl` | cor pré-calculada por vértice dos objetos STM tipo 1 | ainda sem parser (formato confirmado) |
| `N.lm` | lightmaps dos objetos STM tipo 3 | ainda sem parser (formato **não** decifrado) |
| `N.lfg`, `N.ofg`, `N.hfl`, `N.am2`, `N.vch` | luzes/efeitos/altura/outros | não investigados |

Tudo é little-endian. Nomes de objetos e materiais estão em code page legada (CP949): o parser usa `from_utf8_lossy`, então nomes coreanos saem com `�`. Isso é inofensivo para o desenho, mas não use esses nomes como chave estável.

### TTB

Cabeçalho de 16 bytes: `largura`, `altura`, `tamanho do tile` e `quantidade de objetos declarada` (`u32` cada). Depois vêm `largura × altura` valores `u16`, em ordem de linha (índice `z * largura + x`), e por fim um `u16` com a quantidade de seções seguido de bytes ainda não interpretados (`trailing_bytes`).

Cada `u16` de tile é dividido em nibbles:

| Bits | Campo | Observação |
|---|---|---|
| 0–3 | atributo | `1` bloqueia; `9` aparece em áreas especiais (a sandbox pinta de azul) |
| 4–7 | ocupação | diferente de 0 bloqueia |
| 8–15 | seção | índice de seção do mapa |

Um tile é caminhável quando `atributo != 1 && ocupação == 0`. No mapa `1100`: 32×32 tiles de 125 unidades (4.000 × 4.000 unidades de mundo), 231 de 1.024 caminháveis.

### MAP (script textual)

Blocos `GX_*` separados por espaço em branco, com `{ }`. Presentes em `1.map`, `1100.map`, `10001.map` e `10002.map` (`TotalMap.map` não foi investigado):

```text
GX_METADATA { BOX_MAX x y z  BOX_MIN x y z }
GX_MAP      { STATIC_MODEL 1100.stm  HEIGHT_FIELD NA }
GX_OBJECT N { recurso id  sx sy sz  px py pz  ax ay az  angulo  flags }   // N linhas
GX_LIGHT  N { ARGB  px py pz  raio  1000 }                                 // N linhas
GX_TRIGGER N { }
```

- `GX_OBJECT`: `recurso` é `.MOD` (estático) ou `.CHR` (animado, ex.: `RD_BONFIRE.CHR`). `id` pode ser `4294967280` (`-16`). `flags` é uma string hexadecimal (`100000A`, `1000008`, `8`) ainda sem significado. O `1100` declara 0 objetos; outros mapas têm (ex.: `RD_BONFIRE.CHR`, `village_01.MOD`, este último com escala não uniforme).
- `GX_LIGHT`: cor `AARRGGBB` em hexadecimal (`FF323296` = R 0x32, G 0x32, B 0x96), posição, raio e um inteiro sempre `1000` nas amostras. A contagem declarada inclui um registro terminador zerado (`0 0 0 0 0 1000`): o `1100` declara 21 e tem 20 luzes reais. O parser atual ignora esse bloco.
- Coordenadas do `BOX_MIN/MAX` estão nas mesmas unidades do STM. No `1100`: `x -3172..6954`, `y -763..763`, `z 0..9140`.

### STM versão 1

```text
0x00  cabeçalho, 16 bytes: versão (=1), quantidade de materiais, 8 bytes zerados
0x10  materiais, 168 bytes cada
      objetos, um após o outro
```

**Material (168 bytes).** O nome da textura está em `+28` (128 bytes, terminado em NUL; `.tga` no `1100`). Os 28 bytes anteriores parecem ser: versão `1`, três cores (`0x00141414`, `0x00141414`, `0x00E5E5E5`), dois `u32` zero e um `f32` `0.1` entre eles (`0`, `0.1`, `0`); os 12 bytes finais são `01 01 00 00 / 00 00 00 00 / 01 00 00 00`. Esses campos (ambiente/difuso/brilho/flags) **não são usados** pelo parser. A textura real é o `.dds` de mesmo nome em `Map_dds.pak`. Materiais podem repetir a mesma textura: o `1100` tem 53 materiais e 51 texturas únicas.

**Objeto (cabeçalho de `0x170` bytes).**

| Offset | Campo |
|---|---|
| `0xBC` | marcador `0xFFFFFFFF` (usado para achar o início do objeto) |
| `0xC0` | nome, 128 bytes |
| `0x140` | quantidade de vértices `V` |
| `0x144` | `V` de novo |
| `0x148` / `0x14C` | `A` / `B`, com `A + B = V`. `B` é a quantidade de "índices secundários" |
| `0x150` | `V` de novo |
| `0x154` | `0xFFFFFFFF` |
| `0x158` | quantidade de grupos de faces |
| `0x15C` | **tipo do objeto** (1 ou 3 nos mapas vistos) |

Depois do cabeçalho: `V` posições `[f32; 3]`, `V` UVs `[f32; 2]` e `B` índices `u32` secundários (semântica desconhecida). Em seguida, os grupos.

**Grupo (28 bytes + faces).** `+0` índice do material, `+8` quantidade de faces, `+12` a mesma quantidade repetida, `+20` quantidade de coordenadas de lightmap; `+4`, `+16` e `+24` sem significado conhecido. Depois vêm `faces × [u16; 3]` (índices no vetor de vértices do objeto).

**Fim do objeto depende do tipo:**

- **Tipo 1 (nome termina em ` V`, iluminação por vértice):** o grupo termina nas faces. Após o último grupo há 16 bytes não interpretados e `V × [f32; 3]` (provavelmente normais por vértice; o sandbox ainda usa a normal da face). O objeto tem 0 coordenadas de lightmap.
- **Tipo 3 (nome termina em ` L`, lightmap):** cada grupo é seguido por `lightmap × [f32; 2]`; o valor é sempre `3 × faces` (um UV de lightmap por canto de face). Depois dos grupos há dados ainda não decodificados, então o parser procura o próximo cabeçalho por varredura.

O `1100` tem 11 objetos tipo 1 (22.157 vértices) e 9 tipo 3 (598 faces, 1.794 UVs de lightmap).

**Riscos conhecidos do parser:**

- O laço só continua enquanto `is_object_header` aceitar o próximo offset (marcador, nome com 3+ caracteres imprimíveis e contagens ≤ 10.000.000). Se um objeto não passar na heurística, os seguintes são descartados **sem erro**. No `1100` foi verificado por varredura independente que os 20 objetos são lidos; em outros mapas, compare o número de objetos com uma varredura antes de confiar.
- Objetos de tipo diferente de 1 e 3 vão para `skipped_objects`. Ainda não apareceram.
- Alguns objetos trazem vértices com `0xCDCDCDCD` (`-431602080` como `f32`): memória não inicializada do exportador. No `1100` (objetos 0x39630, 0x6e2c2, 0x98992 e 0xc178c) nenhuma face os referencia. Quem renderiza deve descartar faces que os usem em vez de confiar nos limites do arquivo.

### VCL — cor por vértice (confirmado)

Sem cabeçalho: uma sequência de `u32` em `AARRGGBB` (na memória: bytes `B, G, R, A`; alfa `0xFF` nas amostras). No `1100`, `88.628 / 4 = 22.157` cores, exatamente a soma dos vértices dos objetos tipo 1. A hipótese de trabalho, a validar visualmente, é que as cores seguem a ordem dos objetos no arquivo `.stm` e, dentro de cada objeto, a ordem dos vértices. É a iluminação "assada" desses objetos, com tons neutros a levemente coloridos.

### LM — lightmaps (parcial)

Cabeçalho de 12 bytes: `1`, `32`, `32` (`u32`). O restante (`63.584` bytes) tem muitas palavras repetidas (`0x42282842` aparece 11.031 vezes, `0` outras 2.816) e as 512 primeiras palavras são idênticas. Não é uma imagem simples de `32×32`: falta descobrir se `32×32` é o tamanho de cada lightmap, de uma grade de células ou de um atlas, e o formato de pixel. Os objetos tipo 3 fornecem 3 UVs de lightmap por face, que devem endereçar este arquivo. Os pacotes `Map_light.pak` trazem `.lm` com tamanho 0 para alguns mapas (`619`, `604`), então esse mapa também pode simplesmente não ter lightmaps.

## Uso

```powershell
cd rust-client
cargo run -p corum-assets -- info "D:\Games\CorumOnline\Data\Character\Character.pak"
cargo run -p corum-assets -- list "D:\Games\CorumOnline\Data\Character\Character.pak"
cargo run -p corum-assets -- extract "D:\Games\CorumOnline\Data\Character\Character.pak" womanred_c_b.dds .\extracted
cargo run -p corum-assets -- chr-info .\extracted\dfymiss.chr
cargo run -p corum-assets -- mod-info .\extracted\dfymiss.mod
cargo run -p corum-assets -- mod-to-obj .\extracted\dfymiss.mod .\dfymiss.obj
cargo run -p corum-assets -- anm-info .\extracted\dfmiss.anm
```

O extrator rejeita caminhos absolutos, componentes `..`, nomes sem terminador e registros que ultrapassem o tamanho do arquivo.
