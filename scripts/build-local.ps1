param(
    [string]$CompilerRoot = (Join-Path $env:TEMP 'discord-vr-portable-compiler')
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $repoRoot

# npm starts this script in a separate process. These settings do not change
# the user's default Rust toolchain, PATH or PowerShell execution policy.
$rustBin = Join-Path $env:USERPROFILE '.cargo/bin'
$rustup = Join-Path $rustBin 'rustup.exe'
if (!(Test-Path -LiteralPath $rustup -PathType Leaf)) {
    throw 'Rust is required. Install rustup before running this command.'
}
$compiler = Get-ChildItem -LiteralPath $CompilerRoot -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'llvm-mingw-*-msvcrt-x86_64' } |
    Sort-Object Name -Descending | Select-Object -First 1
if (!$compiler) {
    throw "Portable LLVM-MinGW (msvcrt, x86_64) was not found under $CompilerRoot. See docs/DEVELOPMENT.md."
}
$compilerBin = Join-Path $compiler.FullName 'bin'
$linker = Join-Path $compilerBin 'x86_64-w64-mingw32-clang.exe'
$runtime = Join-Path $CompilerRoot 'gcc-runtime'
foreach ($required in @($linker, (Join-Path $runtime 'libgcc_eh.a'), (Join-Path $runtime 'libgcc.a'))) {
    if (!(Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required GNU build dependency was not found: $required"
    }
}
$toolchains = & $rustup toolchain list
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if (!($toolchains | Where-Object { $_ -match '^stable-x86_64-pc-windows-gnu(?:\s|$)' })) {
    throw 'Run: rustup toolchain install stable-x86_64-pc-windows-gnu'
}
if (!(Test-Path -LiteralPath (Join-Path $repoRoot 'node_modules/@tauri-apps/cli/tauri.js') -PathType Leaf)) {
    throw 'Run npm.cmd ci before building.'
}
if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
    $signingKey = Join-Path $env:LOCALAPPDATA 'DiscordToVR-release-signing/updater.key'
    if (!(Test-Path -LiteralPath $signingKey -PathType Leaf)) {
        throw 'Set TAURI_SIGNING_PRIVATE_KEY to your existing updater key. Do not regenerate the release key.'
    }
    $env:TAURI_SIGNING_PRIVATE_KEY = $signingKey
}
if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR = Join-Path $repoRoot 'target/tauri'
} elseif (![IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR = [IO.Path]::GetFullPath((Join-Path $repoRoot $env:CARGO_TARGET_DIR))
}
$env:PATH = $rustBin + ';' + $compilerBin + ';' + $env:PATH
$env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'
$env:CARGO_BUILD_TARGET = $null
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $linker
$env:CC_x86_64_pc_windows_gnu = $linker
$env:RUSTFLAGS = $null
$env:CARGO_ENCODED_RUSTFLAGS = '-L' + [char]31 + 'native=' + $runtime

Write-Host 'Building a signed Windows release using the local GNU toolchain.'
& node (Join-Path $repoRoot 'node_modules/@tauri-apps/cli/tauri.js') build --ci -- --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& node (Join-Path $PSScriptRoot 'release-manifest.mjs')
exit $LASTEXITCODE
