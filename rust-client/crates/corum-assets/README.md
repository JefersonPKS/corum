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

Um grupo tem o **mesmo cabeçalho do STM**: 28 bytes `material, x, faces, faces, 0, ?, 0`, seguidos de `faces` triângulos com três `u16` cada, **sem preenchimento** (o alinhamento a 4 bytes foi testado em ~2.100 malhas e a leitura sem padding é igual ou melhor em todas). O significado de `x` é desconhecido.

**O campo `material` de um grupo não é um índice, é o _seletor_ do material.** O primeiro material do modelo não tem seletor e responde ao valor `1`; os demais têm seletores em ordem decrescente. Exemplos medidos: o farol tem materiais `[None, 10, 9, …, 2]` e grupos numerados `10..1`; o NPC `npc007`, `[None, 7, 6, 5, 4, 3, 2]` e grupos `7..1`; o pinheiro `ks-m2tree`, `[None, 2]` e grupos `2, 1`. `ModelFile::material_index_for_group` faz a tradução. Ler o valor como índice de 0 passa despercebido quando todos os materiais usam a mesma textura (os NPCs), mas deixa grupos sem textura em modelos como a grama e as árvores de `Map_chr`.

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

### Nós, esqueleto e pele (decifrado em 2026-09-19)

**Nós.** Cada registro de malha (`F4`) e de osso (`F5`) começa com o mesmo cabeçalho de `0x174` bytes, lido por `ModelNode`:

| Palavra | Conteúdo |
|---:|---|
| 0 | **identificador** do nó (`i32`, contando para baixo pelo arquivo: 41, 40, … 0) |
| 15–30 | **matriz de mundo** 4×4 da pose de bind (vetor-linha, translação na última linha) |
| 31–46 | a **inversa** dessa matriz (`mundo × inversa = I`, erro medido 1e-5) |
| 48 | identificador do **pai** (`-1` na raiz) |

A palavra 0 já foi lida como "índice do pai" (`parent_index`); era o identificador. Os filhos vêm antes dos pais no arquivo. A matriz local do bind é `mundo × inversa_do_pai`, e a pose é `mundo(t) = local(t) × mundo_do_pai(t)` (`pose::Skeleton`). Sem nenhuma trilha, a pose reproduz o bind **exatamente** (erro 0,0000 medido em dois modelos). A hierarquia de um monstro (`m00160`) tem 42 nós: 16 malhas e 26 ossos de um Biped do 3ds Max, e as pernas do ogro pendem do `Spine`, não do `Pelvis`.

**Pele.** Depois dos grupos de faces de uma malha com esqueleto vem um bloco (`MeshGeometry::skin`):

```text
V, T, T                    (u32 x 3)
V entradas de 5 bytes      nº de influências (u8), índice do primeiro registro (u32)
N registros de 32 bytes    identificador do osso (u32), peso (f32), offset (3 x f32), normal (3 x f32)
V normais de 12 bytes      (não lidas)
```

`N` é o que sobra do tamanho. O offset e a normal estão no espaço do próprio osso, e a posição do vértice é `soma(peso × (offset, 1) × mundo_do_osso)`. Os vértices de costura usam as mesmas entradas do vértice de origem. Verificado contra as posições guardadas no bind, em 495 malhas com esqueleto: **247 reconstroem com erro menor que 0,01**; nas demais a mediana do erro é 0,03 e 90% ficam abaixo de 9 unidades (a pose guardada nas posições não é exatamente a das matrizes dos nós). Isso vale para 167 malhas de `Character`, 303 de `Monster`, 25 de `Npc` e 5 de `Map_chr` (`corum-assets mod-info` mostra `skinned=true`). A maioria dos vértices tem 1 influência; há vértices com 2 a 4 (pesos 0,5, 0,33, 0,25), e um segundo osso pode ter peso 0.

As malhas **sem** bloco de pele são peças rígidas: seguem o próprio nó (`inversa_do_bind × mundo_animado`).

### ANM versão 1

O cabeçalho de 160 bytes contém versão, ticks por frame, primeiro/último frame, velocidade, duração e nome. Em seguida aparecem registros com tag `0x0000F000` e tamanho explícito.

Três tipos de keyframe foram separados por tamanho:

