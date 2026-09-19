<#
.SYNOPSIS
    Abre o Corum (Rust) em modo jogo para testar: mapa, personagem vestido e a interface original.

.DESCRIPTION
    Usa o build de release (`target\release\corum-sandbox.exe`); se ele não existir, compila antes.
    Precisa do cliente original instalado (os arquivos do jogo não vêm no repositório).

    Controles do modo jogo
      clique esquerdo   andar até o ponto clicado (arrastar gira a câmera)
      roda do mouse     aproximar / afastar
      T                 inventário      A  personagem      S  habilidades      O  opções
      Esc               fecha a janela da frente; sem janelas abertas, sai do jogo
      R                 restaura a câmera        Shift  corre quando você anda pelo clique

.PARAMETER Classe   1 guerreiro, 2 sacerdote, 3 invocador, 4 caçadora, 5 maga (padrão 1).
.PARAMETER Mapa     Número do mapa em Data\Map (padrão 604; também funcionam 1100, 5, 750, 10001...).
.PARAMETER Dados    Pasta Data do cliente (padrão D:\Games\CorumOnline\Data).
.PARAMETER Armadura Id do item de armadura (padrão 2200, "Mail").
.PARAMETER Arma     Id do item de arma na mão direita (padrão 1, "Short Sword"); 0 = sem arma.
.PARAMETER Escudo   Id do escudo na mão esquerda (padrão 2400, "Small Shield"); 0 = sem escudo.
.PARAMETER Compilar Força recompilar o release antes de abrir.
.PARAMETER Dev      Modo de desenvolvimento: WASD anda e as teclas de teste do sandbox ficam ativas
                    (I, C, K, P abrem as janelas em vez de T, A, S, O).

.EXAMPLE
    .\jogar.ps1
.EXAMPLE
    .\jogar.ps1 -Classe 4 -Mapa 1100
#>
param(
    [ValidateRange(1, 5)][int]$Classe = 1,
    [int]$Mapa = 604,
    [string]$Dados = 'D:\Games\CorumOnline\Data',
    [int]$Armadura = 2200,
    [int]$Arma = 1,
    [int]$Escudo = 2400,
    [switch]$Compilar,
    [switch]$Dev
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$exe = Join-Path $root 'target\release\corum-sandbox.exe'

$ttb = Join-Path $Dados "Map\$Mapa.ttb"
if (-not (Test-Path $ttb)) {
    throw "Mapa não encontrado: $ttb (confira -Dados e -Mapa)"
}

if ($Compilar -or -not (Test-Path $exe)) {
    Write-Output 'Compilando o release (a primeira vez demora alguns minutos)...'
    Push-Location $root
    try { cargo build --release -p corum-viewer --bin corum-sandbox } finally { Pop-Location }
    if (-not (Test-Path $exe)) { throw 'A compilação não gerou o executável.' }
}

$env:CORUM_DATA = $Dados
$env:CORUM_CLASS = "$Classe"
$env:CORUM_ARMOR = "$Armadura"
$env:CORUM_HEAD = '1001'
$env:CORUM_HELMET = '2000'
if ($Arma -gt 0) { $env:CORUM_ITEM = "$Arma" } else { Remove-Item Env:CORUM_ITEM -ErrorAction SilentlyContinue }
if ($Escudo -gt 0) { $env:CORUM_SHIELD = "$Escudo" } else { Remove-Item Env:CORUM_SHIELD -ErrorAction SilentlyContinue }
if ($Dev) { Remove-Item Env:CORUM_GAME -ErrorAction SilentlyContinue } else { $env:CORUM_GAME = '1' }

Write-Output "Abrindo o mapa $Mapa (classe $Classe)..."
& $exe $ttb
