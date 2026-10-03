<#
.SYNOPSIS
Build/install hfx for the current Windows user, with a Start-menu shortcut.
.DESCRIPTION
Requires Rust 1.95+ and MSVC C++ Build Tools for source builds. -Binary installs a
compatible prebuilt hfx.exe instead. No elevation, PATH, or execution-policy changes.
#>
[CmdletBinding()]
param(
    [switch] $DesktopShortcut,
    [switch] $NoBuild,
    [string] $Binary = '',
    [string] $InstallDir = '',
    [string] $ShortcutDir = '',
    [string] $DesktopDir = '',
    [switch] $Help
)
. (Join-Path $PSScriptRoot 'windows-common.ps1')
if ($Help) {
    Write-Output 'Usage: install-windows.ps1 [-DesktopShortcut] [-NoBuild] [-Binary FILE] [-InstallDir DIR] [-ShortcutDir DIR] [-DesktopDir DIR]'
    return
}
Confirm-HfxWindows
$Root = Split-Path $PSScriptRoot -Parent
if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = Join-Path (Get-HfxKnownFolder LocalApplicationData) 'Programs\hfx'
}
$InstallDir = Resolve-HfxPath $InstallDir
if ($InstallDir -eq [IO.Path]::GetPathRoot($InstallDir).TrimEnd('\', '/')) {
    throw 'Do not install into a filesystem root.'
}
$Executable = Join-Path $InstallDir 'hfx.exe'
$State = Get-HfxInstallState $InstallDir
if ($null -eq $State -and (Test-HfxExists $InstallDir) -and
    @(Get-ChildItem -LiteralPath $InstallDir -Force).Count -ne 0) {
    throw "Refusing a nonempty, unmanaged installation directory: $InstallDir"
}
foreach ($name in $HfxInstalledFiles) { Confirm-HfxOrdinaryPath (Join-Path $InstallDir $name) }
if ([string]::IsNullOrWhiteSpace($ShortcutDir)) { $ShortcutDir = Get-HfxKnownFolder Programs }
$ShortcutDir = Resolve-HfxPath $ShortcutDir
$WantedLinks = @(Join-Path $ShortcutDir 'hfx.lnk')
if ($DesktopShortcut) {
    if ([string]::IsNullOrWhiteSpace($DesktopDir)) { $DesktopDir = Get-HfxKnownFolder DesktopDirectory }
    $WantedLinks += Join-Path (Resolve-HfxPath $DesktopDir) 'hfx.lnk'
}
$WantedLinks = @($WantedLinks | Select-Object -Unique)
$Workspace = Join-Path (Resolve-HfxPath $env:APPDATA) 'hfx\workspace'
Confirm-HfxOrdinaryPath $Workspace -Directory
$Shell = New-Object -ComObject WScript.Shell
$Stage = $null
$Changes = [Collections.Generic.List[object]]::new()
$PendingLink = $null
try {
    foreach ($path in $WantedLinks) {
        if ((Test-HfxExists $path) -and -not (Test-HfxManagedShortcut $Shell $path $Executable)) {
            throw "Refusing to overwrite an unmanaged shortcut: $path"
        }
        Confirm-HfxOrdinaryPath $path
    }
    if ([string]::IsNullOrWhiteSpace($Binary)) {
        if (-not $NoBuild) {
            if ($null -eq (Get-Command cargo -ErrorAction SilentlyContinue)) {
                throw 'Install Rust 1.95+ and the MSVC C++ Build Tools, or pass -Binary FILE.'
            }
            Push-Location -LiteralPath $Root
            try {
                & cargo build --release --locked
                if ($LASTEXITCODE -ne 0) { throw "Cargo build failed with exit code $LASTEXITCODE." }
            } finally { Pop-Location }
        }
        $Target = $env:CARGO_TARGET_DIR
        if ([string]::IsNullOrWhiteSpace($Target)) { $Target = Join-Path $Root 'target' }
        elseif (-not [IO.Path]::IsPathRooted($Target)) { $Target = Join-Path $Root $Target }
        if (-not [string]::IsNullOrWhiteSpace($env:CARGO_BUILD_TARGET)) { $Target = Join-Path $Target $env:CARGO_BUILD_TARGET }
        $Binary = Join-Path $Target 'release\hfx.exe'
    }
    $Source = Get-Item -LiteralPath $Binary -Force
    if ($Source.PSIsContainer -or $Source.Length -eq 0 -or $Source.Extension -ne '.exe') {
        throw "Expected a nonempty, compatible hfx.exe: $Binary"
    }
    $Icon = Join-Path $Root 'assets\hfx.ico'
    if (-not (Test-Path -LiteralPath $Icon -PathType Leaf)) { throw 'Missing assets/hfx.ico; use a complete checkout.' }
    [void] [IO.Directory]::CreateDirectory($InstallDir)
    [void] [IO.Directory]::CreateDirectory($Workspace)
    $Stage = Join-Path $InstallDir ('.hfx-install-' + [guid]::NewGuid().ToString('N'))
    [void] [IO.Directory]::CreateDirectory($Stage)
    $New = Join-Path $Stage 'new'
    $Backup = Join-Path $Stage 'backup'
    [void] [IO.Directory]::CreateDirectory($New)
    [void] [IO.Directory]::CreateDirectory($Backup)
    Copy-Item -LiteralPath $Source.FullName -Destination (Join-Path $New 'hfx.exe')
    Copy-Item -LiteralPath $Icon -Destination (Join-Path $New 'hfx.ico')
    Copy-Item -LiteralPath (Join-Path $Root 'LICENSE') -Destination (Join-Path $New 'LICENSE.txt')
    $Links = @($WantedLinks)
    # Keep earlier owned Desktop/custom shortcuts recorded when their flag is omitted.
    if ($null -ne $State) { $Links += @($State.ShortcutPaths) }
    $Links = @($Links | Select-Object -Unique)
    $Manifest = [ordered]@{ Signature = $HfxSignature; InstallDirectory = $InstallDir; ShortcutPaths = $Links }
    [IO.File]::WriteAllText((Join-Path $New $HfxMarkerName), ($Manifest | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    try {
        foreach ($name in $HfxInstalledFiles) {
            $path = Join-Path $InstallDir $name
            $old = $null
            if (Test-HfxExists $path) {
                $old = Join-Path $Backup $name
                # Windows refuses this if the running executable is locked; no process is killed.
                [IO.File]::Move($path, $old)
            }
            $Changes.Add([pscustomobject]@{ Path = $path; Backup = $old })
            [IO.File]::Move((Join-Path $New $name), $path)
        }
        $Index = 0
        foreach ($path in $WantedLinks) {
            [void] [IO.Directory]::CreateDirectory((Split-Path $path -Parent))
            $old = $null
            if (Test-HfxExists $path) {
                $old = Join-Path $Backup "shortcut-$Index.lnk"
                Copy-Item -LiteralPath $path -Destination $old
            }
            $Changes.Add([pscustomobject]@{ Path = $path; Backup = $old })
            # Save beside the destination so publication is a same-volume rename.
            $PendingLink = Join-Path (Split-Path $path -Parent) ('.hfx-shortcut-' + [guid]::NewGuid().ToString('N') + '.lnk')
            $shortcut = $Shell.CreateShortcut($PendingLink)
            try {
                $shortcut.TargetPath = $Executable
                $shortcut.WorkingDirectory = $Workspace
                $shortcut.IconLocation = (Join-Path $InstallDir 'hfx.ico') + ',0'
                $shortcut.Description = $HfxSignature
                $shortcut.Save()
            } finally { Close-HfxComObject $shortcut }
            if (Test-HfxExists $path) { [IO.File]::Delete($path) }
            [IO.File]::Move($PendingLink, $path)
            $PendingLink = $null
            $Index++
        }
    } catch {
        for ($i = $Changes.Count - 1; $i -ge 0; $i--) {
            $change = $Changes[$i]
            if (Test-HfxExists $change.Path) { [IO.File]::Delete($change.Path) }
            if ($null -ne $change.Backup) { [IO.File]::Copy($change.Backup, $change.Path, $true) }
        }
        throw "Installation failed; previous files were restored. Quit hfx before updating. $($_.Exception.Message)"
    }
    Write-Output "Installed hfx: $Executable"
    Write-Output 'Launch hfx from the Start menu. Rerun this script to update; quit hfx first.'
    Write-Output "For terminal launching, add $InstallDir to your user PATH (not changed automatically)."
} finally {
    if ($null -ne $PendingLink -and (Test-HfxExists $PendingLink)) { [IO.File]::Delete($PendingLink) }
    if ($null -ne $Stage -and (Test-HfxExists $Stage)) { Remove-Item -LiteralPath $Stage -Recurse -Force }
    Close-HfxComObject $Shell
}