| Track provisória | Bytes por keyframe | Conteúdo observado |
|---|---:|---|
| `track_24` | 24 | tick, índice e quatro `f32` |
| `track_20` | 20 | tick, índice e três `f32` |
| `track_36` | 36 | tick, índice e sete `f32` |

**Semântica das trilhas (confirmada em 2026-09-19, ver "Animação" abaixo):** `track_24` é a **rotação** (quaternion `x, y, z, w`), `track_20` é a **posição local** (relativa ao pai; a raiz `Bip01` usa `track_20` com a posição absoluta) e `track_36` não apareceu em nenhum arquivo medido. As chaves são uma por quadro (`frame_index` de 0 a `last_frame`). **Ordem no arquivo:** quando um registro tem os dois tipos, as chaves de 20 bytes (posição) vêm **antes** das de 24 bytes (rotação), embora `counters` liste a contagem de rotação primeiro. Ler na ordem inversa mantém todos os totais de bytes válidos (por isso a validação antiga passava) e desalinha a segunda trilha.

A divisão binária original é consistente: 3.875 registros sem morph do pacote `Effect` obedecem exatamente a essa equação, sem divergências. Todos os 197 ANM de `Effect` e os 259 de `Character` são aceitos. A quinta track é morph por vértice e tem tamanho dependente da malha; por enquanto o parser preserva sua contagem e extensão sem interpretá-la.

## Texturas (DDS e TIFF)

`dds::DecodedImage::from_dds` decodifica o maior nível de mip de DXT1, DXT3, DXT5 e RGB/RGBA de 24/32 bits para RGBA8. As 51 texturas únicas do mapa `1100` (todas DXT1, de 32x256 a 256x256) vivem em `Map_dds.pak` e são lidas com `PakArchive::read_entry`, sem extrair para disco. O material do `.stm` referencia `nome.tga`, mas o pacote guarda `nome.dds` ou `nome.tif`.

`DecodedImage::from_tiff` (`tiff.rs`) lê os TIFFs de `Map_tif.pak`: little-endian, **sem compressão**, RGB de 8 bits com um 4º canal de alfa (ou 3 ou 5 canais; o excedente é ignorado). Em 119 dos 400 arquivos a assinatura `0x002A` do cabeçalho vem trocada por `0xCCCC`, e o restante do arquivo é padrão, então a assinatura não é verificada. O alfa é real: recortes duros (grama, árvores, `min 0`) e translucidez (água, `min 90..210`).

Cobertura medida nos 196 mapas empacotados (7.921 materiais): 7.628 em `Map_dds.pak`, 287 em `Map_tif.pak` e 6 sem textura (4 com nome vazio e `wall_9_skell.tga`). Um `thumbs.db` perdido dentro de `Map_tif.pak` é ignorado. `Map_tif.pak` também guarda alguns `.vcl` e `.lm` (ex.: `1206`).

### Flags de material e mistura (transparência, fogo, água)

`ModelMaterial::flags` nos 9.888 materiais dos 378 modelos de `Map_chr.pak`: `0x1` (5.926), `0x4` (1.924), `0x101` (1.317), `0x0` (378, o material-base), `0x104` (248), `0x10000104` (52), `0x10000004` (35) e `0x10000001` (8). Leitura, com o que foi medido e o que é hipótese:

| Bit | Evidência | Leitura |
|---|---|---|
| `0x4` | aparece em `RD_EFFECT_FIRE`, `fireb_02` (planos de chama), `Two_Window_Light1` (brilho de janela), `waterfall_a`, `pretaeffect` | **mistura aditiva** (efeito que soma luz) |
| `0x100` | paredes e blocos de `110.mod` (`JY_D1_wall_01`, `JY_D1_block_04`) | provavelmente **dois lados** **[hipótese]** |
| `0x10000000` | `waterfall_a`, `wseacolor_001` (mar), `MKY-rush04` | provavelmente **animação de UV** (rolagem) **[hipótese]**; não implementada |
| `0x1` | quase todo o resto | material comum |

Só o `0x4` foi usado. A translucidez da água não vem de flag (`water001` tem flag `0`): vem do alfa da textura.

**Classificação por conteúdo da textura**, medida:

