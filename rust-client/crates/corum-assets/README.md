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

### Geometria da malha (decifrada em 2026-09-19)

Depois do cabeçalho de `0x174` bytes do payload `F4`, com `V` vértices, `T` UVs e `S` costuras (`V = T + S`):

| Trecho | Tamanho | Conteúdo |
|---|---|---|
| posições | `V × 12` | `[f32; 3]` por vértice |
| UVs | `T × 8` | UVs dos vértices "regulares" |
| **UVs de costura** | `S × 8` | UVs dos vértices duplicados ao longo de costuras de textura. Com os `T` anteriores há **um UV por vértice** (`MeshGeometry::texture_coordinates`) |
| **fontes das costuras** | `S × 4` | `u32` por vértice de costura: índice `< T` do vértice que ele duplica (`MeshGeometry::seam_sources`). Os pesos de skinning são guardados só para os `T` primeiros, então é por aqui que a costura herda os pesos **[hipótese]** |
| grupos de faces | variável, **sem contagem** | grupos consecutivos, lidos enquanto o padrão vale |
| resto | variável | ainda não decodificado (ver abaixo) |

Um grupo tem o **mesmo cabeçalho do STM**: 28 bytes `material, x, faces, faces, 0, ?, 0`, seguidos de `faces` triângulos com três `u16` cada, **sem preenchimento** (o alinhamento a 4 bytes foi testado em ~2.100 malhas e a leitura sem padding é igual ou melhor em todas). `material` indexa a tabela de materiais do modelo; o significado de `x` é desconhecido.

**Como foi descoberto:** a relação `T + S = V` apareceu num farol estático de `Map_chr.pak` (`lighthouse-blue`). O mapa de blocos do payload (2.040 bytes de floats = 170 × 12, `V × 12` de floats, 3 registros por grupo) e a regra de que o prefixo depois dos UVs de costura tem exatamente `S` palavras vieram da varredura de ~2.100 malhas; a leitura dos grupos foi validada no `Object01` (16 vértices), onde os materiais 9, 8, 7 e 6 e os triângulos `(5,9,10),(10,11,5)` aparecem exatamente como esperado. Um parser antigo assumia "contagem de grupos + cabeçalho de 24 bytes" e só cobria 5% das malhas: a "contagem" era, na verdade, o primeiro campo de um cabeçalho de grupo.

**Resultado (`tools/survey_models.py`, contando registros `F4`):**

| Pacote | Malhas decodificadas | Modelos completos |
|---|---:|---:|
| `Character` | 1.117 de 1.197 (93%) | 831 de 905 |
| `Map_chr` | 7.000 de 7.051 (99%) | 363 de 378 |
| `Monster` | 1.202 de 1.219 (99%) | 176 de 189 |
| `Npc` | 45 de 45 (100%) | 22 de 22 |

Antes eram 53, 338, 22 e 0. Confirmado na tela: um monstro alado (`Monster_m00630`) e um NPC humanoide (`Npc_npc007`) saem com forma e proporções reconhecíveis.

**Ainda não decodificado:**

- ~80 malhas de `Character`, ~50 de `Map_chr` e ~17 de `Monster` (erros do tipo "campo truncado" ou "contagem irracional" no meio do payload; layout de grupo ou vértices diferente, a investigar com `tools/survey_models.py`);
- o restante do payload de cada malha (a "cauda", depois dos grupos). Medido em ~2.000 malhas com o parser atual:
  - malhas mais simples (sem dados de skinning): `12 bytes + V × 12` (três palavras zero e depois `V` vetores unitários, provavelmente normais por vértice **[hipótese]**); é o que 547 das 1.621 malhas medidas têm (incluindo peças de modelos com ossos);
  - malhas **com esqueleto** têm mais dados, de tamanho variável, e não seguem nenhuma fórmula linear em `V`, `F`, `G` ou `T` (ajuste por mínimos quadrados com erro máximo de ~30 KB). Nas menores (`V = 4`) a cauda é um cabeçalho de 8 palavras `V, T, T, 1, 0x10100, ?, ?, nº de ossos usados`, depois **um registro de 8 palavras por vértice** `osso (u32), peso (1.0), posição local ao osso (3 × f32), normal local (3 × f32)`, e no fim as `V × 12` normais **[hipótese, lida a olho em uma malha de 4 vértices]**. Malhas grandes (a lula `Character_pm1277`, 1.216 vértices, 27 ossos) têm uma tabela com passo regular (`3, 7, 11, 15…` e `1025, 2049, 3073…`) que ainda não foi decifrada;
