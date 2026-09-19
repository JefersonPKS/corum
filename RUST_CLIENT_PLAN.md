# Revisão técnica e plano para o cliente Rust

Data da revisão: 2026-09-18

## Decisão

É viável reimplementar o cliente em Rust mantendo os servidores atuais. A estratégia recomendada não é traduzir o C++ arquivo por arquivo, mas preservar o protocolo e o comportamento observável enquanto cada subsistema é substituído.

O primeiro produto deve ser um cliente sem renderização capaz de:

1. conectar ao LoginAgent;
2. autenticar;
3. listar e selecionar personagens;
4. entrar no WorldServer;
5. entrar em um DungeonServer e trocar mensagens básicas.

Só depois desse fluxo estar coberto por testes de bytes e capturas de rede deve começar a reconstrução visual.

## Retrato do repositório

| Área | Evidência encontrada |
|---|---|
| Cliente C++ | 399 arquivos de código e aproximadamente 129.896 linhas em `CorumOnlineProject` |
| Fluxo principal | Aplicação Win32 iniciada por `WinMain`, com estados `INTRO`, `LOGIN`, `CHAR_SELECT`, `WORLD` e `PLAY` |
| Renderização | Direct3D 8 e interfaces COM-like carregadas de DLLs `SS3D*` |
| Rede | `BaseNetwork.dll`, callbacks Win32 e despacho por dois bytes: estado + comando |
| Protocolo | 13 headers principais, aproximadamente 7.388 linhas e cerca de 578 structs, em grande parte com `#pragma pack(1)` |
| Som | Miles Sound System (`mss32`) e `SoundLib.dll` |
| Dados | Tabelas `.cdb`, mapas `.ttb`, recursos `.erd` e assets referenciados como `.chr`, `.mod`, `.tga`, `.dds` e `.wav` |
| Build | Projeto Visual Studio legado, com bibliotecas e executáveis binários versionados |
| Grafel | 2.425 arquivos, 19.030 entidades, 59.265 relações e 76 fluxos indexados no grupo `corum-rust` |

## Cliente de referência encontrado

Foi inspecionado estaticamente o cliente em `D:\Games\CorumOnline`. Ele é adequado como cliente de referência e contém o conjunto de assets que faltava neste checkout.

### Evidências de compatibilidade

| Verificação | Resultado |
|---|---|
| Arquitetura | `CorumOnline.exe` é PE x86 (`0x014c`), como o projeto Win32 legado |
| Linhagem | O executável instalado declara versão `4.7.5.1`, a mesma versão dos recursos do projeto C++ |
| Nome do produto | As configurações antigas do projeto geram `CorumOnline.exe`, confirmando que esse é o executável principal esperado |
| Rede | `BaseNetwork.dll` é idêntica por SHA-256 à cópia em `CorumOnlineProject` |
| Índices de recursos | `Paklist.sin`, `CorumResource.erd` e `DefResource.erd` são idênticos aos arquivos do repositório |
| Pacotes | Os 13 arquivos `.pak` referenciados por `Paklist.sin` estão presentes |
| Assets soltos | De 263 arquivos `.ttb`, `.cdb` e `.pal`, 197 são idênticos, 61 têm o mesmo nome mas conteúdo diferente e 5 não aparecem no repositório |
| Mapas TTB | 194 idênticos e 8 diferentes |
| Tabelas CDB | 2 idênticas, 53 diferentes e 5 ausentes no repositório |

O `CorumOnline.exe` instalado foi linkado em 21/03/2007 e tem SHA-256 `DD725E1B9F7A2F32E858870BE214D1C72C2F038D6ED45D0E01F185B76673AEEA`. O binário preservado junto ao fonte é de 08/01/2005. Portanto, trata-se da mesma família de cliente, mas não da mesma revisão.

O cliente indica revisão `07032101`; o servidor-fonte usa `05033101`. A validação em `LoginAgent/MsgProc.cpp` rejeita apenas clientes com versão **menor** que a do servidor. Assim, a barreira de versão deve aceitar esse cliente mais novo. Isso não prova que todos os pacotes de 2007 mantiveram o layout de 2005.

### Veredito de compatibilidade

- **Assets e formatos para orientar o cliente Rust:** compatível.
- **Cliente original como referência de comportamento:** compatível e muito útil.
- **Login nos servidores deste repositório:** provável, pois assinatura, código de nação padrão, framing e regra de versão são da mesma família.
- **Gameplay completo sem ajustes:** ainda não comprovado. As diferenças nas tabelas CDB e os dois anos de evolução podem afetar itens, skills, mapas e pacotes adicionados.
- **Misturar DLLs do repositório com as do cliente:** não recomendado. As DLLs `SS3D*`, `CommonServer.dll` e `SoundLib.dll` instaladas são revisões diferentes e devem permanecer como um conjunto coerente com o executável de 2007.

