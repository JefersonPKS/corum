# Corum Viewer

Ferramentas 3D nativas em Rust para validar os assets do cliente original do
Corum Online. Elas usam `winit` para a janela, `wgpu` para a GPU e os parsers do
crate `corum-assets`.

## Executar

Na pasta `rust-client`:

```powershell
cargo run -p corum-viewer --bin corum-viewer -- "C:\caminho\para\modelo.mod"
```

Sem argumento, o visualizador tenta abrir o modelo de verificação:

```powershell
cargo run -p corum-viewer --bin corum-viewer
```

Também é possível arrastar um arquivo `.MOD` sobre `corum-viewer.exe` depois
de compilar o projeto.

## Sandbox de mapa

A sandbox abre a grade de colisão de um `.TTB` real, coloca um personagem de
teste controlável e um mob em patrulha. Por padrão, ela usa o mapa `1100` e,
quando disponível, combina `1100.ttb` com a geometria de `1100.stm`:

```powershell
cargo run -p corum-viewer --bin corum-sandbox
```

Para outro mapa:

```powershell
cargo run -p corum-viewer --bin corum-sandbox -- "D:\Games\CorumOnline\Data\Map\1100.ttb"
```

O segundo argumento aceita explicitamente o cenário estático:

```powershell
cargo run -p corum-viewer --bin corum-sandbox -- `
  "D:\Games\CorumOnline\Data\Map\1100.ttb" `
  "target\verification\map-scene\1100.stm"
```

Controles da sandbox:

- `WASD` ou setas: movimentar o personagem;
- `Shift`: correr;
- `Tab` / `Shift+Tab`: inspecionar a próxima peça STM ou a anterior;
- `0`: voltar a exibir todas as peças do cenário;
- `F`: focar a câmera na peça selecionada;
- `L`: ligar ou desligar as luzes pontuais do `GX_LIGHT` (só afetam personagem e mob);
- `B`: alternar o ganho da iluminação assada entre ×1 e ×2 (para comparar);
- `G`: exibir ou ocultar a grade de colisão TTB;
- `H`: exibir ou ocultar personagem e mob de teste;
- botão esquerdo + arrastar: orbitar a câmera;
- roda do mouse: zoom;
- `R`: restaurar a câmera;
- `Esc`: fechar.

As células verdes são navegáveis, as vermelhas são bloqueios reais do `.TTB`
e as azuis/laranjas representam atributos especiais. O movimento do personagem
e a patrulha do mob consultam essa grade para não atravessar bloqueios.

## Controles

- botão esquerdo + arrastar: orbitar a câmera;
- roda do mouse: aproximar ou afastar;
- `R`: restaurar a câmera;
- `Esc`: fechar.

## Estado atual

O visualizador já abre uma janela real, envia vértices à GPU, calcula normais,
aplica iluminação simples e renderiza as malhas estáticas cujo layout já foi
decodificado.

O script textual `.MAP`, a grade `.TTB` e os objetos visuais tipos 1 e 3 do
`.STM` já são interpretados. A sandbox renderiza posições, UVs, grupos de
material e 13.934 faces distribuídas por 20 peças do cenário `1100`. O modo de
inspeção mostra uma peça por vez e informa nome, tipo e total de triângulos na
barra de título.

As texturas são lidas de `Data\Map_dds\Map_dds.pak` (DXT1/3/5) e, quando o material
não está lá, de `Data\Map_tif\Map_tif.pak` (TIFF sem compressão). São enviadas à GPU
como um `texture_2d_array` com mipmaps, filtragem anisotrópica e *alpha test*
(recorte duro abaixo de 50% de alfa). No mapa `1100`, as 51 texturas únicas dos 53
materiais são resolvidas. Se nenhuma textura for encontrada, o material volta a ter
uma cor estável por hash. **Limite conhecido:** a água e outras texturas
translúcidas (alfa entre 90 e 210) perdem os pixels abaixo de 50% em vez de
misturar com o fundo, porque ainda não há blending.

Se o segundo argumento for omitido, a sandbox procura o `.stm` ao lado do `.ttb`.

### Outros mapas