- pesos de skinning e a ligação de cada malha a um osso. **Correção:** as posições cruas dos modelos já são a **pose de bind** (um NPC humano, um monstro alado, um diabrete e um espectro saem inteiros sem aplicar nenhum osso), então não é preciso hierarquia de nós para *mostrar* o modelo parado. A hierarquia (`F5`, `pivot`, `parent_index`) e o skinning só entram para **animar**. A aparência "embaralhada" de `Character_pm1277_000` era uma lula de tentáculos abertos, não um erro.

### ANM versão 1

O cabeçalho de 160 bytes contém versão, ticks por frame, primeiro/último frame, velocidade, duração e nome. Em seguida aparecem registros com tag `0x0000F000` e tamanho explícito.

Três tipos de keyframe foram separados por tamanho:

| Track provisória | Bytes por keyframe | Conteúdo observado |
|---|---:|---|
| `track_24` | 24 | tick, índice e quatro `f32` |
| `track_20` | 20 | tick, índice e três `f32` |
| `track_36` | 36 | tick, índice e sete `f32` |

As três semânticas finais ainda serão confirmadas contra o motor, mas a divisão binária é consistente: 3.875 registros sem morph do pacote `Effect` obedecem exatamente a essa equação, sem divergências. Todos os 197 ANM de `Effect` e os 259 de `Character` são aceitos. A quinta track é morph por vértice e tem tamanho dependente da malha; por enquanto o parser preserva sua contagem e extensão sem interpretá-la.

## Texturas (DDS e TIFF)

`dds::DecodedImage::from_dds` decodifica o maior nível de mip de DXT1, DXT3, DXT5 e RGB/RGBA de 24/32 bits para RGBA8. As 51 texturas únicas do mapa `1100` (todas DXT1, de 32x256 a 256x256) vivem em `Map_dds.pak` e são lidas com `PakArchive::read_entry`, sem extrair para disco. O material do `.stm` referencia `nome.tga`, mas o pacote guarda `nome.dds` ou `nome.tif`.

`DecodedImage::from_tiff` (`tiff.rs`) lê os TIFFs de `Map_tif.pak`: little-endian, **sem compressão**, RGB de 8 bits com um 4º canal de alfa (ou 3 ou 5 canais; o excedente é ignorado). Em 119 dos 400 arquivos a assinatura `0x002A` do cabeçalho vem trocada por `0xCCCC`, e o restante do arquivo é padrão, então a assinatura não é verificada. O alfa é real: recortes duros (grama, árvores, `min 0`) e translucidez (água, `min 90..210`).

Cobertura medida nos 196 mapas empacotados (7.921 materiais): 7.628 em `Map_dds.pak`, 287 em `Map_tif.pak` e 6 sem textura (4 com nome vazio e `wall_9_skell.tga`). Um `thumbs.db` perdido dentro de `Map_tif.pak` é ignorado. `Map_tif.pak` também guarda alguns `.vcl` e `.lm` (ex.: `1206`).

## Formatos de mapa

Um mapa `N` é composto por arquivos em `Data\Map` (soltos ou dentro de `Map_stm.pak`, `Map_light.pak` etc.):

| Arquivo | Conteúdo | Parser |
|---|---|---|
| `N.ttb` | grade de colisão/atributos de tiles | `ttb::TileMap` |
| `N.map` | script textual: limites, referência ao `.stm`, objetos e luzes | `map_script::MapScript` (`GX_LIGHT` incluído) |
| `N.stm` | geometria estática do cenário (posições, UVs, materiais, faces) | `stm::StaticModelFile` |
| `N.vcl` | cor pré-calculada por vértice dos objetos STM tipo 0 e 1 | `vcl::VertexColors` |
| `N.lm` | lightmaps (RGB565) dos objetos STM tipo 3 | `lightmap::LightmapFile` |
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
- `GX_LIGHT`: cor `AARRGGBB` em hexadecimal (`FF323296` = R 0x32, G 0x32, B 0x96), posição, raio e um inteiro sempre `1000` nas amostras. A contagem declarada inclui um registro terminador zerado (`0 0 0 0 0 1000`): o `1100` declara 21 e tem 20 luzes reais. `MapScript::lights` já traz as luzes reais (sem o terminador); `MapLight::rgb()` converte a cor.
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
| `0x15C` | **campo de tipo**: o **byte baixo** é o tipo (0, 1, 3 ou 48); os bytes altos são outra coisa (`type_flags`, ver abaixo) |