- **Água/vidro (mistura alfa):** 21 dos 399 TIFFs com alfa têm mais de 60% dos texels com alfa parcial (água, mar, vidro, cachoeira, placas). Água: `jy_cbwwater_001` 100% parcial (alfa médio 0,79), `wseacolor_001` 100% (0,24). Recortes duros (`ks-tree02` 1%, `ks-stree01` 0%, grama 19–24%) ficam abaixo de 30%. Corte em 60%.
- **Efeitos sobre preto (aditivo):** das texturas opacas, 71 têm mais de 50% dos texels quase pretos (`max(rgb) < 12`) e são **quase todas efeitos**: `fireb_02` 0,75, `rd_effect_fire` 0,80, `fire`, `smoke_train`, `lightning`, `waterfall_b`, `w0760_effect` (brilho de arma), os prefixos `mky-*` e `kcs-fire*`. Um material normal como `lighthouse01` fica em 0,52. Corte em 70%. No preto, somar não muda nada, então "aditivo" e "preto transparente" são visualmente equivalentes.
- Por que o fogo precisava disso: `JY_D1_fire_a` tem um plano com `fireb_02` e flag `0x1` (não `0x4`); com fundo preto opaco ele aparecia como um quadrado preto.

### Nomes de textura dos modelos e orientação dos TIFF

- A extensão pedida pelo material vale: `KS-tree02.tif` pede o TIFF, enquanto materiais `.tga` estão nos pacotes como `.dds`. Isso importa porque o mesmo nome pode existir nos dois formatos com **imagens diferentes** (`ks-tree02.dds` em `Map_dds.pak` e `ks-tree02.tif` em `Map_tif.pak`). O `Paklist.sin` só lista os pacotes (`Character, DamageNumber, Effect, Item, Map_chr, Map_dds, Map_light, Map_stm, Map_tga, Map_tif, Monster, Npc, UI`) e não decide a prioridade.
- **Os TIFF são armazenados de cima para baixo mas amostrados com `v = 0` embaixo**, como TGA (nenhum tem a tag de orientação). O sinal veio do tronco de `ks-m2tree`, cujos UVs (`v` de 0,05 a 0,26, com `u` repetido) só caem na tira de casca se a base do arquivo for `v = 0`. Quem usa `from_tiff` deve inverter as linhas (o sandbox faz isso). DDS não precisa. Isso vale para os TIFF usados por modelos; para os materiais TIFF dos STM (`Map_tif.pak`) a inversão é a mesma convenção, mas não foi verificada em um caso assimétrico.
- O 4º canal dos TIFF (tag `ExtraSamples = 0`, "não especificado") é de fato o alfa: as silhuetas de galhos e copas são recortes corretos.

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

### Objetos posicionados (`GX_OBJECT`)

Levantamento: **10.282 objetos em 158 mapas** (197 mapas têm o bloco, 39 com ele vazio) (7.389 `.MOD` e 2.893 `.CHR`). O maior é o `1` (746 objetos), depois `604` (713), `1308` (584) e `10001` (555). Os recursos estão em `Map_chr.pak`; só 36 objetos (`wobj0001` a `wobj0004.mod`) não têm arquivo.

- **`.CHR`** é um manifesto (`*MOD_FILE_NAME` + animações): o objeto é o modelo apontado, animado por um `.ANM`; o sandbox o desenha na pose de bind.
- **Transformação:** `escala (x y z)`, `posição (x y z)`, `eixo (x y z)` e `ângulo` em radianos. O eixo é **sempre `(0, 1, 0)`** nos 10.282 objetos; a posição está nas unidades do STM (o sandbox usa a mesma conversão do cenário, `x * escala - largura / 2`); o ângulo vai de −10 a 7,4 e a escala de −3,4 a 3,6, com **escala negativa (espelhamento) e não uniforme** em 118 objetos.
- **Sentido do ângulo:** o script parece guardar uma rotação do Direct3D (mão esquerda). O sandbox aplica `-ângulo` na rotação de mão direita do glam e o resultado parece coerente (casas, cercas e ruínas alinhadas com o terreno), mas **não foi verificado contra o cliente original**.
- **`flags`** (`1000002` em 5.482 objetos, `1000008` em 2.075, `0` em 1.095, `100000A` em 821, `4`, `1000006`, `100000E`, `8`): significado desconhecido.

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

### Animação (2026-09-19)

