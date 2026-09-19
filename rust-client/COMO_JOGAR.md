# Como testar o Corum (Rust)

Versão de teste **offline** do cliente reescrito em Rust: um mapa, o seu personagem vestido, andar clicando e a
interface original. Ainda **não** é o jogo completo (veja "O que ainda não existe").

## Abrir

1. Confirme que o cliente original está instalado em `D:\Games\CorumOnline` (os arquivos do jogo não vêm no
   repositório). Se estiver em outra pasta: `.\jogar.ps1 -Dados "C:\...\CorumOnline\Data"`.
2. Dê dois cliques em **`Jogar.bat`** (ou, no PowerShell, dentro de `rust-client`: `.\jogar.ps1`).
   - O executável já está compilado em `target\release\corum-sandbox.exe`; se não existir, o script compila.
   - Opções: `.\jogar.ps1 -Classe 4` (1 guerreiro, 2 sacerdote, 3 invocador, 4 caçadora, 5 maga), `-Mapa 1100`
     (também abrem 5 e 750; o 10001 é um mapa de mundo e sai sem relevo), `-Arma 0` (sem arma), `-Escudo 0`, `-Armadura 2245`, `-Compilar`.

## Controles

| Tecla / mouse | O que faz |
|---|---|
| **clique esquerdo** no chão | o personagem anda até lá, contornando bloqueios (um marcador amarelo mostra o destino) |
| arrastar com o botão esquerdo | gira a câmera (a câmera acompanha o personagem) |
| roda do mouse | aproxima / afasta |
| setas | andam em linha reta; cancelam a rota do clique |
| `Shift` | corre (velocidade maior) |
| **T** | inventário (janela `Item`) |
| **A** | personagem (`Character`) |
| **S** | habilidades (`Skill`) |
| **O** | opções (`Option`) |
| `Esc` | fecha a janela da frente; sem nenhuma aberta, sai do jogo |
| `R` | restaura a câmera |

As letras seguem o `KeyConfig.ini` do cliente original. As janelas abrem nas posições originais (1024 × 768,
escaladas à janela), podem ser **arrastadas pela barra de título**, fechadas pelo "X" e vêm para a frente ao clicar.

## O que ainda não existe

- **Sem servidor, combate, monstros que reagem ou NPCs com fala.** Há um monstro patrulhando o mapa, só isso.
- **A interface é só a "moldura"**: as janelas mostram os desenhos originais (slots de equipamento, campos,
  botões), mas **não têm dados nem texto** (nome, PV, itens no inventário) e os botões ainda não fazem nada. A
  barra principal (PV/PM, atalhos, minimapa, chat) é desenhada por código no cliente original e ainda não foi
  refeita. Uma interface nova está prevista para depois.
- **Chão plano**: o personagem anda em altura zero (a maioria dos mapas é assim no original).
- **Detalhes visuais conhecidos**: o elmo pode ficar escondido pelo cabelo; a empunhadura da arma não foi
  conferida com o original; algumas armas animadas (garras) aparecem incompletas; a janela de personagem mostra
  três botões soltos ("Cancel / Invitation / Close") que pertencem a outro diálogo.
- Ainda não há som nem música.

## Para desenvolvedores

`.\jogar.ps1 -Dev` liga o modo de desenvolvimento: `WASD` anda e as teclas de inspeção do sandbox voltam
(`Tab`, `0`, `F`, `G`, `H`, `L`, `B`, `O`); as janelas abrem com `I`, `C`, `K` e `P`. Os documentos técnicos estão em
`crates\corum-viewer\README.md`, `crates\corum-assets\README.md` e `docs\VISUAL_ROADMAP.md`.