Depois do cabeçalho: `V` posições `[f32; 3]`, `V` UVs `[f32; 2]` e `B` índices `u32` secundários (semântica desconhecida). Em seguida, os grupos.

**Grupo (28 bytes + faces).** `+0` índice do material, `+8` quantidade de faces, `+12` a mesma quantidade repetida, `+20` quantidade de coordenadas de lightmap; `+4`, `+16` e `+24` sem significado conhecido. Depois vêm `faces × [u16; 3]` (índices no vetor de vértices do objeto).

**Tipos (byte baixo do campo em `0x15C`), medidos em 196 mapas (~27.600 objetos):**

| Tipo | Objetos | Layout | Iluminação |
|---:|---:|---|---|
| 0 | ~10.700 | igual ao tipo 1 (o fim do objeto cai exatamente no próximo cabeçalho em 99%) | cor por vértice (`.vcl`) |
| 1 | ~7.500 | descrito abaixo | cor por vértice (`.vcl`) |
| 3 | ~9.000 | com lightmap, descrito abaixo | lightmap (`.lm`) |
| 48 | 153 | como o tipo 3, sem UVs de lightmap, e 16 bytes finais; nomes ` BILLBOARD*`, 4 vértices | placa que gira para a câmera (não desenhada ainda) |

Nomes com `ALP`/`Alp`/`alpha` aparecem no tipo 0, o que sugere transparência **[hipótese]**. `type_flags` (bits acima do byte de tipo) é 0 no `1100`; em outros mapas vale 5, 10, 12, 20, 30, 40, 50 ou 60 (múltiplos de 10 na maioria; talvez um percentual de transparência **[hipótese]**). Só o byte baixo decide o layout.

**Fim do objeto depende do tipo:**

- **Tipos 0 e 1 (o tipo 1 tem nome terminado em ` V`, iluminação por vértice):** o grupo termina nas faces. Após o último grupo há 16 bytes não interpretados e `V × [f32; 3]` (provavelmente normais por vértice; o sandbox ainda usa a normal da face). O objeto tem 0 coordenadas de lightmap.
- **Tipo 3 (nome termina em ` L`, lightmap):** cada grupo é seguido por `lightmap × [f32; 2]`; o valor é sempre `3 × faces` (um UV de lightmap por canto de face, em ordem de face, lidos em `StaticFaceGroup::lightmap_coordinates`). Depois dos grupos vêm 12 bytes zerados e o cabeçalho `(primeiro campo, largura, altura)` do registro `.lm` do objeto (`StaticObject::lightmap`), seguido de floats ainda não interpretados (parecem uma normal/plano e limites do objeto). O parser procura o próximo cabeçalho de objeto por varredura.

O `1100` tem 11 objetos tipo 1 (22.157 vértices), nenhum tipo 0 e 9 tipo 3 (598 faces, 1.794 UVs de lightmap).

**Como o parser acha os objetos, e o que ele garante.** Um cabeçalho é aceito quando tem o marcador `0xFFFFFFFF` em `0xBC`, um nome não vazio sem caracteres de controle e os contadores coerentes: `V, V, A, B, V` em `0x140..0x150` com `A + B = V`. Nomes curtos existem (`09`), por isso o tamanho do nome não conta; a regra dos contadores é o que separa cabeçalho de dado. Se o fim calculado de um objeto não cair em um cabeçalho (há objetos com preenchimento fora do layout), o parser ressincroniza no próximo cabeçalho em vez de parar. `StaticModelFile::unread_object_offsets` lista qualquer cabeçalho que sobrar depois de o laço terminar (vazio = arquivo lido até o fim); `stm-info` mostra o total como `unread_objects`.

Resultado nos 196 mapas: 0 erros de parse, 0 objetos não lidos. Antes dessa regra, 5 mapas perdiam objetos em silêncio (`1`, `750`, `917`, `919` e `1002`), e o `.vcl` deles não fechava.

**Riscos conhecidos do parser:**

- Objetos de tipo diferente de 0, 1 e 3 (na prática o 48) vão para `skipped_objects`, sem geometria. O laço de objetos os atravessa por varredura.
- O mapa `1` é uma exceção não resolvida: 221 dos 222 objetos são tipo 0 (98.055 vértices) e o `.vcl` tem 87.063 cores; o `.lm` tem 257 registros para um único objeto tipo 3. É o mapa mais antigo (2006) e parece montado de outro jeito; qualquer suposição sobre ele é **[hipótese]**.
- Alguns objetos trazem vértices com `0xCDCDCDCD` (`-431602080` como `f32`): memória não inicializada do exportador. No `1100` (objetos 0x39630, 0x6e2c2, 0x98992 e 0xc178c) nenhuma face os referencia. Quem renderiza deve descartar faces que os usem em vez de confiar nos limites do arquivo.