- **Nós por nome:** os registros do `.ANM` casam com os nós do `.MOD` pelo nome (`Bip01 R Calf`, `Object09`…). Nós sem trilha seguem o pai; um monstro típico tem 16 a 24 dos 42 nós animados.
- **Quaternion:** a matriz de rotação é a **transposta** da do `D3DXMatrixRotationQuaternion` (as trilhas guardam a rotação inversa, convenção do 3ds Max). Medido no ogro: convertendo o quaternion do quadro 0, o erro contra a rotação local do bind é de 0,001 a 0,008 nos ossos; com a matriz do D3DX seria de 0,3 a 1,7.
- **Amostragem:** interpolação linear das posições e `slerp` (caminho mais curto) das rotações entre chaves, mantendo a primeira/última fora do intervalo; o movimento repete (`MotionFile::frame_at`). Um monstro tem 60 quadros a 24 por segundo no idle (2,5 s) e 20 a 45 nos outros.
- **Slots do `.chr`:** cada slot é um tipo de ação, de 1 em diante, então o slot é `tipo − 1`. Constantes do cliente original (`GameDefine.h`): monstros `STAND1 = 1, STAND2 = 2, MOVE1 = 3, MOVE2 = 4, ATTACK1..4 = 5..8, DEFENSE1 = 9, OFENSEFAIL = 10..11, DEFENSEFAIL = 12..14, DOWN = 15` (os nomes `m00160_01`, `_03`, `_04`, `_05` casam); personagens `VILLAGESTAND = 1, DUNGEONSTAND = 2, WARSTAND = 3, STAND1 = 4, STAND2 = 5, VILLAGEWALK = 6, WALK = 7, RUN = 8, RUNSTOP = 9, ATTACK1..2 = 10..11, CASTINGSKILL = 12…`; NPCs têm um único movimento. Slots sem arquivo apontam para `blank_player_ani.anm` (5 quadros, nenhuma trilha).
- **Consistência (`corum-assets pose-check <mod> <anm>`):** com as trilhas na ordem certa, o comprimento dos ossos varia entre 0,4 e 4,6 unidades ao longo de três movimentos do ogro (antes da correção da ordem chegava a 84), e a maior variação é do `Footsteps`, que se move de verdade. O primeiro quadro de um idle está a 90–170 unidades do bind (o bind é uma pose aberta): isso não é erro.

Ainda não decifrado: o significado das chaves `track_36`; o que `x` (segundo campo do cabeçalho de grupo) e o campo de 5 bytes por vértice guardam além de "contagem e primeiro registro"; e o que faz algumas peças rígidas de armas (a maça do ogro) ficarem soltas: suspeita-se que a matriz de mundo guardada nas malhas não seja a do bind para todas **[hipótese]**.

## Tabelas de jogo `.cdb` (`Data\Manager`, módulo `cdb`, 2026-09-19)

- **Cifra [confirmado]:** `u32 tamanho` + `tamanho` bytes; o byte `i` é XOR com `(DECODE_KEY[i % 21] + 2) & 0xFF`. A chave `DECODE_KEY` está em `CorumOnlineProject/GameControl.h` em GBK e o parser a traz em bytes crus (`cdb::DECODE_KEY`). A rotina do cliente é `DecodeCDBData` (`GameControl.cpp`); o `CMessagePool::DecodeData4Text` (`MessagePool.cpp`) faz o mesmo para os textos.
- **Depois de decifrado** o corpo é uma tabela de registros de tamanho fixo (`#pragma pack(1)`) **ou** um pool de textos.
- **Como cada layout foi confirmado:** os tamanhos das structs do código-fonte (`struct.h`) dividem exatamente o corpo decifrado, e os primeiros registros fazem sentido. O teste `typed_tables_match_the_real_client_files` (rode com `CORUM_DATA` apontando para `Data`) confere as contagens abaixo.

