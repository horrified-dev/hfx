<#
.SYNOPSIS
Installer QA in temporary workspace paths. Use -HelpersOnly on non-Windows hosts.
#>
[CmdletBinding()]
param([string] $Binary = '', [switch] $HelpersOnly, [switch] $MockWindows)
. (Join-Path $PSScriptRoot '../scripts/windows-common.ps1')
$Root = Split-Path $PSScriptRoot -Parent
$Target = Join-Path $Root 'target'
[void] [IO.Directory]::CreateDirectory($Target)
$Temporary = Join-Path $Target ('windows-install-qa-' + [guid]::NewGuid().ToString('N'))
[void] [IO.Directory]::CreateDirectory($Temporary)
$PreviousOS = $env:OS
$PreviousAppData = $env:APPDATA
$PreviousCargoTarget = $env:CARGO_TARGET_DIR
$PreviousBuildTarget = $env:CARGO_BUILD_TARGET
$Shell = $null

class HfxShortcutFixture {
    [string] $Path
    [string] $TargetPath = ''
    [string] $Description = ''
    [string] $WorkingDirectory = ''
    [string] $IconLocation = ''
    HfxShortcutFixture([string] $Path) {
        $this.Path = $Path
        if ([IO.File]::Exists($Path)) {
            $data = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
            $this.TargetPath = $data.TargetPath
            $this.Description = $data.Description
            $this.WorkingDirectory = $data.WorkingDirectory
            $this.IconLocation = $data.IconLocation
        }
    }
    [void] Save() {
        [IO.File]::WriteAllText($this.Path, ($this | ConvertTo-Json))
    }
}
class HfxShellFixture {
    [HfxShortcutFixture] CreateShortcut([string] $Path) { return [HfxShortcutFixture]::new($Path) }
}