Os arquivos da maioria dos mapas estão dentro de pacotes (`Map_stm.pak`, `Map_light.pak`). Extraia o mapa desejado (ou todos, com `tools/survey_maps.py`, que já deixa `N.ttb`, `N.stm`, `N.map`, `N.vcl` e `N.lm` numa pasta) e aponte a sandbox para o `.ttb`:

```powershell
cargo run -p corum-viewer --bin corum-sandbox -- .\target\verification\maps\101.ttb
```

Texturas e iluminação são procuradas, nesta ordem: na pasta de dados do jogo que contém o `.ttb` (`<Data>\Map\N.ttb`), na pasta indicada por `CORUM_DATA` e em `D:\Games\CorumOnline\Data`. Assim mapas extraídos para outro lugar ainda acham `Map_dds.pak`, `Map_tif.pak` e `Map_light.pak`. `.vcl` e `.lm` são lidos ao lado do `.ttb` ou, se não houver, dos pacotes `Map_light.pak` e `Map_tif.pak`.

Verificados na tela: `1100` (masmorra escura com poças de luz coloridas), `5` (vila, com sombras assadas no chão), `101` (masmorra com 141 luzes) e `750` (templo flutuante). Nos três últimos todos os materiais receberam textura.

### Transparência, fogo e água

Cada vértice carrega uma **classe de mistura**, e há três pipelines que desenham os mesmos buffers, cada um mantendo só a sua classe: **opaco** (com recorte abaixo de 50% de alfa), **mistura alfa** (`src_alpha, 1 - src_alpha`, sem escrever profundidade) e **aditivo** (`src_alpha, 1`, sem escrever profundidade e sem iluminação). A ordem é opaco, depois alfa, depois aditivo, e passadas sem nada são puladas.

A classe vem de:

1. **aditivo:** o material tem a flag `0x4` (só modelos), ou a textura é um efeito sobre preto (opaca e com mais de 70% dos texels quase pretos);
2. **mistura alfa:** a textura tem mais de 60% dos texels com alfa parcial (água, mar, vidro);
3. **opaco:** o resto.

Os números e as evidências estão em "Flags de material e mistura" no README do `corum-assets`. Vale para o cenário (STM), os objetos do mapa e os atores. Verificado na tela: fogo em tochas e braseiros do `604` (chamas brilhantes, sem o quadrado preto) e água e jatos translúcidos no `201`.

**Limites:** a mistura alfa **não é ordenada** (superfícies translúcidas sobrepostas podem aparecer na ordem errada), a água **não rola** (a flag `0x10000000` não foi implementada) e os planos de chama ficam todos visíveis ao mesmo tempo, porque a animação que os alterna ainda não roda. O STM não tem flags de material decifradas, então lá só vale a regra da textura.

Para apontar a câmera para um ponto (por exemplo uma fogueira), use `CORUM_PLAYER_AT=x,z` (coordenadas do script do mapa) junto com `CORUM_CAMERA`.

### Objetos do mapa (`GX_OBJECT`)

Os objetos listados no `.map` (árvores, casas, cercas, barris, ruínas, fogueiras) são
desenhados a partir de `Map_chr.pak`: os `.MOD` direto e os `.CHR` pelo modelo que o
manifesto aponta (os `.CHR` animam, ver "Animação"). Cada modelo único vira um lote com as instâncias
já em coordenadas de mundo (escala, rotação Y e posição do script), com as suas texturas
(`Map_chr`, depois `Map_dds` e `Map_tif`). Como os atores, recebem o ambiente e as luzes
pontuais, e não têm cor assada. A tecla `O` mostra ou oculta os objetos. O sandbox imprime
`placed N of M map objects`.

Verificados na tela: `5` (vila: 43 objetos, casas, pinheiros e arbustos), `604` (savana com
ilhas: 713 objetos em 10 modelos) e `750` (2 objetos). Fogo e água: ver a seção acima.

### Animação

O personagem, o mob e os objetos `.CHR` do mapa **animam**. Cada ator carrega o `.chr` do modelo e os `.anm` que ele lista, e a cada quadro o sandbox calcula a pose do esqueleto e reposiciona os vértices na CPU: vértices com pele por `soma(peso × inversa_do_bind × mundo_animado)`, peças rígidas pelo próprio nó. Detalhes do formato em "Nós, esqueleto e pele" e "Animação" no README do `corum-assets`.