| Arquivo | Registro | Tamanho | Registros | Observação |
|---|---|---|---|---|
| `Level.cdb` | `LevelExp` | 9 | 200 | `u8 nível` + `u64 exp`; o `SLEVEL_EXP` do código-fonte (5 B) **não** bate com este arquivo |
| `GuardianLevel.cdb`, `GuardianExp.cdb` | `GuardianLevelExp` | 5 | 200 | `u8` + `u32`, igual ao código-fonte (idênticos entre si) |
| `ItemResource.cdb` | `ItemResource` | 89 | 3.058 | id → ícone (`weapon_icon01.tga`), modelo (`w0001`), tipo/animação |
| `SkillResource.cdb` | `SkillResource` | 50 | 110 | id → ícone (`skill_icon1.tga`) |
| `ItemOption.cdb` | `ItemOption` | 263 | 1.117 | até 4 linhas de texto por item |
| `ItemStore.cdb` | `ItemStore` | 5 | 1.660 | item, tipo e mapa da loja |
| `npctable.cdb` | `NpcTable` | 808 | 112 | id, nome, tipo, 3 falas de 256 B |
| `CPTable.cdb` | `CpTable` | 249 | 31 | habilidades "CP": nomes, animações, sons, 5 pares (id, valor) |
| `Help.cdb`, `HelpInfo.cdb` | `HelpInfo` | 73 | 732 | dica de ajuda + posição na tela |
| `DungeonProductionItemMinMax.cdb` | `DungeonProductionItemRange` | 7 | 30 | faixa de ids de item por tipo de dungeon |
| `BaseClassInfo.cdb` | `BaseClassInfo` | 20 | 6 | 5 × `i32` (aura, divino, invocação, chakra, magia), todos 100 |
| `message.cdb`, `Cmd_Message.cdb`, `Emoticon.cdb`, `Filter_*Conv_Message.cdb` | `TextPool` | — | 1.823 / 17 / 40 / 22 | assinatura `Oops`; `u32 (ignorado)`, `Oops`, `contagem`, `tamanho_dos_textos`, `contagem × (id, posição)`, textos NUL-terminados; o cliente indexa pela posição |

- **Idioma [confirmado]:** este cliente traz os textos em **inglês** (`message.cdb`: "Bank", "Occupied Dungeon"…); nomes de NPC como "Takion". Campos fixos são bytes crus (`FixedText`), com decodificação GBK só quando aparecer texto não ASCII.
- **Todas as tabelas com estrutura conhecida** já têm esquema (ver a seção do "System" editável abaixo); só fica sem decifrar o conteúdo de alguns campos `unknown_*`.
- **CLI:** `cdb-info <arquivo.cdb>` mostra o tipo/contagem e os 10 primeiros registros; `cdb-decode-all <Data\Manager> <saida>` grava os 55 corpos decifrados como `.bin`, para inspecionar as tabelas ainda sem parser.

## O "System" editável: esquemas e TSV (módulos `schema` e `tables`, 2026-09-19)

Como a pasta `System` do Lineage 2 (`itemname`, `weapongrp`, `armorgrp`, `skillgrp`…): cada tabela `.cdb` decifrada vira um **TSV com cabeçalho e uma coluna por campo**, editável em planilha, e volta a `.cdb` idêntico.