Os executáveis não foram iniciados durante esta revisão. Eles são antigos, não possuem assinatura Authenticode e incluem o anti-cheat HShield. O teste dinâmico deve ocorrer em uma VM isolada, sem usar os endereços públicos antigos presentes em `Connect.ini`.

### Smoke test necessário

1. Criar um snapshot da VM com os servidores funcionando.
2. Copiar o cliente para dentro da VM e manter intacto o original.
3. Alterar apenas a cópia de `Connect.ini` para o IP privado do LoginAgent.
4. Capturar a rede com Wireshark ou instrumentar `RecvMsg`/`SendToUser` no servidor.
5. Validar, nesta ordem: conexão, login, lista de personagens, WorldServer, DungeonServer, movimento e chat.
6. Registrar o primeiro comando/tamanho divergente caso a sessão seja desconectada.

Esse teste decide se o cliente de 2007 pode ser usado diretamente para gerar fixtures ou se precisaremos compilar o cliente de 2005 para servir de oráculo do protocolo.

## Progresso da implementação

O primeiro risco do pipeline de assets já foi removido. O crate `rust-client/crates/corum-assets` implementa, sem dependências externas e sem `unsafe`, a leitura e a extração dos contêineres PAK versão 1.

- Os 13 pacotes do cliente foram percorridos integralmente: 10.990 registros e aproximadamente 1,03 GiB.
- Todos os registros obedeceram ao layout documentado e terminaram exatamente no EOF de cada pacote.
- Uma textura DDS real foi extraída e teve assinatura e tamanho confirmados.
- O extrator valida offsets e limites e rejeita caminhos absolutos ou com `..`.
- Testes unitários, `cargo fmt` e `cargo clippy -D warnings` passam.

O formato `.chr` também foi identificado como um manifesto textual que relaciona um `.mod` às suas animações `.anm`.

O primeiro subconjunto de `.mod` e `.anm` já está implementado:

- os 905 MOD do pacote `Character` passam pela leitura estrutural;
- materiais, malhas `F4`, ossos `F5` e registros especiais `F1` são delimitados sem usar as DLLs antigas;
- o modelo mínimo `dfymiss.mod` foi exportado para OBJ com 4 vértices, 4 UVs e 2 triângulos;
- 53 de 1.197 malhas de `Character` já usam o layout estático diretamente exportável;
- os 197 ANM de `Effect` e 259 ANM de `Character` passam pelo parser;
- três tracks de keyframe, de 24, 20 e 36 bytes, foram confirmadas em 3.875 registros sem morph.

O maior risco gráfico agora está concentrado no remapeamento de costuras, pesos de skinning e track de morph. Depois disso, os dados poderão ser convertidos para glTF e carregados por um motor Rust.

## Achados principais

### 1. O motor gráfico é o maior bloqueio

O cliente carrega `SS3DExecutiveForCorum.dll` dinamicamente em `CorumOnlineProject/InitGame.cpp` e usa uma interface C++ muito extensa definida em `4DyuchiGRX_common/IExecutive.h`. Não foi encontrada neste repositório uma implementação correspondente do motor; há principalmente headers, `.lib` e DLLs.

Isso cria duas rotas possíveis:

- **Compatibilidade primeiro:** manter as DLLs antigas temporariamente e criar um pequeno adaptador C++ com ABI C, usado pelo Rust. Essa rota fica restrita a Windows/x86, mas permite validar rapidamente o protocolo e os assets originais.
- **Cliente nativo:** escrever importadores/conversores dos formatos antigos e renderizar com um motor Rust. É a solução sustentável, porém representa a maior parte do esforço.

Para o cliente nativo, a recomendação inicial é avaliar **Fyrox 1.x**, pois já inclui editor, cena 3D, animação, UI, áudio e física. Usar `wgpu` diretamente dá mais controle, mas transfere para o projeto a construção desses sistemas. A escolha deve ser confirmada com um protótipo que carregue um mapa e um personagem reais.

### 2. O protocolo depende da ABI de MSVC 32-bit

Os pacotes são classes/structs C++ serializadas pelo próprio layout em memória. Existem centenas de usos de `sizeof`, herança de headers de pacote e `#pragma pack(1)`. Tipos como `DWORD`, `BOOL`, enums e `time_t` precisam ter tamanhos reproduzidos explicitamente.

