param(
    [string]$Makensis = (Join-Path $env:LOCALAPPDATA 'tauri/NSIS/makensis.exe')
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
if (!(Test-Path -LiteralPath $Makensis -PathType Leaf)) {
    throw 'NSIS is required. Build a Tauri installer first, or specify -Makensis.'
}
$workspace = (Resolve-Path -LiteralPath $repoRoot).Path
$fixtureRoot = Join-Path $workspace ('target/uninstall-hook-fixture-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
$fixtureRoot = (Resolve-Path -LiteralPath $fixtureRoot).Path
if (!$fixtureRoot.StartsWith($workspace + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The fixture must be inside the workspace.'
}
$hook = Get-Content -LiteralPath (Join-Path $repoRoot 'src-tauri/installer-hooks.nsh') -Raw -Encoding UTF8
$productionDirectory = '"$APPDATA\discord-to-vr"'
if ($hook.Split(@($productionDirectory), [StringSplitOptions]::None).Count -ne 2) {
    throw 'Unexpected hook cleanup path. Review the fixture before executing.'
}

foreach ($case in @(
    @{ name='keep-settings'; selected=0; updating=0; removed=$false },
    @{ name='preserve-on-update'; selected=1; updating=1; removed=$false },
    @{ name='explicit-cleanup'; selected=1; updating=0; removed=$true }
)) {
    $caseRoot = Join-Path $fixtureRoot $case.name
    $settings = Join-Path $caseRoot 'discord-to-vr'
    New-Item -ItemType Directory -Path $settings -Force | Out-Null
    $settings = (Resolve-Path -LiteralPath $settings).Path
    if (!$settings.StartsWith($fixtureRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to run cleanup against a path outside the fixture.'
    }
    [IO.File]::WriteAllText((Join-Path $settings 'config.json'), 'synthetic settings')
    [IO.File]::WriteAllText((Join-Path $settings 'updates.json'), 'synthetic update preference')
    $sibling = Join-Path $caseRoot 'must-remain.txt'
    [IO.File]::WriteAllText($sibling, 'unrelated fixture file')
    $hookFile = Join-Path $caseRoot 'hook.nsh'
    $fixtureHook = $hook.Replace($productionDirectory, '"' + $settings + '"')
    [IO.File]::WriteAllText($hookFile, $fixtureHook, [Text.UTF8Encoding]::new($false))
    $fixtureExe = Join-Path $caseRoot 'fixture.exe'
    $fixtureScript = Join-Path $caseRoot 'fixture.nsi'
    $script = @'
Unicode true
!include "LogicLib.nsh"
Name "Discord to VR cleanup fixture"
VIProductVersion "0.0.0.0"
VIAddVersionKey "FileVersion" "0.0.0.0"
VIAddVersionKey "FileDescription" "Isolated cleanup test"
VIAddVersionKey "LegalCopyright" "Test fixture"
!define MANUFACTURER "Discord to VR"
OutFile "@EXE@"
RequestExecutionLevel user
SilentInstall silent
Var DeleteAppDataCheckboxState
Var UpdateMode
!include "@HOOK@"
Section
  StrCpy $DeleteAppDataCheckboxState @SELECTED@
  StrCpy $UpdateMode @UPDATING@
  !insertmacro NSIS_HOOK_POSTUNINSTALL
SectionEnd
'@
    $script = $script.Replace('@EXE@', $fixtureExe).Replace('@HOOK@', $hookFile).
        Replace('@SELECTED@', [string]$case.selected).Replace('@UPDATING@', [string]$case.updating)
    [IO.File]::WriteAllText($fixtureScript, $script, [Text.UTF8Encoding]::new($false))
    & $Makensis /V2 $fixtureScript
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $process = Start-Process -FilePath $fixtureExe -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "Fixture failed: $($case.name)" }
    if ((Test-Path -LiteralPath $settings) -eq $case.removed) {
        throw "Unexpected cleanup result: $($case.name)"
    }
    if (!(Test-Path -LiteralPath $sibling -PathType Leaf)) { throw 'Unrelated file was removed.' }
    if (!$case.removed) {
        if ([IO.File]::ReadAllText((Join-Path $settings 'config.json')) -ne 'synthetic settings' -or
            [IO.File]::ReadAllText((Join-Path $settings 'updates.json')) -ne 'synthetic update preference') {
            throw 'Preserved settings were modified.'
        }
    }
    Write-Host "Installer cleanup fixture passed: $($case.name)"
}