- **Esquema (`schema::Schema`):** lista de colunas nomeadas e tipadas (`u8/u16/u32/u64/i16/i32`, texto de tamanho fixo, bytes em hexadecimal para o que ainda não foi decifrado). Grupos repetidos viram `set_option1_kind`, `level12_max`… A mesma definição serve para ler bytes, escrever TSV e fazer o caminho de volta.
- **Definições (`tables::schema_for`):** 55 arquivos, seguindo `CommonServer/BaseItem.h` (itens), `CorumOnlineProject/Effect.h` (`BASESKILL`), `struct.h` e `LoginAgent/ItemManager.h`. Cada tamanho de registro foi conferido contra o arquivo do cliente instalado.
- **Ida e volta [confirmado]:** o teste `every_real_table_round_trips_through_tsv` (com `CORUM_DATA`) faz `.cdb` → TSV → `.cdb` nas 55 tabelas e exige bytes **idênticos** ao original.
- **Texto:** bytes crus (o cliente é GBK); no TSV cada byte vira um caractere Latin-1, o que é idêntico ao ASCII e sem perdas para o resto. `\t`, `\n`, `\r` e `\` são escapados. Um texto maior que o campo é rejeitado.
- **Regras de edição:** não mudar nome nem ordem das colunas; o número de linhas pode variar. Os ids não são validados (um id repetido ou fora de faixa passa).

| Tabela (arquivo) | Linhas | Colunas | Conteúdo |
|---|---|---|---|
| `ItemWeapon` | 554 | 63 | arma: tipo, mão, grau, nível mínimo, dano, velocidade, alcance, 6 opções de set, 4 opções de parte, preço |
| `ItemArmor` | 1.246 | 57 | armadura (mesma estrutura, sem mão/mana/destreza) |
| `ItemSpecial`, `ItemConsumable`, `ItemSupplies`, `ItemZodiac`, `ItemRide`, `ItemGuardian`, `ItemMagicArray`, `ItemMaterials`, `ItemMixUpgrade`, `ItemMagicFieldArray`, `ItemUpgrade`, `ItemLiquid`, `ItemEdition`, `ItemBag` | 219 / 167 / 124 / 35 / 1 / 66 / 23 / 211 / 16 / 26 / 22 / 14 / 173 / 15 | 14–39 | um tipo de item cada; todos começam com `id`, `name_kor`, `name_eng`, `code_id`, `code_type`, `rand_item`, `movable` |
| `ItemSetInfo` | 92 | 47 | sets de equipamento e bônus |
| `ItemAttrDefine`, `ItemAttrValueList` | 321 / 476 | 7 / 5 | descrição e faixas de valor dos atributos de item |
| `Skill` (= `SkillEffect`) | 117 | 346 | habilidades: nome, descrição, alvo, alcance, tempos e 51 níveis (`levelN_min/max/mana/compass/duration/probability`) |
| `SkillResource`, `itemresource`, `itemstore`, `itemoption` | 110 / 3.058 / 1.660 / 1.117 | 9 / 9 / 3 / 10 | ícones, modelos, lojas e textos de opção |
| `npctable`, `CPTable`, `Level`, `GuardianLevel`, `BaseClassInfo`, `Help`, `KeyInfo`, `GroupInfo`, `ItemMaking`, `Itemtalisman`… | — | — | NPCs, CP, níveis, ajuda, teclas, grupos, receitas |
| `questtitle`, `questlist`, `questnpc` | 38 / 218 / 334 | 2 / 13 / 3 | missões: títulos, texto, dica, alvos (`target1..5`) e NPCs |
| `Hairshopoption` | 192 | 9 | estilos de cabelo da loja (`model_id` = id em `DefResource.erd`, `1001` = `ph101001.MOD`) |
| `InterfaceResourceInfo`, `InterfaceSpriteManager`, `InterfaceComponentInfo`, `InterfaceFrameInfo` | 496 / 506 / 1.657 / 84 | 7 / 2 / 14 / 3 | interface: arquivos e recortes de imagem, componentes com posição/escala, janelas |
| `EventDungeonDescription` | 2 | 2 | textos longos de eventos de dungeon |

- **Diferenças entre o código-fonte (2005) e o cliente (2007) [hipótese quanto ao significado]:** `ItemConsumable` tem 3 bytes a mais que `BASEITEM_CONSUMABLE` (um `u16` entre `min_lev` e `max_lev`, sempre 0, e um `u8` final de 0 ou 1: colunas `unknown_min_lev_2` e `unknown_tail`); `ItemBag` tem 4 bytes a mais no fim (`unknown_tail`, sempre 0); `ItemAttrDefine` tem 111 B (texto de 100 B + `unknown_flag`); `Level.cdb` tem 9 B por nível (o `SLEVEL_EXP` do código-fonte, 5 B, vale só para `GuardianLevel`). `Itemtalisman` não tem struct no código-fonte: só o cabeçalho de item foi separado e os 162 B seguintes ficam em `unknown_body` (hex).
- **Conferência de sentido:** "Marbes' Death Fist" pede nível 192 e dá dano 59–110; "Vaselin's Cross Shower" custa 218.412 (venda 70.353); "Mana Mastery" é passiva (`type=3`) da propriedade 500, como comentam as structs. `ItemWeapon` 327 de 554 armas não pedem nível.
- **Como usar:** `corum-assets cdb-export-tsv <Data/Manager> <pasta>` gera os TSV; depois de editar, `corum-assets tsv-to-cdb <Tabela> <arquivo.tsv> <saida.cdb>` gera o `.cdb` cifrado (verificado: trocar "Short Sword" por "Espada Curta" e reexportar altera só essa linha; o arquivo mantém o tamanho, 110.250 B). Isso permite um mod do cliente original **e** um cliente Rust que leia os TSV direto.
- **Limite:** o servidor é quem manda nas regras (dano, preço, drop); editar o cliente só muda o que aparece.
- **Pools de texto e `.cdt`:** `message`, `Cmd_Message`, `Emoticon` e os dois filtros saem como `id<TAB>texto` (com `#prefix=` na primeira linha, os 4 bytes que o cliente ignora); os 213 `.cdt` no formato `ChrInfo` saem juntos em `Cdt.tsv` (ver a seção `.cdt` abaixo). Ficam de fora seis `.cdt` de outro layout (`m00011`, `pm01001`–`pm05001`), que o cliente nunca lê.
- **Bytes velhos em textos:** alguns campos (`questlist`, `EventDungeonDescription`, `ItemAttrDefine`) guardam sobras depois do NUL, porque o editor original reaproveitava o campo. O TSV mostra essas sobras como `\0` para a ida e volta ser exata; ao editar, dá para apagá-las.

