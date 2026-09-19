# Roteiro visual: mapa, luz e sombra, mobs e personagem

Atualizado em 2026-09-19. Este documento complementa o `RUST_CLIENT_PLAN.md` (Marco 3, prova de assets e renderer) e descreve como chegar do mapa básico atual a uma cena com iluminação, mobs e personagem. Os formatos estão em `crates/corum-assets/README.md`; os controles do sandbox, em `crates/corum-viewer/README.md`.

Legenda de certeza: **[confirmado]** medido nos arquivos ou visto na tela; **[hipótese]** plausível, mas ainda sem verificação.

## Como verificar cada etapa

1. Testes unitários e `cargo clippy --workspace --all-targets -- -D warnings` para qualquer parser novo, com dados sintéticos pequenos.
2. Uma checagem contra o arquivo real do cliente, registrada no README do formato (contagens que fecham, como os 22.157 do VCL).
3. Captura de tela com `tools/capture-window.ps1`, guardada em `target/verification/shots/` (fora do Git), comparando "antes" e "depois" no mesmo ângulo.
4. Para "parece com o original": captura do cliente original rodando na VM do Marco 0 (não é possível comparar sem isso; até lá vale a coerência interna).

## Estado atual (mapa `1100`)

| Item | Estado |
|---|---|
| Geometria do cenário STM (20 objetos, 13.934 faces) | **[confirmado]** completa |
| Texturas do cenário (51 únicas, DXT1, `Map_dds.pak`) | **[confirmado]** aplicadas, com mipmaps e alpha-test |
| Colisão TTB, personagem e mob de teste (caixas) | funcionando |
| Outros mapas | 196 mapas passam nos parsers; texturas DDS e TIFF resolvem; verificado na tela em 4 mapas |
| Luz | Iluminação assada completa no `1100`: VCL (tipo 1) e lightmaps (tipo 3), em espaço gamma; luzes pontuais só em personagem e mob (B1–B4 prontos) |
| Céu, névoa, água, objetos posicionados, mobs e personagem reais | ausentes |

---

## Fase A — Fechar o mapa básico

Objetivo: qualquer mapa do cliente abre sem erro e mostra toda a geometria e os objetos posicionados.

| # | Tarefa | Notas | Aceite |
|---|---|---|---|
| A1 ✅ | Rodar os parsers e o sandbox nos demais mapas | Feito com `tools/survey_maps.py` nos 196 mapas empacotados: 0 erros de parse, `.vcl` exato em 195, `.lm` sem divergência em 196, `.map` lido em 196. Achados: o tipo 0 (10.700 objetos) não era desenhado nem contado no VCL; o campo de tipo usa os bytes altos; nomes curtos eram rejeitados. Verificado na tela em `1100`, `5`, `101` e `750`. **Aberto:** o mapa `1` (ver README do `corum-assets`) | Cumprido, com o mapa `1` como exceção documentada |
| A2 ✅ | Parser de STM que não perde objetos em silêncio | Feito: regra estrutural dos contadores no lugar do tamanho do nome, ressincronização e `unread_object_offsets` (mostrado por `stm-info`) | Cumprido: 0 objetos não lidos nos 196 mapas |
| A3 | Normais por vértice do STM | Tipo 1 tem `V × [f32;3]` após 16 bytes **[hipótese: normais]**. Validar renderizando e comparando com a normal da face | Superfícies curvas sem facetas |
| A4 | `GX_OBJECT`: ler `.MOD` (estático) e `.CHR` (animado) posicionados (o `.map` já traz a lista) | O parser de MAP já lê posição, escala, eixo e ângulo. Falta instanciar o modelo. Só funciona para malhas estáticas até a Fase C1 | `RD_BONFIRE.CHR` e `village_*.MOD` aparecem nos mapas que os usam |
| A5 | Céu/fundo e névoa | Descobrir de onde vem a cor de fundo (pode estar em `.cdb`/`.cdt` ou no executável). Fase B5 cobre a névoa | Fundo deixa de ser azul sólido |
| A6 | Água e transparência | Texturas TIFF/DDS de água têm alfa de 90 a 210 (medido) e hoje perdem os pixels abaixo de 50% no *alpha test*. Requer uma passada com blending (ordenada de trás para frente) e possivelmente animação por UV; objetos tipo 0 (`ALP`) e `type_flags` podem indicar quais são translúcidos **[hipótese]** | Água semi-transparente, sem buracos |
| A8 | Billboards (tipo 48) | 153 objetos, 4 vértices cada, nomes ` BILLBOARD*`. Placas que giram para a câmera (efeitos, chamas). Hoje são lidas e não desenhadas | Placa visível e voltada para a câmera |
| A9 | Mapa `1` | 98.055 vértices tipo 0 contra 87.063 cores VCL; 257 lightmaps para 1 objeto tipo 3. Investigar o layout diferente antes de aceitar o mapa | `vcl-info` e `lm-info` sem divergência |
| A7 | Sanidade de alinhamento TTB↔STM | Hoje ambos usam origem 0 e escala `1/tile_size`; visualmente o personagem fica no chão da área jogável **[confirmado no 1100]**. Repetir em outros mapas com `G` (grade) ligada | Grade TTB coincide com o piso em 3+ mapas |