No Rust, os pacotes não devem ser implementados por simples cast de memória ou por referências a structs `packed`. Cada campo deve ser lido e escrito explicitamente em little-endian. Para cada pacote portado, um programa auxiliar C++ x86 deve produzir:

- `sizeof` do pacote e de seus componentes;
- offsets de todos os campos;
- bytes de uma instância conhecida;
- bytes reais capturados entre cliente e servidor.

Esses artefatos serão os testes dourados do crate de protocolo.

### 3. O framing da rede ainda precisa ser documentado

`CNetworkClient::SendMsg` entrega um buffer e tamanho ao `BaseNetwork.dll`. O callback recebe mensagens já delimitadas, mas o código-fonte dessa camada não está disponível aqui. Portanto, não se deve presumir que um `read` TCP equivale a um pacote.

Antes do cliente Rust se conectar, é necessário capturar uma sessão do cliente original ou instrumentar os servidores para identificar exatamente:

- prefixo de tamanho/framing usado pela DLL;
- ordem de conexão entre Agent, World e Dungeon;
- comportamento de keep-alive e reconexão;
- variantes de criptografia habilitadas pelas macros de build.

### 4. Há uma falha de validação no dispatcher atual

`ReceivedMsg`, em `CorumOnlineProject/NetworkClient.cpp`, lê os dois primeiros bytes sem verificar `dwLen >= 2` e usa `bStatus` diretamente como primeiro índice de `PacketProc`. Como o primeiro eixo tem `MAX_UPDATE_GAME` posições, um pacote malformado pode causar acesso fora dos limites. Há também um buffer local fixo de 4096 bytes no caminho de descriptografia.

O cliente Rust deve rejeitar, antes de despachar:

- frames menores que o header mínimo;
- estado ou comando fora do intervalo;
- tamanhos diferentes do esperado pelo comando;
- contagens variáveis que ultrapassem o restante do frame;
- strings sem terminador quando o protocolo exigir C string.

### 5. O conjunto de assets do cliente não está completo neste checkout

O código referencia muitos recursos `.chr`, `.mod`, `.tga`, `.dds`, `.wav` e tabelas de manager. No repositório, o conjunto distribuído em `补丁/Data` contém principalmente 226 arquivos `.ttb` e 81 `.cdb`; o README aponta os recursos completos para um download externo.

Antes do trabalho gráfico, é necessário criar um inventário com hash dos arquivos do cliente original e confirmar os formatos realmente usados. Sem isso, é possível concluir a rede e a lógica, mas não reproduzir o jogo visualmente.

### 6. Build, encoding e licença precisam ser normalizados

- O projeto usa Visual Studio antigo, Direct3D 8, Win32, inline assembly e 137 DLLs versionadas. O build atual não é reproduzível em uma máquina moderna sem preparar uma toolchain de referência.
- Muitos fontes e dados parecem usar encoding legado coreano/chinês. A conversão para UTF-8 deve preservar os bytes usados pelo protocolo; nomes enviados ao servidor provavelmente exigirão uma code page específica.
- Não foi encontrada uma licença de software na raiz. Antes de distribuir um cliente refeito ou assets convertidos, é necessário confirmar os direitos de uso e redistribuição.

## Arquitetura proposta

```text
corum-client
├── corum-wire       tipos, codecs e validação de pacotes
├── corum-net        framing, conexões, keep-alive e reconexão
├── corum-domain     personagens, inventário, skills, guilda e combate
├── corum-assets     CDB/TTB/ERD e pipeline de conversão
├── corum-session    estados Login -> Character -> World -> Dungeon
└── corum-render     adaptador do motor; inicialmente headless
```

Regras de arquitetura:

- `corum-wire` não depende de UI, engine, socket ou estado global.
- `corum-net` entrega eventos tipados e nunca expõe buffers recebidos diretamente ao jogo.
- a simulação consome comandos e produz eventos; renderização apenas apresenta o estado.
- parsers usam limites explícitos e não confiam em tamanhos enviados pela rede ou pelos assets.
- recursos originais permanecem fora do Git quando a redistribuição não estiver autorizada.

## Plano por marcos

### Progresso já implementado

- O workspace Rust contém um parser seguro de PAK e parsers estruturais de
  `.CHR`, `.MOD` e `.ANM`.
- Os 13 pacotes do cliente de referência foram lidos, totalizando 10.990
  entradas.
- 905 modelos e 259 animações do pacote de personagens foram validados
  estruturalmente.
- O crate `corum-viewer` abre uma janela nativa com `winit`/`wgpu`, carrega um
  `.MOD` real, envia sua geometria à GPU e oferece câmera orbital e zoom.
