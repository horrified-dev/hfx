# Shared per-user Windows install helpers. Windows PowerShell 5.1 and PowerShell 7.
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$HfxSignature = 'hfx-windows-install-v1'
$HfxMarkerName = '.hfx-windows-install.json'
$HfxInstalledFiles = @('hfx.exe', 'hfx.ico', 'LICENSE.txt', $HfxMarkerName)

function Confirm-HfxWindows {
    if ($env:OS -ne 'Windows_NT') { throw 'This script is Windows-only.' }
}

function Resolve-HfxPath([string] $Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathRooted($Path)) {
        throw "An absolute filesystem path is required: $Path"
    }
    $root = [IO.Path]::GetPathRoot($Path)
    if ([IO.Path]::DirectorySeparatorChar -eq '\' -and ($root -eq '\' -or $root -match '^[A-Za-z]:$')) {
        throw "A fully qualified path is required, not a drive-relative path: $Path"
    }
    $absolute = [IO.Path]::GetFullPath($Path)
    $full = $absolute.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $root = [IO.Path]::GetPathRoot($absolute).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    if ($full -eq $root) { throw 'A filesystem root is not a suitable hfx path.' }
    return $full
}

function Test-HfxExists([string] $Path) {
    return $null -ne (Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue)
}

function Confirm-HfxOrdinaryPath([string] $Path, [switch] $Directory) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if ($null -eq $item) { return }
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $item.PSIsContainer -ne [bool] $Directory) {
        throw "Refusing a symlink, reparse point, or unexpected file type: $Path"
    }
}

function Get-HfxInstallState([string] $Directory) {
    Confirm-HfxOrdinaryPath $Directory -Directory
    $marker = Join-Path $Directory $HfxMarkerName
    Confirm-HfxOrdinaryPath $marker
    if (-not (Test-HfxExists $marker)) { return $null }
    $state = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
    if ($state.Signature -ne $HfxSignature -or
        -not [string]::Equals($state.InstallDirectory, $Directory, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Unrecognized hfx installation marker: $marker"
    }
    foreach ($path in @($state.ShortcutPaths)) {
        if (-not [IO.Path]::IsPathRooted($path) -or [IO.Path]::GetExtension($path) -ne '.lnk') {
            throw "Invalid shortcut path in installation marker: $marker"
        }
    }
    return $state
}

function Test-HfxManagedShortcut($Shell, [string] $Path, [string] $Executable) {
    Confirm-HfxOrdinaryPath $Path
    if (-not (Test-HfxExists $Path)) { return $false }
    $shortcut = $Shell.CreateShortcut($Path)
    try {
        return $shortcut.Description -eq $HfxSignature -and
            [string]::Equals($shortcut.TargetPath, $Executable, [StringComparison]::OrdinalIgnoreCase)
    } finally {
        Close-HfxComObject $shortcut
    }
}

function Get-HfxKnownFolder([Environment+SpecialFolder] $Folder) {
    $path = [Environment]::GetFolderPath($Folder)
    if ([string]::IsNullOrWhiteSpace($path)) { throw "Windows has no configured $Folder folder." }
    return Resolve-HfxPath $path
}

function Close-HfxComObject($Object) {
    if ($null -ne $Object -and [Runtime.InteropServices.Marshal]::IsComObject($Object)) {
        [void] [Runtime.InteropServices.Marshal]::FinalReleaseComObject($Object)
    }
}