- **Slots por pacote:** parado/andando são `Monster` 0/2 (`STAND1`/`MOVE1`), `Character` 0/6 (`STAND1`... `WALK`) e `Npc` 0/nenhum (um movimento só). O personagem anda com o movimento de andar enquanto você o move e volta ao parado ao soltar; o mob patrulha sempre com o de andar. `CORUM_PLAYER_ANIM=idle,andar` e `CORUM_MOB_ANIM=idle,andar` escolhem os slots (0-based). Ex.: `CORUM_PLAYER=Monster/m00160.mod` com `CORUM_PLAYER_ANIM=2,2` mostra a caminhada do ogro.
- **Item na mão (`CORUM_ITEM`, 2026-09-19):** `CORUM_ITEM=<id>` procura o item nas tabelas do jogo (`Data\Manager`), acha o modelo pelo `ItemResource` (`w0001` → `w0001_000.mod`) no pacote `Item` (ou `Character`) e o prende a um osso do personagem, seguindo a animação dele, como o `ItemAttach` do cliente original. `CORUM_ITEM_BONE` troca o osso (padrão `Bip01 R Hand`; também `Bip01 L Hand` e `Bip01 Head`) e `CORUM_ITEM_MODEL` escolhe o índice do modelo (padrão 0). Sem `CORUM_PLAYER`, o personagem é o guerreiro base (veja o próximo item); com ele: `CORUM_PLAYER=Character/pm01000.mod CORUM_ITEM=1` mostra a "Short Sword" na mão direita (e `CORUM_ITEM=812` a garra `.chr`, só na pose de bind). O modelo da arma está longe da origem; o sandbox prende o **pivô do nó** ao osso (ver a seção de itens do README do `corum-assets`). Itens sem modelo 3D (poções, materiais...) e ids inexistentes só geram uma mensagem no terminal.
- **Personagem vestido (`CORUM_CLASS`, `CORUM_ARMOR`, `CORUM_HEAD`, `CORUM_HELMET`, `CORUM_ITEM`, `CORUM_SHIELD`, 2026-09-19):** monta o personagem como o pacote de aparência do cliente original (`DungeonProcess.cpp`, `pAppear`). Basta definir uma delas (e não `CORUM_PLAYER`):
  - `CORUM_CLASS` 1 guerreiro, 2 sacerdote, 3 invocador, 4 caçadora, 5 maga (padrão 1).
  - **Corpo:** sem `CORUM_ARMOR`, o corpo base da classe (`pm01000`…`pm05000`, `RESTYPE_BASE_BODY`); com ele, o **modelo da armadura é o corpo inteiro** (`ItemDataName(armor, classe − 1)`: `CORUM_ARMOR=2200` "Mail" na classe 1 = `pm1100_000`). Não há peças de armadura soltas: a armadura troca o corpo.
  - `CORUM_HEAD=<id>` (1001…): cabeça masculina para as classes 1–3 (`ph101001.mod`) e feminina para 4–5 (`ph201001.mod`), presa a `Bip01 Head`.
  - `CORUM_HELMET=<id do item>` (tipo 10, ex.: 2000 "Cap") na cabeça; `CORUM_ITEM=<id>` (arma) em `Bip01 R Hand`; `CORUM_SHIELD=<id>` (tipo 12, ex.: 2400 "Small Shield") em `Bip01 L Hand`. `CORUM_ITEM_BONE` troca o osso da arma; `CORUM_ITEM_MODEL`, `CORUM_HELMET_MODEL` e `CORUM_SHIELD_MODEL` escolhem o índice do modelo (padrão 0).
  - Cada peça segue a animação do osso; o modelo tem o pivô do nó fixado ao osso (ver a seção de itens do README do `corum-assets`). Itens sem modelo 3D e ids inexistentes só geram uma mensagem no terminal.
  - Exemplo: `CORUM_CLASS=1 CORUM_ARMOR=2200 CORUM_HEAD=1001 CORUM_HELMET=2000 CORUM_ITEM=1 CORUM_SHIELD=2400` mostra o guerreiro completo; `CORUM_CLASS=4 CORUM_HEAD=1003`, uma caçadora.
