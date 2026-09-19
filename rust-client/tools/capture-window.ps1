<#
.SYNOPSIS
    Abre um binário gráfico do workspace, opcionalmente mexe na câmera e salva a janela em PNG.

.DESCRIPTION
    Serve para verificar visualmente mudanças de renderização (mapa, luz, modelos) sem depender
    de captura manual. A janela é colocada em primeiro plano; se ela NÃO ficar ativa, o script
    não envia nenhum clique/rolagem/tecla (para não agir sobre outro programa) e não captura.

.EXAMPLE
    # Sandbox de mapa, câmera afastada e inclinada
    .\tools\capture-window.ps1 -Out .\target\verification\shots\map -Wheel -14 -Drag -60

.EXAMPLE
    # Outro binário/argumentos
    .\tools\capture-window.ps1 -Out .\shot -Binary corum-viewer -Arguments 'C:\x\dfymiss.mod'

.PARAMETER Out     Caminho base (sem extensão). Gera <Out>.png e <Out>.log (stderr do programa).
.PARAMETER Binary  Nome do binário em target\debug (padrão: corum-sandbox). Rode `cargo build` antes.
.PARAMETER Wait    Segundos de espera até a janela abrir e carregar (padrão 8).
.PARAMETER Wheel   Passos da roda do mouse (negativo afasta a câmera).
.PARAMETER Drag    Pixels de arrasto vertical com o botão esquerdo (orbita a câmera).
.PARAMETER Keys    Texto para SendKeys (ex.: '{TAB}', '0', 'g').
#>
param(
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Binary = 'corum-sandbox',
    [string[]]$Arguments = @(),
    [int]$Wait = 8,
    [int]$Wheel = 0,
    [int]$Drag = 0,
    [string]$Keys = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class CorumWin {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, int e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, int flags, int extra);
  public struct RECT { public int L, T, R, B; }
}
"@

$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\debug\$Binary.exe"
if (-not (Test-Path $exe)) { throw "Binário não encontrado: $exe (rode 'cargo build -p corum-viewer')" }
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null

$startArgs = @{ FilePath = $exe; PassThru = $true; WorkingDirectory = $root; RedirectStandardError = "$Out.log" }
if ($Arguments.Count -gt 0) { $startArgs.ArgumentList = $Arguments }
$process = Start-Process @startArgs
try {
    Start-Sleep -Seconds $Wait
    $process.Refresh()
    $handle = $process.MainWindowHandle
    if ($handle -eq [IntPtr]::Zero) { throw 'A janela não abriu (veja o .log).' }

    [CorumWin]::ShowWindow($handle, 9) | Out-Null
    [CorumWin]::SetWindowPos($handle, [IntPtr](-1), 0, 0, 0, 0, 0x43) | Out-Null   # TOPMOST, sem mover/redimensionar
    # O Windows só deixa trocar o foco se houver uma entrada recente: um Alt sintético resolve.
    for ($try = 0; $try -lt 5 -and [CorumWin]::GetForegroundWindow() -ne $handle; $try++) {
        [CorumWin]::keybd_event(0x12, 0, 0, 0)
        [CorumWin]::SetForegroundWindow($handle) | Out-Null
        [CorumWin]::keybd_event(0x12, 0, 2, 0)
        Start-Sleep -Milliseconds 400
    }
    if ([CorumWin]::GetForegroundWindow() -ne $handle) {
        Write-Output 'A janela não ficou ativa; nenhuma entrada enviada e nada capturado.'
        exit 2
    }

    $rect = New-Object CorumWin+RECT
    [CorumWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $cx = $rect.L + [int](($rect.R - $rect.L) / 2)
    $cy = $rect.T + [int](($rect.B - $rect.T) / 2)
    [CorumWin]::SetCursorPos($cx, $cy) | Out-Null

    if ($Drag -ne 0) {
        [CorumWin]::mouse_event(0x2, 0, 0, 0, 0)
        for ($i = 1; $i -le 10; $i++) {
            [CorumWin]::SetCursorPos($cx, $cy + [int]($Drag * $i / 10)) | Out-Null
            Start-Sleep -Milliseconds 30
        }
        [CorumWin]::mouse_event(0x4, 0, 0, 0, 0)
    }
    for ($i = 0; $i -lt [math]::Abs($Wheel); $i++) {
        [CorumWin]::mouse_event(0x800, 0, 0, [math]::Sign($Wheel) * 120, 0)
        Start-Sleep -Milliseconds 40
    }
    if ($Keys -ne '') { [System.Windows.Forms.SendKeys]::SendWait($Keys) }
    Start-Sleep -Milliseconds 800

    [CorumWin]::GetWindowRect($handle, [ref]$rect) | Out-Null
    $width = $rect.R - $rect.L; $height = $rect.B - $rect.T
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($rect.L, $rect.T, 0, 0, $bitmap.Size)
    $png = "$Out.png"
    $bitmap.Save($png)
    Write-Output "salvo: $png (${width}x${height})"
}
finally {
    if (-not $process.HasExited) { $process.Kill() }
}
