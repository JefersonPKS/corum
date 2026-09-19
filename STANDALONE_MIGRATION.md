# Integração com o pacote Standalone

Este documento registra a decisão de usar o pacote localizado em
`D:\Games\Corum_Standalone_latest` como ambiente executável de referência para o cliente
Rust e para os testes de login.

## Estado da análise

| Componente | Local | Estado |
| --- | --- | --- |
| Cliente standalone | `Installed Client\CorumOnlineResult.exe` | compatível com os servidores do pacote |
| LoginAgent | `INNER\1_LoginAgent Configurations_INNER\LoginAgentResult.exe` | porta local `13100` |
| WorldServer | `INNER\2_WorldServer Configurations\WorldServerResult.exe` | usuário `13201`, servidor `17201` |
| Dungeons | `INNER\3_DungeonServer Configurations_*` | portas `162xx`/`172xx` |
| Dados | `Installed Client\Data` | lidos pelo `corum-assets` |

Os executáveis são PE 32-bit e possuem datas de build próximas entre si (20/07/2026),
indicando que pertencem à mesma distribuição. Os hashes e tamanhos são diferentes dos
executáveis da instalação antiga `D:\Games\CorumOnline`; não devemos misturar DLLs ou
executáveis entre as duas instalações.

## Topologia de rede local

```text
Cliente Rust/legado
        |
        | TCP 127.0.0.1:13100
        v
LoginAgent -- TCP 127.0.0.1:17100 --> WorldServer
                                         |
                                         | TCP 127.0.0.1:17201
                                         v
                                Dungeons/Villages/Passages
```

O cliente standalone confirma essa topologia em `Installed Client\Connect.ini`. Os
scripts `LAUNCH_SERVERS.bat` e `LAUNCH_GAME.bat` são os lançadores oficiais do pacote.

## Evidência de compatibilidade dos dados

Com `CORUM_DATA` apontando para `D:\Games\Corum_Standalone_latest\Installed Client\Data`:

- os 84 testes do crate `corum-assets` passaram;
- o comando `item-check` leu 2.945 itens e 1.213 referências de modelos;
- o comando `item-info` decodificou o item `1` (`Short Sword`) e seu modelo;
- o pacote contém 2.480 arquivos de dados (aproximadamente 1,4 GB).

Isso valida o formato dos recursos para o visualizador Rust, mas não substitui o teste de
rede. O handshake de login será validado pelo `corum-login-probe`.

## Estratégia de DLLs

O cliente Rust não deve carregar `BaseNetwork.dll`, `CommonServer.dll` ou DLLs de
renderização do cliente antigo para executar o fluxo de login. A ordem de migração será:

1. login e cadastro via TCP/Rust;
2. seleção e criação de personagem;
3. entrada no WorldServer;
4. movimentação, entidades e combate;
5. somente então avaliar recursos que ainda dependam do cliente legado.

As DLLs do pacote standalone continuam sendo responsabilidade dos executáveis standalone
enquanto eles forem usados. Elas não serão copiadas para o cliente Rust sem uma dependência
comprovada.

## Como executar o primeiro teste

1. Instale os redistribuíveis VC++ incluídos no pacote, se ainda não estiverem instalados.
2. Execute `D:\Games\Corum_Standalone_latest\LAUNCH_SERVERS.bat`.
3. Em outro terminal, dentro de `rust-client`, execute:

   ```powershell
   cargo run -p corum-net --bin corum-login-probe -- 127.0.0.1:13100 teste senha
   ```

   O probe usa o mesmo pacote `CTWS_LOGIN` que já é exercitado nos testes unitários. Por
   padrão ele usa a versão `0x07032101`; uma versão diferente pode ser informada como quarto
   argumento.

4. Para o cadastro local descrito pelo pacote standalone, repita o teste com o mesmo ID e
   senha conforme o comportamento documentado em `READ_THIS_FIRST.txt`.

O probe não inicia nem encerra servidores e não altera a instalação standalone.

### Primeiro teste executado

Em 19/09/2026, o conjunto mínimo de WorldServer/dungeons foi iniciado e abriu as portas
`13201`, `16201`, `16204` e `17201`. Quando o LoginAgent foi iniciado automaticamente em modo
oculto, ele encerrou com o código Windows `0xC0000409` antes de abrir `13100`/`17100`; por isso o
probe recebeu `WSAECONNREFUSED` (`10061`). Os processos de teste foram encerrados depois da
coleta. Isso é uma falha de inicialização/ambiente do binário standalone, não uma falha
comprovada do codec Rust. O próximo diagnóstico deve executar o LoginAgent pelo launcher
visível original e comparar VC++/DLLs e diretório de trabalho antes de alterar o protocolo.

### Reteste do probe

Com o launcher executado pelo usuário, o TCP passou a conectar em `13100`, mas o servidor
registrou `Unknown packet received!(127.0.0.1)` e fechou a conexão. No cliente isso aparece
como `WSAECONNRESET`/Windows `10054`. Portanto o socket e a porta estão corretos; ainda falta
reproduzir no Rust o transporte legado usado por essa build (o pacote `CTWS_LOGIN` tem 51 bytes,
mas a camada de rede antiga pode acrescentar framing e/ou criptografia antes de entregá-lo ao
LoginAgent). Não devemos alterar o codec de campos nem assumir que `10054` seja senha inválida.