## Tabelas `.cdt` (`Data\Cdt`, módulo `cdt`, 2026-09-19)

Quadros de efeito e sons de cada movimento de um personagem, monstro ou efeito. **Sem cifra.**

- **Formato [confirmado]:** lido por `InitChrInfo` (`GameControl.cpp`) com a struct `ChrInfo` (`ChrInfo.h`): `u32 animações`, `u32 movimentos`, depois `animações × movimentos` registros de **92 bytes**: `u8 quadro_de_efeito[10]`, 2 bytes de padding do compilador (sempre 0) e `10 × (u32 quadro, u32 som)`. O cliente indexa `[animação × movimentos + movimento − 1]` (`ChrInfoLayer::GetFrameInfo`).
- **Dimensões medidas:** 153 arquivos de monstro `1 × 15` (15 movimentos, os mesmos slots do `.chr`), 55 de efeito/NPC `1 × 1`, e 5 de jogador (`pm01000`–`pm05000`) `9 × 50`: 9 tipos de item na mão × 50 movimentos. `GetFrameInfo(classe−1, tipo_de_item, movimento, n)` devolve o `n`-ésimo quadro-chave do movimento.
- **Para que serve:** os quadros de efeito marcam o instante do passo (pés no chão: `m00160` movimento 3 tem 15 e 35), do golpe e do som; o sandbox pode usá-los para sincronizar efeitos e poeira com a animação. Nas amostras, 1.878 quadros são não nulos e nenhum som usa o segundo campo (`sound_id`) — **[hipótese]** os sons vêm de outra tabela.
- **Fora do formato:** `m00011.cdt` (registros de 2 bytes, 1 × 15) e `pm01001`–`pm05001.cdt` (idênticos, 9 × 50 registros de 2 bytes) têm outro layout e nenhum leitor no código do cliente; o parser os recusa e o export os lista como ignorados.
- **TSV [confirmado a ida e volta]:** `corum-assets cdt-export-tsv <Data/Cdt> <Cdt.tsv>` gera **um** TSV com 4.600 linhas (uma por arquivo, animação e movimento; colunas `file`, `animation`, `motion`, `effect_frame1..10`, `padding`, `sound1_frame`, `sound1_id`…). `corum-assets tsv-to-cdt <Cdt.tsv> <pasta>` recria os `.cdt`; o teste `real_cdt_files_round_trip_through_tsv` exige bytes idênticos nos 213 arquivos.

## Tabelas de recursos `.erd` (módulo `erd`, 2026-09-19)

`CorumResource.erd` e `DefResource.erd` (raiz do cliente) ligam **id de recurso → caminho de arquivo**, como os comentários de `CorumOnlineProject/DefResource.h` sugerem (`167782161 // .\Data\Map\worldmap1.cdb`).

- **Layout [confirmado]:** `u32 contagem`, `contagem × (u32 id, u32 tamanho)`, depois os caminhos concatenados (sem terminador). O total fecha exatamente com o arquivo: 1.140 entradas (33.669 B) e 181 entradas (5.939 B).
- **Categoria = byte alto do id [hipótese]:** em `CorumResource.erd`, `0x0A` = UI (709 entradas, `.\Data\UI\*.tga/.tif`), `0x0D` = 176, `0x0C` = 32, `0x0B`/`0x0E`/`0x0F` poucos, `0x64`–`0x6A` = efeitos (`.\Data\Effect\*.chr`), `0x73`, `0xBA`, `0x2F`. Em `DefResource.erd`: `0x01`/`0x02` = modelos de personagem (`ph101001.MOD`…), `0x0A` = mapas (`.map`), `0x0C` = NPC (`NPC4.cdb`…).
- **Cobertura:** 1.103 dos 1.140 caminhos de `CorumResource.erd` existem no cliente instalado (o resto são placeholders `.` e alguns efeitos/águas ausentes). `DefResource.erd` é de 2004 e cita 85 arquivos que este cliente de 2007 não traz (mapas antigos): usar só como pista.
- **CLI:** `corum-assets erd-dump <arquivo.erd>` imprime `0xID<TAB>caminho`.