## Fase B — Luz e sombra

Contexto: o cliente original usa o pipeline fixo do Direct3D 8. As pistas de que a iluminação é *assada* nos dados:

- **VCL [confirmado]:** `N.vcl` tem exatamente uma cor `AARRGGBB` por vértice dos objetos tipo 1 (22.157 no `1100`).
- **Lightmaps [confirmado o vínculo, não o formato]:** os objetos tipo 3 têm 3 UVs de lightmap por face (598 faces → 1.794 UVs), e existe `N.lm`. O formato do `.lm` ainda não foi decifrado.
- **Luzes [confirmado]:** `GX_LIGHT` traz 20 luzes coloridas com posição e raio.

| # | Tarefa | Notas | Aceite |
|---|---|---|---|
| B1 ✅ | **Parser de VCL e uso como cor de vértice nos objetos tipo 1** | Feito. Ordem confirmada numericamente (diferença de luminância 3,9 nas arestas contra 16,6 em pares aleatórios) e na tela (variação de tom nas muralhas). O shader faz `textura × VCL` sem ganho extra; se, comparado ao cliente original, o resultado ficar escuro, o ganho (`MODULATE2X`?) é o primeiro suspeito **[hipótese]** | Cumprido: contagem `22.157 = 22.157`, `vcl-info` confere |
| B2 ✅ | Parser de `GX_LIGHT` e luzes pontuais no shader | Feito: 20 luzes num buffer de uniformes (até 32), atenuação quadrática até o raio, tecla `L`. Como as poças de luz dos lightmaps/VCL têm cores compatíveis com as dessas luzes (verificação visual), elas parecem já estar assadas no cenário; hoje só iluminam personagem e mob. O inteiro `1000` continua sem significado; a intensidade (`× 1,5`) foi escolhida a olho | Falta comparar com o cliente original para calibrar a intensidade e decidir se o cliente as usa em tempo de execução (**[hipótese]**: usa nos atores) |
| B3 ✅ | **Decifrar `.lm`** | Feito, sem precisar das DLLs: registros `(primeiro campo, largura, altura, texels RGB565)` em sequência, um por objeto tipo 3, na ordem do STM; fecha no byte exato do arquivo (9 registros no `1100`) e o STM repete o cabeçalho de cada um, o que confirma o emparelhamento. Falta entender o primeiro campo (1, 6, 38). `Map_light.pak` traz `.lm` de 0 bytes em alguns mapas (`619`, `604`): significa "sem lightmaps" | Cumprido: `lm-info` mostra os 9 pares batendo |
| B4 ✅ | Aplicar lightmap nos objetos tipo 3 | Feito: 3 UVs por face vindos do STM, segundo `texture_2d_array`, `textura × lightmap`. Junto, o pipeline passou para espaço gamma (como o D3D8) para que a conta seja fiel. Ganho padrão ×1 (`B` alterna ×2): a inferência é que os pontos claros chegam a 252/255 e `0xFFFF`, então ×2 estouraria **[hipótese]** | Cumprido: piso e muralhas com poças de luz coloridas coerentes com o VCL |
| B5 | Névoa e cor ambiente por mapa | Procurar parâmetros por mapa em `Manager/*.cdb` ou constantes no executável | Profundidade visual à distância |
| B6 | Sombra de personagem e mobs: primeiro um "blob" (disco escuro projetado no chão), depois *shadow map* | O cliente original provavelmente usa sombra simples; comece com o blob e só evolua se a comparação com o original pedir | Personagem com sombra que segue o terreno |

Ordem sugerida: ~~B1 → B2 → B3 → B4~~ (feitos) → **B6 (sombra de personagem)** → B5 (névoa/ambiente). Falta comparar com o cliente original (VM do Marco 0) para decidir o ganho ×1/×2 e a intensidade das luzes dos atores.

## Fase C — Mobs e personagem

Inventário **[confirmado]**: `Monster.pak` tem 189 modelos, 192 manifestos `.chr` e 1.086 animações; `Npc.pak` tem 22 modelos; `Character.pak` tem 905 modelos, 617 manifestos e 259 animações.

Estado do formato (2026-09-19): a geometria (posições, UVs, costuras e faces) já é decodificada em 93% das malhas de `Character`, 99% de `Monster` e `Map_chr` e 100% de `Npc` (`tools/survey_models.py`). Falta a pose (hierarquia de nós/ossos) e os pesos de skinning.