O teste com a conta pré-existente `test3/test3` teve o mesmo resultado no probe. O log do
servidor registra um `Sent to user login success packet: 0` antes da tentativa do probe, o que
confirma que a conta e o servidor funcionam com o cliente standalone; a falha está isolada no
transporte Rust atual.

## Captura do transporte do cliente original

Para descobrir exatamente o framing/criptografia usado por esta build, o workspace agora
contém o binário `corum-wire-proxy`. Ele é um proxy TCP de uma sessão: encaminha os bytes sem
alterá-los e grava cópias binárias e um log hexadecimal de cada leitura.

O proxy não deve ser colocado na porta `13100` enquanto o LoginAgent estiver rodando. Faça uma
alteração temporária no arquivo
`D:\Games\Corum_Standalone_latest\Installed Client\Connect.ini`, trocando apenas:

```ini
set_01_port = 13101
```

Mantenha os servidores standalone em execução e, em um terminal dentro de `rust-client`, rode:

```powershell
cargo run -p corum-net --bin corum-wire-proxy -- 127.0.0.1:13101 127.0.0.1:13100 captures/login-proxy
```

Depois inicie o cliente original e faça uma tentativa com a conta local `test3/test3`. O proxy
aceita uma conexão, encaminha a sessão para o LoginAgent e cria uma pasta
`captures/login-proxy/session-*` com:

- `client-to-server.bin` e `.log`;
- `server-to-client.bin` e `.log`.

Ao terminar, feche o cliente, interrompa o proxy e restaure `set_01_port = 13100` no
`Connect.ini`. Os arquivos de captura podem conter ID, senha e dados de sessão; mantenha-os
locais e não os publique. A captura permitirá comparar a sessão funcional com o probe Rust e
identificar se a build usa pacote cru, prefixo de tamanho, Blowfish/AES ou outra negociação.

### Resultado da captura

A sessão real confirmou o framing exato:

- cliente → LoginAgent: `33 00` + 51 bytes de `CTWS_LOGIN`;
- LoginAgent → cliente: `56 00` + 86 bytes de `WSTC_LOGIN_SUCCESS`.

Os dois bytes são um tamanho little-endian, não criptografia. O payload começa depois desse
prefixo com `01 00` (estado de login e comando), seguido da assinatura `@SAD`. O probe Rust foi
atualizado para escrever e ler esse prefixo, mantendo os codecs dos pacotes inalterados.

O teste seguinte foi concluído com sucesso:

```text
LOGIN_SUCCESS: 1 personagem(ns)
slot=0 índice=1 nome="testkont1" nível=79 mapa=0
```

Isso comprova a compatibilidade do transporte Rust com o LoginAgent standalone, sem carregar
`BaseNetwork.dll` ou alterar o servidor.

### Source como referência do protocolo

Não será necessário capturar cada pacote. A source já define os contratos principais em
`CommonServer/ProtocolDefinition.h`, `LoginPacket.h`, `CharSelectPacket.h` e
`WorldPacket.h`; os dispatchers estão em `LoginAgent/recvmsg.cpp` e o fluxo do cliente em
`CorumOnlineProject/LoginProcess.cpp`, `CharSelectProcess.cpp` e `CharSelectMsg.cpp`.
As capturas ficam reservadas para confirmar o framing, opções condicionais de compilação e
diferenças entre a source e uma build standalone.

### Fase de seleção de personagem

O probe aceita um quinto argumento opcional para selecionar um slot após o login:

```powershell
cargo run -p corum-net --bin corum-login-probe -- 127.0.0.1:13100 test3 test3 0x07032101 0
```

O teste contra o servidor retornou o handoff esperado para o WorldServer:

```text
WORLD_HANDOFF: ip=0x0100007F porta=13201 personagem=1 serial=4294967294 evento=1
```

O pacote `ASTC_CONNECT_WORLD_SERVER` já está modelado em `corum-wire`; a próxima etapa é usar
esses dados para abrir a conexão `13201`, enviar `CTWS_WORLD_LOGIN` e decodificar o primeiro
`WSTC_WORLD_USER_INFO`.

## Limitações conhecidas

- Os arquivos `.ini` ainda possuem campos de banco em `127.0.0.1:1433`, embora a documentação
  do pacote diga que não é necessário instalar SQL. Isso será confirmado no primeiro login.
- Uma resposta `EncryptionKey` significa que o servidor aceitou a conexão, mas a derivação da
  criptografia legada ainda não foi implementada no cliente Rust.
- A compatibilidade comprovada até agora é estrutural e de codecs/recursos; o teste
  cliente-servidor real depende dos servidores estarem em execução.

## Próximas etapas

- implementar seleção de personagem e a conexão subsequente ao WorldServer;
- implementar a derivação da chave, caso o standalone a exija;
- adicionar criação de personagem ao transporte Rust;
- registrar cada comando e pacote confirmado neste documento.