- O parser de `.TTB` lê dimensões, tamanho original do tile e os campos de
  atributo, ocupação e seção. O parser textual de `.MAP` extrai limites,
  referência ao `.STM` e objetos posicionados.
- O binário `corum-sandbox` abre a topologia de um mapa real, permite caminhar
  com WASD respeitando a colisão e mantém um mob de teste em patrulha.
- O parser inicial de `.STM` reconhece a tabela de materiais e os objetos
  visuais tipo 1. No mapa `1100`, ele recupera 53 materiais, 9 objetos visuais
  e 9.361 triângulos, já renderizados com cores provisórias por material.
- O crate `corum-assets` decodifica DDS (DXT1/3/5 e RGB/RGBA sem compressão) e
  lê entradas de PAK em memória. A sandbox usa isso para texturizar o cenário
  `1100` com as 51 texturas únicas de `Map_dds.pak`; lightmaps, luzes do `.MAP`,
  objetos posicionados e céu ainda faltam. O roteiro detalhado de mapa, luz e
  sombra, mobs e personagem está em `rust-client/docs/VISUAL_ROADMAP.md`.
- A prova gráfica atual cobre malhas estáticas diretamente decodificáveis. O
  próximo bloqueio visual é o cenário `.STM`; para personagens reais, é o
  skinning/costuras usado pelos mobs, NPCs e personagens.

### Marco 0 — baseline reproduzível

- Executar cliente e servidores originais em ambiente isolado.
- Registrar versões, macros de compilação e configuração regional.
- Gerar manifesto SHA-256 dos binários e assets.
- Capturar uma sessão de login até entrada em dungeon.
- Gerar catálogo C++ de tamanhos e offsets dos pacotes.

**Aceite:** outra máquina consegue repetir a sessão e produzir as mesmas evidências.

### Marco 1 — protocolo Rust

- Criar workspace Cargo e os crates `corum-wire` e `corum-net`.
- Implementar framing descoberto no Marco 0.
- Portar headers e pacotes de login.
- Comparar byte a byte contra fixtures C++ e capturas.
- Adicionar fuzzing aos decoders.

**Aceite:** login e listagem de personagens funcionam sem usar código C++ no processo Rust.

### Marco 2 — cliente headless jogável por comandos

- Portar seleção de personagem, World e o subconjunto mínimo de Dungeon.
- Implementar movimento, chat, spawn/despawn e keep-alive.
- Gravar testes de integração contra uma instância local dos servidores.

**Aceite:** um personagem entra no mapa, se movimenta e conversa por uma CLI.

### Marco 3 — prova de assets e renderer

- Documentar CDB, TTB, ERD e os formatos de modelo/animação realmente encontrados.
- Converter um mapa, um personagem e uma animação para uma representação intermediária versionada.
- Evoluir o protótipo nativo `wgpu`; comparar com a rota temporária do adaptador C++ quando necessário.

**Aceite:** o cliente Rust renderiza um mapa real, um personagem animado e sua posição recebida do servidor.

### Marco 4 — vertical slice

- Login visual, seleção de personagem e uma dungeon.
- Câmera, colisão, movimento, chat, alvo, ataque e inventário mínimo.
- Telemetria local, logs estruturados e relatório de crash sem dados sensíveis.

**Aceite:** uma sessão curta é jogável de ponta a ponta apenas no novo cliente.

### Marco 5 — paridade progressiva

- UI completa, skills, itens, lojas, party, guilda, trade, áudio e efeitos.
- Compatibilidade de localização e input moderno.
- Performance, segurança e empacotamento.

## Primeira entrega recomendada

A próxima mudança no código deve criar apenas o workspace Rust com:

- codec seguro do header comum de dois bytes;
- tipos `GameStatus` e comandos de login;
- encode/decode de `CTWS_LOGIN`, `WSTC_LOGIN_FAIL` e `WSTC_LOGIN_SUCCESS`;
- fixtures geradas pelo C++ x86;
- um binário CLI que conecta, autentica e imprime a lista de personagens.

Esse recorte valida a premissa central — compatibilidade Rust com os servidores existentes — antes de assumir o custo do renderer.

## Grafel

O repositório foi registrado no grupo `corum-rust`. O arquivo `.grafel/group.json` identifica o projeto e o bloco em `AGENTS.md` orienta agentes compatíveis a consultar o grafo para navegação estrutural.

O índice inicial foi concluído. O dashboard local fica em `http://127.0.0.1:47274/`. A criação do watcher como tarefa agendada foi recusada pelo Windows; por isso, até essa permissão ser corrigida, atualize o índice manualmente após mudanças relevantes:

```powershell
grafel index "D:\2 - Projetos\corum"
grafel status corum-rust
```
