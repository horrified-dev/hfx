<#
.SYNOPSIS
Remove only a script-managed hfx executable, icon, license, and owned shortcuts.
.DESCRIPTION
Quit hfx first. Saved chats, preferences, credentials, and workspace files are kept.
#>
[CmdletBinding()]
param([string] $InstallDir = '', [switch] $Help)
. (Join-Path $PSScriptRoot 'windows-common.ps1')
if ($Help) { Write-Output 'Usage: uninstall-windows.ps1 [-InstallDir DIR]'; return }
Confirm-HfxWindows
if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = Join-Path (Get-HfxKnownFolder LocalApplicationData) 'Programs\hfx'
}
$InstallDir = Resolve-HfxPath $InstallDir
$State = Get-HfxInstallState $InstallDir
if ($null -eq $State) { throw "No script-managed hfx installation at $InstallDir; nothing was removed." }
foreach ($name in $HfxInstalledFiles) { Confirm-HfxOrdinaryPath (Join-Path $InstallDir $name) }
$Executable = Join-Path $InstallDir 'hfx.exe'
if (Test-HfxExists $Executable) {
    try {
        $probe = [IO.File]::Open($Executable, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $probe.Dispose()
    } catch { throw 'Cannot remove hfx.exe. Quit hfx first and check file permissions.' }
}
# Remove the executable first: a locked binary leaves shortcuts and the marker intact.
if (Test-HfxExists $Executable) { [IO.File]::Delete($Executable) }
$Shell = New-Object -ComObject WScript.Shell
try {
    foreach ($path in @($State.ShortcutPaths)) {
        if (-not (Test-HfxExists $path)) { continue }
        try {
            if (Test-HfxManagedShortcut $Shell $path $Executable) { [IO.File]::Delete($path) }
            else { Write-Warning "Preserved unrelated shortcut: $path" }
        } catch { Write-Warning "Preserved unreadable or redirected shortcut: $path" }
    }
    foreach ($name in $HfxInstalledFiles | Where-Object { $_ -ne 'hfx.exe' }) {
        $path = Join-Path $InstallDir $name
        if (Test-HfxExists $path) { [IO.File]::Delete($path) }
    }
    # Never recursively remove the installation directory: extra user files survive.
    if (@(Get-ChildItem -LiteralPath $InstallDir -Force).Count -eq 0) {
        [IO.Directory]::Delete($InstallDir)
    }
    Write-Output 'hfx uninstalled. Saved chats, preferences, credentials, and workspace files were preserved.'
} finally { Close-HfxComObject $Shell }