- **Objetos `.CHR` do mapa** (fogueiras, tochas, bandeiras) tocam o primeiro movimento real em repetição. Só lotes com até 60.000 vértices (todas as instâncias juntas) animam, porque são reposicionados e reenviados à GPU a cada quadro; lotes maiores ficam na pose de bind.
- **Verificado na tela:** o NPC humano `npc007` (mãos e braços mudam durante os 7,5 s do idle), o diabrete alado `m00010`, o ogro de armadura `m00160` (perfil de caminhada) e o fogo do `604` (a chama sobe e a fumaça se desloca).

**Limites:** não há mistura entre movimentos (a troca parado/andar é seca), a velocidade do movimento não acompanha a do personagem, alguns fragmentos rígidos de armas ficam soltos (a maça do ogro), a pele é feita na CPU (a GPU só recebe os vértices prontos) e nenhum movimento de ataque, dano ou morte é disparado ainda.

### Personagem e mob reais

O personagem controlável e o mob em patrulha são modelos `.MOD` reais e texturizados. Eles
começam na **pose de bind** (as posições cruas dos modelos já são o bind) e animam (ver
"Animação"). Cada modelo pega as texturas do seu próprio pacote
(`.dds`, às vezes `.tif`), tem normais suaves, recebe o ambiente e as luzes pontuais do mapa e
gira para a direção em que anda. Padrões: `Npc/npc007.mod` (humano) e `Monster/m00010.mod`
(diabrete alado). Para trocar, use `Pacote/arquivo.mod` (pacotes `Npc`, `Monster`, `Character`,
`Map_chr`) ou `none` para voltar às caixas:

```powershell
$env:CORUM_PLAYER = 'Npc/npc012.mod'
$env:CORUM_MOB = 'Monster/m00760.mod'
cargo run -p corum-viewer --bin corum-sandbox
```

`H` mostra ou oculta os dois. As unidades são as do mapa (1 tile = 125 unidades; o NPC tem 175, ou
1,4 tile), e a frente dos modelos é `+Z` (`ACTOR_YAW_OFFSET` no código corrige um modelo que
olhe para o lado errado). Verificados na tela: `npc007`, `npc012`, `m00010` e `m00760`;
`Character/bw0001_000.mod` e `Monster/m00150.mod` carregam sem erro.

### Iluminação

- **Cor por vértice (`.vcl`)**: os objetos tipo 1 recebem `textura × cor do VCL`, sem luz adicional, porque a cor já contém a iluminação pré-calculada. É o que dá às muralhas e rochas a variação de tom (verde, rosado, alaranjado). No `1100`, as 22.157 cores casam com os 22.157 vértices; se a contagem não bater, o arquivo é ignorado com um aviso.
- **Lightmaps (`.lm`)**: os objetos tipo 3 (piso da arena, chão, muralha externa) recebem `textura × lightmap`, usando os 3 UVs de lightmap por face do STM. Cada objeto usa o registro de mesma ordem no `.lm`, e o par só é aceito se o cabeçalho que o objeto repete bater com o do registro. Os lightmaps (32×32 até 128×128) sobem à GPU como um segundo array de texturas, ampliados com filtro bilinear e amostrados sem sRGB.
- **Espaço gamma:** o Direct3D 8 original multiplica valores brutos, sem conversão sRGB. Por isso a superfície e as texturas usam formatos sem sRGB e a conta é `textura × cor assada` direto. Cor de fundo, personagem e mob (que foram ajustados em espaço linear) são codificados no shader.
- **Luzes pontuais (`GX_LIGHT`)**: as luzes do `.map` (20 no `1100`, 141 no `101`) vão para um buffer de uniformes (até 256), com atenuação quadrática até o raio. Como as poças de luz dos lightmaps e do VCL têm cores compatíveis com as do `.map` (azul, roxo, verde, laranja; correspondência visual, não medida posição a posição), as luzes parecem ter sido *assadas* na geração dos arquivos; por isso o cenário não as recebe de novo, e elas só iluminam o personagem e o mob de teste. `L` compara com e sem elas.
- **Ganho (`B`)**: com ×1, os pontos claros do VCL e do lightmap chegam a 252/255 e `0xFFFF`, o que indica que ×1 é o valor de projeto (×2 estouraria). É uma inferência: só o cliente original rodando decide.

