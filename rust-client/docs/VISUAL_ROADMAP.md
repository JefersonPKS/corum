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
| Luz | VCL aplicado aos objetos tipo 1 e 20 luzes pontuais do `GX_LIGHT` (B1 e B2 prontos); falta o lightmap dos objetos tipo 3 |
| Céu, névoa, água, objetos posicionados, mobs e personagem reais | ausentes |

---

## Fase A — Fechar o mapa básico

Objetivo: qualquer mapa do cliente abre sem erro e mostra toda a geometria e os objetos posicionados.

| # | Tarefa | Notas | Aceite |
|---|---|---|---|
| A1 | Rodar o sandbox nos demais mapas (`1`, `10001`, `10002` soltos; os de `Map_stm.pak`, extraídos com `corum-assets extract`) | Compare o número de objetos lidos com a varredura por marcador `0xFFFFFFFF` (ver "Riscos" do STM). É a forma mais rápida de achar o próximo bug do parser | Lista de mapas com "N objetos lidos == N encontrados" |
| A2 | Tornar o parser de STM estrito: falhar (ou avisar) quando sobrarem objetos que a heurística não aceitou | Hoje o laço para em silêncio | Aviso com offset do primeiro objeto não lido |
| A3 | Normais por vértice do STM | Tipo 1 tem `V × [f32;3]` após 16 bytes **[hipótese: normais]**. Validar renderizando e comparando com a normal da face | Superfícies curvas sem facetas |
| A4 | `GX_OBJECT`: ler `.MOD` (estático) e `.CHR` (animado) posicionados | O parser de MAP já lê posição, escala, eixo e ângulo. Falta instanciar o modelo. Só funciona para malhas estáticas até a Fase C1 | `RD_BONFIRE.CHR` e `village_*.MOD` aparecem nos mapas que os usam |
| A5 | Céu/fundo e névoa | Descobrir de onde vem a cor de fundo (pode estar em `.cdb`/`.cdt` ou no executável). Fase B5 cobre a névoa | Fundo deixa de ser azul sólido |
| A6 | Água e transparência | Materiais `JE_water_map_*` (há texturas `je_water_map_*` em `Map_dds.pak` e `Map_tif.pak`). Requer blending e possivelmente animação por UV | Água semi-transparente |
| A7 | Sanidade de alinhamento TTB↔STM | Hoje ambos usam origem 0 e escala `1/tile_size`; visualmente o personagem fica no chão da área jogável **[confirmado no 1100]**. Repetir em outros mapas com `G` (grade) ligada | Grade TTB coincide com o piso em 3+ mapas |

## Fase B — Luz e sombra

Contexto: o cliente original usa o pipeline fixo do Direct3D 8. As pistas de que a iluminação é *assada* nos dados:

- **VCL [confirmado]:** `N.vcl` tem exatamente uma cor `AARRGGBB` por vértice dos objetos tipo 1 (22.157 no `1100`).
- **Lightmaps [confirmado o vínculo, não o formato]:** os objetos tipo 3 têm 3 UVs de lightmap por face (598 faces → 1.794 UVs), e existe `N.lm`. O formato do `.lm` ainda não foi decifrado.
- **Luzes [confirmado]:** `GX_LIGHT` traz 20 luzes coloridas com posição e raio.

| # | Tarefa | Notas | Aceite |
|---|---|---|---|
| B1 ✅ | **Parser de VCL e uso como cor de vértice nos objetos tipo 1** | Feito. Ordem confirmada numericamente (diferença de luminância 3,9 nas arestas contra 16,6 em pares aleatórios) e na tela (variação de tom nas muralhas). O shader faz `textura × VCL` sem ganho extra; se, comparado ao cliente original, o resultado ficar escuro, o ganho (`MODULATE2X`?) é o primeiro suspeito **[hipótese]** | Cumprido: contagem `22.157 = 22.157`, `vcl-info` confere |
| B2 ✅ (parcial) | Parser de `GX_LIGHT` e luzes pontuais no shader | Feito: 20 luzes num buffer de uniformes (até 32), atenuação quadrática até o raio, tecla `L`. Aplicadas só ao que não tem cor pré-calculada (personagem, mob e objetos tipo 3), porque num objeto com VCL a luz já foi somada na geração e seria contada duas vezes **[hipótese: o cliente original talvez nem as use em tempo de execução]**. O efeito no `1100` é sutil (tom frio no personagem e no chão). O inteiro `1000` continua sem significado; a intensidade (`× 1,5`) foi escolhida a olho | Falta comparar com o cliente original para calibrar intensidade e decidir se valem em tempo de execução |
| B3 | **Decifrar `.lm`** | Cabeçalho `1, 32, 32` **[confirmado]**; o corpo não é uma imagem 32×32 simples. Experimentos: tratar `32×32` como tamanho de cada lightmap e dividir o corpo por `598` faces; testar formatos RGB565, RGB888 e L8; ver se `Map_light.pak` traz `.lm` vazios (`619`, `604`) **[confirmado]**, o que sugere que nem todo mapa usa lightmap. Se travar, desmontar o carregador nas DLLs `SS3D*` (esta configuração tem ferramentas IDA via MCP) | Imagem de lightmap legível exportada para PNG |
| B4 | Aplicar lightmap nos objetos tipo 3 | Segundo conjunto de UV (as 3 coordenadas por face, portanto vértices não compartilhados entre faces); multiplicar por 2 costuma ser o padrão de `MODULATE2X` **[hipótese]** | Piso principal (`do_maintile-*`) com sombreado suave |
| B5 | Névoa e cor ambiente por mapa | Procurar parâmetros por mapa em `Manager/*.cdb` ou constantes no executável | Profundidade visual à distância |
| B6 | Sombra de personagem e mobs: primeiro um "blob" (disco escuro projetado no chão), depois *shadow map* | O cliente original provavelmente usa sombra simples; comece com o blob e só evolua se a comparação com o original pedir | Personagem com sombra que segue o terreno |