function Assert-True([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw "Assertion failed: $Message" }
}
function Assert-Fails([scriptblock] $Action) {
    $failed = $false
    try { & $Action } catch { $failed = $true }
    Assert-True $failed 'operation must fail rather than overwrite unmanaged files'
}
function Invoke-Install {
    param([switch] $DesktopShortcut)
    $parameters = @{ Binary = $Source; InstallDir = $Install; ShortcutDir = $Programs; DesktopDir = $Desktop }
    if ($DesktopShortcut) { $parameters.DesktopShortcut = $true }
    & (Join-Path $Root 'scripts/install-windows.ps1') @parameters
}
function Invoke-Uninstall {
    & (Join-Path $Root 'scripts/uninstall-windows.ps1') -InstallDir $Install
}
try {
    # Parse even on Linux; the native COM/UI steps below run only on Windows.
    foreach ($script in Get-ChildItem -LiteralPath (Join-Path $Root 'scripts') -Filter '*windows*.ps1') {
        $tokens = $null
        $errors = $null
        $null = [Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref] $tokens, [ref] $errors)
        Assert-True (@($errors).Count -eq 0) "syntax of $($script.Name)"
    }
    $Fixture = Join-Path $Temporary 'helpers space [literal]'
    [void] [IO.Directory]::CreateDirectory($Fixture)
    Assert-True ($null -eq (Get-HfxInstallState $Fixture)) 'empty directory is unowned'
    Assert-Fails { Resolve-HfxPath 'relative-path' }
    Assert-Fails { Resolve-HfxPath ([IO.Path]::GetPathRoot($Temporary)) }
    $Marker = Join-Path $Fixture $HfxMarkerName
    $state = @{ Signature = $HfxSignature; InstallDirectory = $Fixture; ShortcutPaths = @((Join-Path $Temporary 'hfx.lnk')) }
    $state | ConvertTo-Json | Set-Content -LiteralPath $Marker -Encoding UTF8
    Assert-True ((Get-HfxInstallState $Fixture).Signature -eq $HfxSignature) 'valid marker with literal paths'
    $state.Signature = 'not hfx'
    $state | ConvertTo-Json | Set-Content -LiteralPath $Marker -Encoding UTF8
    Assert-Fails { Get-HfxInstallState $Fixture }
    $state.Signature = $HfxSignature
    $state.ShortcutPaths = @('relative.lnk')
    $state | ConvertTo-Json | Set-Content -LiteralPath $Marker -Encoding UTF8
    Assert-Fails { Get-HfxInstallState $Fixture }
    if ($env:OS -ne 'Windows_NT') {
        Assert-Fails { Confirm-HfxWindows }
        Assert-Fails { & (Join-Path $Root 'scripts/install-windows.ps1') -Binary $Marker -InstallDir $Fixture }
        Assert-Fails { & (Join-Path $Root 'scripts/uninstall-windows.ps1') -InstallDir $Fixture }
        $link = Join-Path $Temporary 'redirected'
        New-Item -ItemType SymbolicLink -Path $link -Target $Fixture | Out-Null
        Assert-Fails { Get-HfxInstallState $link }
        Remove-Item -LiteralPath $link
    }
    Write-Output 'Windows script parser/filesystem helper checks passed.'
    if ($HelpersOnly) { return }
    if ($MockWindows) {
        if ($env:OS -eq 'Windows_NT') { throw '-MockWindows is only for non-Windows filesystem QA.' }
        # Simulate shortcut metadata only. This is NOT native Windows COM validation.
        $env:OS = 'Windows_NT'
        function New-Object([string] $ComObject) {
            if ($ComObject -ne 'WScript.Shell') { throw 'Unexpected COM request in fixture.' }
            return [HfxShellFixture]::new()
        }
    }
    Confirm-HfxWindows
    $env:APPDATA = Join-Path $Temporary 'roaming data'
    $env:CARGO_BUILD_TARGET = $null
    $Install = Join-Path $Temporary ('installed space [literal] $ ' + [char]0x65e5 + [char]0x672c)
    $Programs = Join-Path $Temporary 'localized Start menu'
    $Desktop = Join-Path $Temporary 'redirected Desktop'
    $Source = Join-Path $Temporary 'source binary.exe'
    if ($MockWindows) {
        [IO.File]::WriteAllText($Source, 'filesystem-only executable fixture')
    } elseif ([string]::IsNullOrWhiteSpace($Binary)) {
        # The installer does not run the binary; cmd.exe is a native PE fixture.
        Copy-Item -LiteralPath $env:ComSpec -Destination $Source
    } else { Copy-Item -LiteralPath (Get-Item -LiteralPath $Binary).FullName -Destination $Source }
    $ForeignTarget = $Source
    $Shell = New-Object -ComObject WScript.Shell
    Invoke-Install -DesktopShortcut
    $Installed = Join-Path $Install 'hfx.exe'
    if (-not $MockWindows) {
        Add-Type -AssemblyName System.Drawing
        $nativeIcon = [Drawing.Icon]::new((Join-Path $Install 'hfx.ico'))
        try { Assert-True ($nativeIcon.Width -gt 0) 'Windows decodes the installed icon' }
        finally { $nativeIcon.Dispose() }
    }
    Assert-True ((Get-FileHash -LiteralPath $Installed).Hash -eq (Get-FileHash -LiteralPath $Source).Hash) 'binary copied exactly'
    foreach ($dir in @($Programs, $Desktop)) {
        $path = Join-Path $dir 'hfx.lnk'
        Assert-True (Test-HfxManagedShortcut $Shell $path $Installed) 'owned native shortcut'
        $shortcut = $Shell.CreateShortcut($path)
        try {
            Assert-True ($shortcut.WorkingDirectory -eq (Join-Path $env:APPDATA 'hfx\workspace')) 'safe starter workspace'
            Assert-True ($shortcut.IconLocation -like '*hfx.ico*') 'native icon configured'
        } finally { Close-HfxComObject $shortcut }
    }
    $Data = Join-Path $env:APPDATA 'hfx\data'
    [void] [IO.Directory]::CreateDirectory($Data)
    [IO.File]::WriteAllText((Join-Path $Data 'chats.json'), 'preserve chats')
    [IO.File]::WriteAllText((Join-Path $Data 'codex-auth.json'), 'fixture credentials')
    $WorkspaceFile = Join-Path $env:APPDATA 'hfx\workspace\user-file'
    [IO.File]::WriteAllText($WorkspaceFile, 'preserve workspace')
    $UserFile = Join-Path $Install 'user-file'
    [IO.File]::WriteAllText($UserFile, 'preserve extra files')
    # A destination-directory failure after binary replacement restores all files.
    $Blocked = Join-Path $Temporary 'blocked desktop'
    [IO.File]::WriteAllText($Blocked, 'foreign file')
    $BeforeFailure = (Get-FileHash -LiteralPath $Installed).Hash
    Assert-Fails {
        & (Join-Path $Root 'scripts/install-windows.ps1') -Binary $Source -InstallDir $Install -ShortcutDir $Programs -DesktopShortcut -DesktopDir $Blocked
    }
    Assert-True ((Get-FileHash -LiteralPath $Installed).Hash -eq $BeforeFailure) 'failed publication restored binary'
    Assert-True (@((Get-HfxInstallState $Install).ShortcutPaths).Count -eq 2) 'failed publication restored marker'
    Assert-True ((Get-Content -LiteralPath $Blocked -Raw) -eq 'foreign file') 'blocked foreign path is preserved'
    Assert-True (Test-HfxManagedShortcut $Shell (Join-Path $Programs 'hfx.lnk') $Installed) 'failed publication restored shortcut'
    # Repeat installs keep a previously installed Desktop shortcut without its flag.
    Invoke-Install
    Assert-True (@((Get-HfxInstallState $Install).ShortcutPaths).Count -eq 2) 'earlier shortcuts stay recorded'
    # A locked executable fails safely, retaining the old bytes and marker.
    $before = (Get-FileHash -LiteralPath $Installed).Hash
    if (-not $MockWindows) {
    $lock = [IO.File]::Open($Installed, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        Assert-Fails { Invoke-Install }
        Assert-Fails { Invoke-Uninstall }
    } finally { $lock.Dispose() }
    }
    Assert-True ((Get-FileHash -LiteralPath $Installed).Hash -eq $before) 'locked update kept original binary'
    Assert-True (Test-HfxExists (Join-Path $Install $HfxMarkerName)) 'locked update kept marker'
    # Exercise -NoBuild with a custom Cargo target directory and native target subdir.
    $env:CARGO_TARGET_DIR = Join-Path $Temporary 'cargo target space'
    $env:CARGO_BUILD_TARGET = 'fixture-target'
    $Release = Join-Path $env:CARGO_TARGET_DIR 'fixture-target\release'
    [void] [IO.Directory]::CreateDirectory($Release)
    Copy-Item -LiteralPath $Source -Destination (Join-Path $Release 'hfx.exe')
    & (Join-Path $Root 'scripts/install-windows.ps1') -NoBuild -InstallDir $Install -ShortcutDir $Programs
    Assert-True ((Get-FileHash -LiteralPath $Installed).Hash -eq (Get-FileHash -LiteralPath $Source).Hash) 'NoBuild uses target directory'
    # Do not remove a user-replaced shortcut during uninstall.
    $ForeignShortcut = Join-Path $Desktop 'hfx.lnk'
    $shortcut = $Shell.CreateShortcut($ForeignShortcut)
    try { $shortcut.TargetPath = $ForeignTarget; $shortcut.Description = 'user replacement'; $shortcut.Save() }
    finally { Close-HfxComObject $shortcut }
    Invoke-Uninstall
    Assert-True (-not (Test-HfxExists $Installed)) 'executable removed'
    Assert-True (-not (Test-HfxExists (Join-Path $Programs 'hfx.lnk'))) 'owned shortcut removed'
    Assert-True (Test-HfxExists $ForeignShortcut) 'foreign replacement preserved'
    Assert-True ((Get-Content -LiteralPath $UserFile -Raw) -eq 'preserve extra files') 'extra files preserved'
    Assert-True ((Get-Content -LiteralPath (Join-Path $Data 'chats.json') -Raw) -eq 'preserve chats') 'chats preserved'
    Assert-True ((Get-Content -LiteralPath (Join-Path $Data 'codex-auth.json') -Raw) -eq 'fixture credentials') 'credentials preserved'
    Assert-True ((Get-Content -LiteralPath $WorkspaceFile -Raw) -eq 'preserve workspace') 'workspace preserved'
    Assert-Fails { Invoke-Install } # Existing nonempty unmanaged install directory.
    Remove-Item -LiteralPath $UserFile
    Remove-Item -LiteralPath $ForeignShortcut
    # Refuse to overwrite an unrelated Start-menu entry before touching the binary.
    $path = Join-Path $Programs 'hfx.lnk'
    $shortcut = $Shell.CreateShortcut($path)
    try { $shortcut.TargetPath = $ForeignTarget; $shortcut.Description = 'foreign'; $shortcut.Save() }
    finally { Close-HfxComObject $shortcut }
    Assert-Fails { Invoke-Install }
    Assert-True (-not (Test-HfxExists $Installed)) 'foreign shortcut prevented installation'
    Remove-Item -LiteralPath $path
    # Junction refusal does not require Developer Mode or administrator access.
    Remove-Item -LiteralPath $Install
    $linkType = if ($MockWindows) { 'SymbolicLink' } else { 'Junction' }
    New-Item -ItemType $linkType -Path $Install -Target $Fixture | Out-Null
    Assert-Fails { Invoke-Install }
    Assert-Fails { Invoke-Uninstall }
    Remove-Item -LiteralPath $Install
    if ($MockWindows) { Write-Output 'Windows filesystem QA passed with MOCK shortcut metadata (not native COM).' }
    else { Write-Output 'Native Windows installer QA passed (temporary paths only).' }
} finally {
    $env:OS = $PreviousOS
    $env:APPDATA = $PreviousAppData
    $env:CARGO_TARGET_DIR = $PreviousCargoTarget
    $env:CARGO_BUILD_TARGET = $PreviousBuildTarget
    if ($null -ne $Shell) { Close-HfxComObject $Shell }
    Remove-Item -LiteralPath $Temporary -Recurse -Force
}