| # | Tarefa | Notas | Aceite |
|---|---|---|---|
| C0 ✅ | **Mostrar modelos reais** | Feito, no visualizador e no **sandbox de mapa**: o personagem é um NPC humano texturizado (`Npc/npc007`) e o mob, um diabrete alado (`Monster/m00010`), ambos na pose de bind, com luz e normais suaves, girando para onde andam; escolhidos por `CORUM_PLAYER`/`CORUM_MOB`. Verificado com 4 modelos de 2 pacotes | Cumprido: modelos reais e texturizados no sandbox |
| C1 ✅ (geometria) | **Decodificar costuras e geometria do `.MOD`** | Feito: as "costuras" são `S` UVs extras (`T + S = V`), seguidos de `S` índices de origem, e os grupos de faces usam o cabeçalho do STM (28 bytes) com triângulos `u16` sem preenchimento. 93–100% das malhas decodificam (ver README do `corum-assets`). **Aberto:** ~150 malhas com layout diferente, o restante do payload (normais, registros por grupo) e os **pesos de skinning** | Cumprido para a geometria; pesos em C3/C4 |
| C2 | Semântica das tracks do `.ANM` | **[hipótese]** `track_24` = rotação (quaternion), `track_20` = posição ou escala (3 floats), `track_36` = posição + quaternion; a quinta é morph por vértice. Confirmar animando um osso simples e comparando visualmente | Animação de idle reconhecível |
| C3 | Esqueleto: hierarquia `F5` (ossos/nós) e pose de bind | **Correção:** a pose de bind já está nas posições cruas dos modelos, então o C3 deixou de ser pré-requisito para mostrar modelos. Continua necessário para animar: a matriz de cada osso vem do `F5`/`pivot`/`parent_index` e das tracks do `.ANM` | Ossos posicionados sobre o modelo |
| C4 | Skinning na GPU (até N ossos por vértice). Os pesos ficam na cauda do payload `F4` (ver README do `corum-assets`): registros `osso, peso, posição local, normal local` nas malhas pequenas, tabela ainda não decifrada nas grandes | Buffer de matrizes por instância; começar com CPU se ajudar a depurar | Mob caminhando com o modelo correto |
| C5 | Parser de `.CDB`/`.CDT` (tabelas de Manager) | Necessário para ligar "monstro N" a `.chr`, tamanho, animação de idle/andar/ataque. O cliente instalado tem 56 arquivos em `Data\Manager` (a maioria `.cdb`) e 219 `.cdt` em `Data\Cdt`. O cliente instalado é de 2007 e algumas tabelas diferem do repositório (ver o plano principal); usar as do cliente | Dado um id de monstro, sabemos modelo e animações |
| C6 | Personagem: montagem por partes (corpo, cabeça, armadura) e troca de equipamento | 905 modelos de `Character`; a convenção de nomes (`pm1245_005.chr`) provavelmente codifica classe/parte **[hipótese]**. Requer C5 para a tabela de itens | Personagem completo no sandbox |
| C7 | Integração com o sandbox: mob com patrulha usando o modelo real e a animação de andar; personagem controlável com animação por movimento | O sandbox já tem colisão TTB e input | Cena jogável local |
| C8 | Ligar ao servidor | Só depois dos Marcos 1 e 2 do plano (protocolo). Posições e spawns passam a vir de pacotes | Marco 3/4 do plano |

Ordem sugerida: **C0 → C1 → C2 → C3 → C4 → C7**, com C5 em paralelo (independente de gráfico) e C6 depois.

## O que dá para preparar em paralelo

Há três trilhas que quase não dependem uma da outra:

| Trilha | Primeira entrega | Bloqueia |
|---|---|---|
| Mapa e luz (A + B) | B1 (VCL) e B2 (luzes) no sandbox | nada |
| Formatos de modelo (C1–C3) | Geometria já decifrada; falta a pose, os pesos e a animação | mobs e personagem animados |
| Dados de jogo (C5) e protocolo (Marcos 1–2 do plano) | Parser de CDB; login por CLI | integração com servidor |

Recomendação: a iluminação assada (B1–B4, A1, A2) e os modelos parados e texturizados (C0, C1) estão prontos. Seguir com **A4** (objetos posicionados nos mapas, com ~99% dos `.MOD` de `Map_chr`), com o **skinning e a animação** (decifrar a cauda do `F4`, as tracks do `.ANM`, C2–C4) e com o **C5** (tabelas CDB: qual modelo é qual monstro/personagem).

## Riscos e decisões pendentes

- **C1 pode exigir engenharia reversa das DLLs** `SS3D*`. Há licença/direitos a confirmar antes de distribuir qualquer conversão (ver o plano principal).
- **Fyrox vs. `wgpu` direto:** o sandbox atual usa `wgpu`. Se a Fase C crescer para animação, UI e áudio, reavaliar Fyrox com o protótipo já existente (mapa + modelo) como critério.
- **Cliente de 2007 vs. código de 2005:** dados como CDB podem diferir; sempre usar os arquivos do cliente instalado como referência visual.
- **Sem cliente original rodando** não há "resposta certa" para a aparência; o Marco 0 do plano (VM isolada) destrava a comparação lado a lado.