## Exportação completa para montar o cliente

`tools/export_client_data.py <cliente> <saida>` reúne tudo numa pasta (12.723 arquivos, 1,28 GB em 54 s, **fora do git**: `rust-client/export/` está no `.gitignore`):

| Pasta | Conteúdo |
|---|---|
| `paks/<Pacote>/` | conteúdo dos 13 `.pak` (`.dds` 3.441, `.mod` 2.158, `.anm` 1.819, `.chr` 1.498, `.tif` 683, `.tga` 342, `.vcl` 206, `.lm` 204…) |
| `loose/` | soltos de `Data`: `Map` (202 `.ttb`, 5 `.map`…), `Cdt` (219), `Sound` (1.054 `.wav`, 31 `.mp3`), `Cursor`, `Manager` |
| `manager_decoded/` | os 55 `.cdb` decifrados (`.bin`) |
| `system/` | 55 tabelas em TSV editável, mais `Cdt.tsv` (`ItemWeapon.tsv`, `Skill.tsv`…), ver "System" editável |
| `resources/` | os dois `.erd` como `0xID<TAB>caminho` |
| `config/` | `.erd`, `.ini`, `Paklist.sin` |
| `manifest.json` | por arquivo: tamanho e sha256; contagem por extensão e por pasta |

Não entram: executáveis/DLLs do cliente e a pasta de patch. Rodar de novo sobrescreve; comparar o `manifest.json` de duas execuções mostra o que mudou entre versões do cliente.

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
cargo run -p corum-assets -- pose-check .\extracted\m00160.mod .\extracted\m00160_03.anm
cargo run -p corum-assets -- extract-all "D:\Games\CorumOnline\Data\Map_stm\Map_stm.pak" .\maps

# mapas (aceitam arquivos soltos ou extraídos)
cargo run -p corum-assets -- ttb-info "D:\Games\CorumOnline\Data\Map\1100.ttb"
cargo run -p corum-assets -- map-info "D:\Games\CorumOnline\Data\Map\1100.map"   # inclui luzes
cargo run -p corum-assets -- stm-info "D:\Games\CorumOnline\Data\Map\1100.stm"   # mostra unread_objects
cargo run -p corum-assets -- vcl-info "D:\Games\CorumOnline\Data\Map\1100.vcl" "D:\Games\CorumOnline\Data\Map\1100.stm"
cargo run -p corum-assets -- lm-info  "D:\Games\CorumOnline\Data\Map\1100.lm"  "D:\Games\CorumOnline\Data\Map\1100.stm"

# tabelas de jogo (Manager)
cargo run -p corum-assets -- cdb-info "D:\Games\CorumOnline\Data\Manager\npctable.cdb"
cargo run -p corum-assets -- cdb-decode-all "D:\Games\CorumOnline\Data\Manager" .\target\verification\cdb
cargo run -p corum-assets -- cdb-export-tsv "D:\Games\CorumOnline\Data\Manager" .\export\system
cargo run -p corum-assets -- tsv-to-cdb ItemWeapon .\export\system\ItemWeapon.tsv .\ItemWeapon.cdb
cargo run -p corum-assets -- cdt-export-tsv "D:\Games\CorumOnline\Data\Cdt" .\export\system\Cdt.tsv
cargo run -p corum-assets -- tsv-to-cdt .\export\system\Cdt.tsv .\cdt_editado
cargo run -p corum-assets -- erd-dump "D:\Games\CorumOnline\CorumResource.erd"

# exporta tudo para uma pasta só (paks + soltos + cdb decifrados + erd + manifesto; fora do git)
python tools/export_client_data.py "D:\Games\CorumOnline" export
```

Para conferir todos os mapas de uma vez (extrai os arquivos dos pacotes e roda `stm-info`, `vcl-info`, `lm-info` e `map-info` em cada um):

```powershell
cargo build -p corum-assets
python tools/survey_maps.py "D:\Games\CorumOnline\Data" target/verification/maps
```

O extrator rejeita caminhos absolutos, componentes `..`, nomes sem terminador e registros que ultrapassem o tamanho do arquivo.
