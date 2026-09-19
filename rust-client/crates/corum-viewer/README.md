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
- `L`: ligar ou desligar as luzes pontuais do `GX_LIGHT`;
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

As texturas são lidas de `Data\Map_dds\Map_dds.pak` (pasta irmã de `Data\Map`),
decodificadas de DXT1/3/5 para RGBA e enviadas à GPU como um `texture_2d_array`
com mipmaps, filtragem anisotrópica e *alpha test* (DXT1 com 1 bit de alpha). No
mapa `1100`, as 51 texturas únicas dos 53 materiais são resolvidas. Se o pacote
não for encontrado, cada material volta a ter uma cor estável por hash.

Se o segundo argumento for omitido, a sandbox procura o `.stm` ao lado do `.ttb`.

### Iluminação

- **Cor por vértice (`.vcl`)**: os objetos tipo 1 recebem `textura × cor do VCL`, sem luz adicional, porque a cor já contém a iluminação pré-calculada. É o que dá às muralhas e rochas a variação de tom (verde, rosado, alaranjado). No `1100`, as 22.157 cores casam com os 22.157 vértices; se a contagem não bater, o arquivo é ignorado com um aviso.
- **Luzes pontuais (`GX_LIGHT`)**: as 20 luzes do `.map` viram um buffer de uniformes (até 32), com atenuação quadrática até o raio de cada uma. Elas iluminam o que **não** tem cor pré-calculada: o personagem e o mob de teste, e os objetos tipo 3 (o chão principal), que ainda esperam o `.lm`. `L` compara com e sem luzes. Que as luzes do `.map` também sejam aplicadas em tempo de execução é uma **hipótese**: elas podem ter servido apenas para gerar o VCL/LM na ferramenta de mapas.

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

- lightmaps (`1100.lm`, formato ainda não decifrado; os objetos tipo 3 têm
  coordenadas de lightmap, hoje descartadas). É o principal item de iluminação
  que falta: o chão e o piso da arena ainda usam luz genérica;
- objetos `GX_OBJECT` (`.MOD` posicionados) e efeitos `.ofg` (árvores, flores,
  fogo), presentes em outros mapas;
- normais por vértice do `.STM` (hoje a normal é a da face);
- fundo/céu e névoa (hoje é uma cor sólida) e materiais de água/transparência.

Mobs e NPCs do cliente usam, em sua maioria, o trecho ainda não documentado do
formato `.MOD` que liga vértices às costuras e aos ossos. Para exibi-los
corretamente, a próxima etapa é decodificar esse skinning, combinar o manifesto
`.CHR` com a animação `.ANM` e carregar a textura indicada pelo material.
