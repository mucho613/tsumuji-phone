[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$SourceDir = Join-Path $Root "vendor\pjproject"
$Project = Join-Path $SourceDir "pjsip-apps\build\libpjproject.vcxproj"
$ConfigSource = Join-Path $Root "native\pjsip-config\windows\config_site.h"
$ConfigDestination = Join-Path $SourceDir "pjlib\include\pj\config_site.h"
$Output = Join-Path $SourceDir "lib\libpjproject-x86_64-x64-vc14-Release.lib"

if (-not (Test-Path $Project -PathType Leaf)) {
    throw "PJSIP submodule is missing. Run: git submodule update --init"
}

$VsWhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $VsWhere -PathType Leaf)) {
    throw "vswhere.exe was not found. Install Visual Studio 2022 Build Tools with Desktop development with C++."
}

$MSBuild = & $VsWhere -latest -products * `
    -requires Microsoft.Component.MSBuild `
    -find "MSBuild\**\Bin\MSBuild.exe" |
    Select-Object -First 1
if (-not $MSBuild) {
    throw "MSBuild was not found. Install Visual Studio 2022 Build Tools with Desktop development with C++."
}

Copy-Item -Force $ConfigSource $ConfigDestination

& $MSBuild $Project `
    /m `
    /nologo `
    /t:Build `
    /p:Configuration=Release `
    /p:Platform=x64
if ($LASTEXITCODE -ne 0) {
    throw "PJSIP build failed with exit code $LASTEXITCODE."
}
if (-not (Test-Path $Output -PathType Leaf)) {
    throw "MSBuild completed but the expected library was not found: $Output"
}

Write-Host ""
Write-Host "PJSIP static library is ready:"
Write-Host "  $Output"
Write-Host "Run: cargo check -p telephony-pjsip --features native-pjsip"
