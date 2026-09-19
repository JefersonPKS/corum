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