Ordem sugerida: ~~B1 → B2~~ (feitos) → **B3/B4 (lightmap, o próximo)** → B6 → B5. O chão principal (`JE_T_land_L_*`) e o piso da arena (`do_maintile-*`) são objetos tipo 3: continuam com iluminação genérica até o `.lm` ser decifrado.

## Fase C — Mobs e personagem

Inventário **[confirmado]**: `Monster.pak` tem 189 modelos, 192 manifestos `.chr` e 1.086 animações; `Npc.pak` tem 22 modelos; `Character.pak` tem 905 modelos, 617 manifestos e 259 animações.

Estado do formato: 53 de 1.197 malhas de `Character` são estáticas e já exportáveis. As demais precisam de remapeamento de costuras (vértices duplicados em UV/normal) e pesos de skinning.

| # | Tarefa | Notas | Aceite |
|---|---|---|---|
| C0 | **Mostrar malhas estáticas agora** | O `corum-viewer` já abre `.MOD` estáticos. Levar isso ao sandbox: NPCs/objetos estáticos no lugar das caixas, com textura (`.dds` do próprio pacote) | Um modelo estático real no lugar do mob de teste |
| C1 | **Decodificar costuras e pesos de skinning do `.MOD`** | É o maior risco gráfico. Pistas: F4 tem contagens separadas de vértices, vértices de textura e costuras; o resto do payload deve conter o remapeamento e os pesos. Estratégia: (1) achar modelos pequenos com poucos ossos e vértices; (2) verificar hipóteses de layout contra `sizeof` do registro (o total tem que fechar sem sobras); (3) se travar, desmontar o carregador de modelos nas DLLs `SS3D*` do cliente | Os 1.197 malhas de `Character` viram triângulos com posição e UV corretos |
| C2 | Semântica das tracks do `.ANM` | **[hipótese]** `track_24` = rotação (quaternion), `track_20` = posição ou escala (3 floats), `track_36` = posição + quaternion; a quinta é morph por vértice. Confirmar animando um osso simples e comparando visualmente | Animação de idle reconhecível |
| C3 | Esqueleto: hierarquia `F5` (ossos/nós) e pose de bind | Depende de C1 e C2. A matriz de pose vem do pivot e do pai de cada nó | Modelo na pose de bind sem deformação |
| C4 | Skinning na GPU (até N ossos por vértice) | Buffer de matrizes por instância; começar com CPU se ajudar a depurar | Mob caminhando com o modelo correto |
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
| Formatos de modelo (C1–C3) | Decodificação de costuras/pesos em um modelo pequeno | mobs e personagem animados |
| Dados de jogo (C5) e protocolo (Marcos 1–2 do plano) | Parser de CDB; login por CLI | integração com servidor |

Recomendação: B1 e B2 já estão feitos. Seguir com **B3** (decifrar o `.lm`) e **C0** (modelo estático no sandbox), e deixar **C1** como investigação contínua, já que ela decide o cronograma dos mobs e do personagem.

## Riscos e decisões pendentes

- **C1 pode exigir engenharia reversa das DLLs** `SS3D*`. Há licença/direitos a confirmar antes de distribuir qualquer conversão (ver o plano principal).
- **Fyrox vs. `wgpu` direto:** o sandbox atual usa `wgpu`. Se a Fase C crescer para animação, UI e áudio, reavaliar Fyrox com o protótipo já existente (mapa + modelo) como critério.
- **Cliente de 2007 vs. código de 2005:** dados como CDB podem diferir; sempre usar os arquivos do cliente instalado como referência visual.
- **Sem cliente original rodando** não há "resposta certa" para a aparência; o Marco 0 do plano (VM isolada) destrava a comparação lado a lado.