### VCL — cor por vértice (confirmado, parser em `vcl::VertexColors`)

Sem cabeçalho: uma sequência de `u32` em `AARRGGBB` (na memória: bytes `B, G, R, A`; alfa `0xFF` nas amostras). No `1100`, `88.628 / 4 = 22.157` cores, exatamente a soma dos vértices dos objetos tipo 1. Nos outros mapas o `.vcl` cobre os vértices dos objetos **tipo 0 e 1**: **195 dos 196 mapas** empacotados batem exatamente (a exceção é o mapa `1`). As cores seguem a ordem dos objetos tipo 1 no `.stm` e, dentro de cada objeto, a ordem dos vértices. Isso foi verificado: a diferença média de luminância entre vértices ligados por uma aresta é 3,9, contra 16,6 entre pares aleatórios do mesmo objeto (o alfa é sempre `0xFF` e a luminância varia de 70 a 252). `corum-assets vcl-info <N.vcl> <N.stm>` confere a contagem. É a iluminação "assada" desses objetos, com tons neutros a levemente coloridos.

### LM — lightmaps (decifrado, parser em `lightmap::LightmapFile`)

O arquivo é uma sequência de registros, sem cabeçalho geral, que termina exatamente no fim do arquivo:

```text
u32  primeiro campo   (1 na maioria; 6 e 38 nos dois atlas do 1100; significado desconhecido)
u32  largura
u32  altura
     largura × altura texels RGB565, little-endian (2 bytes cada)
```

**[confirmado no `1100`]** 9 registros (32×32 ×7, 64×128 e 128×128) percorrem os 63.596 bytes sem sobra. Há um registro por objeto STM tipo 3, **na ordem em que os objetos aparecem no `.stm`**: o objeto repete `(primeiro campo, largura, altura)` do seu registro nos dados finais (ver o STM), e os 9 pares conferem (`corum-assets lm-info <N.lm> <N.stm>`).

O conteúdo é iluminação: um cinza uniforme (`0x4228`, ≈ RGB 66/69/66) com "poças" de luz quente e azulada, além de regiões pretas não usadas nos atlas. Quatro dos seis mapas 32×32 do piso são totalmente uniformes. Os UVs que apontam para os registros são os 3 por face guardados no STM. Um `.lm` de 0 bytes (como `619` e `604` em `Map_light.pak`) é válido e significa "sem lightmaps". Nos 196 mapas, o emparelhamento objeto↔registro (cabeçalho repetido) confere em todos, exceto o caso já citado do mapa `1`.

Fica em aberto: o significado do primeiro campo e se o valor de fábrica do cinza (`0x4228`) é um ambiente constante do mapa.

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
cargo run -p corum-assets -- extract-all "D:\Games\CorumOnline\Data\Map_stm\Map_stm.pak" .\maps

# mapas (aceitam arquivos soltos ou extraídos)
cargo run -p corum-assets -- ttb-info "D:\Games\CorumOnline\Data\Map\1100.ttb"
cargo run -p corum-assets -- map-info "D:\Games\CorumOnline\Data\Map\1100.map"   # inclui luzes
cargo run -p corum-assets -- stm-info "D:\Games\CorumOnline\Data\Map\1100.stm"   # mostra unread_objects
cargo run -p corum-assets -- vcl-info "D:\Games\CorumOnline\Data\Map\1100.vcl" "D:\Games\CorumOnline\Data\Map\1100.stm"
cargo run -p corum-assets -- lm-info  "D:\Games\CorumOnline\Data\Map\1100.lm"  "D:\Games\CorumOnline\Data\Map\1100.stm"
```

Para conferir todos os mapas de uma vez (extrai os arquivos dos pacotes e roda `stm-info`, `vcl-info`, `lm-info` e `map-info` em cada um):

```powershell
cargo build -p corum-assets
python tools/survey_maps.py "D:\Games\CorumOnline\Data" target/verification/maps
```

O extrator rejeita caminhos absolutos, componentes `..`, nomes sem terminador e registros que ultrapassem o tamanho do arquivo.