### Clique para andar (2026-09-19)

- **Controle:** um **clique esquerdo** sem arrasto (o cursor move menos de 5 pixels entre apertar e soltar) manda o personagem andar até o ponto do chão sob o cursor; **arrastar** continua orbitando a câmera. `WASD`/setas cancelam a rota e voltam ao controle manual. `Shift` faz a viagem correr (só a velocidade: 6,0 em vez de 3,3 tiles/s; a animação usada é a de andar). Um marcador amarelo mostra o destino até chegar; o personagem para no fim, volta ao movimento parado e a câmera o acompanha.
- **Mira:** o raio do pixel clicado (matriz inversa da câmera, profundidade estilo Direct3D 0..1) é cortado com o plano do chão (`y` do personagem). Coberto por teste: projetar o ponto do chão de volta devolve o pixel.
- **Rota:** `corum_assets::navigation::find_path` (A* de 8 direções sobre os tiles caminháveis do `.ttb`, sem cortar quinas, com alisamento por linha de visão para o raio do corpo, 0,20 tile). Um clique em tile bloqueado gruda no tile livre mais próximo (até 3 tiles); destino fora do mapa ou isolado dá "no route" no terminal e o personagem fica parado. O log mostra `walk: N waypoint(s) to tile (x, z)`.
- **Diferença do original [hipótese]:** o cliente usa o módulo de busca `FindShortestWay` (em DLL) e anda entre os pontos de curva devolvidos (`A_STAR`, `DungeonProcess.cpp`); o resultado daqui é equivalente em espírito, mas o traçado exato pode variar.
- **Altura do terreno:** os mapas com `HEIGHT_FIELD NA` (a maioria, inclusive `1100`) são planos e o personagem anda em `y = 0`. Só há 5 arquivos `.hfl` (`10001`, `10002`, `property`, `property02`, `worldmap2`), ligados a mapas de mundo; o formato não foi decifrado (ver o README do `corum-assets`).
- **Verificação:** `tools/capture-window.ps1 -Click "0.85,0.2" -ClickWait 0.1` clica numa fração da janela e captura logo depois. Com `CORUM_CLASS=1 CORUM_ARMOR=2200 CORUM_HEAD=1001 CORUM_ITEM=1 CORUM_CAMERA=0.6,0.9,9` o personagem vestido anda até o marcador.

### Verificação visual automatizada

`tools/capture-window.ps1` abre um binário, mexe na câmera (roda, arrasto, teclas) e salva a janela em PNG. Se a janela não ficar ativa, o script não envia nenhuma entrada e não captura. Exemplo, depois de `cargo build -p corum-viewer`:

```powershell
.\tools\capture-window.ps1 -Out .\target\verification\shots\mapa -Wheel -14 -Drag -60
```

O arrasto do mouse depende de onde o cursor estava, então o enquadramento varia entre execuções. Para comparações A/B, fixe a câmera com `CORUM_CAMERA=yaw,pitch,distância` (radianos, radianos, tiles; também vale para `R`) e alterne a tecla com `-Keys`:

```powershell
$env:CORUM_CAMERA = '0.3,0.55,14'
.\tools\capture-window.ps1 -Out .\target\verification\shots\luz-on
.\tools\capture-window.ps1 -Out .\target\verification\shots\luz-off -Keys 'l'
Remove-Item Env:CORUM_CAMERA
```

### O que ainda falta para o mapa ficar 100%

O passo a passo (com ordem, critérios de aceite e riscos) está em [`docs/VISUAL_ROADMAP.md`](../../docs/VISUAL_ROADMAP.md). Resumo:

- objetos `GX_OBJECT` (`.MOD` posicionados) e efeitos `.ofg` (árvores, flores,
  fogo), presentes em outros mapas;
- normais por vértice do `.STM` (hoje a normal é a da face);
- fundo/céu e névoa (hoje é uma cor sólida) e materiais de água/transparência.

Os modelos `.MOD` de personagens, monstros e NPCs são decodificados em 93 a 100% das
malhas, com textura, esqueleto, pele e animação (ver `corum-assets/README.md`). A próxima etapa
é misturar movimentos, disparar ataque/dano/morte pelos slots do `.chr` e levar a pele para a GPU.
