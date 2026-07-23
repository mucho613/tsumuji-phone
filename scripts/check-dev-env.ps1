[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Failures = 0

function Test-Command {
    param(
        [Parameter(Mandatory)]
        [string] $Name,
        [Parameter(Mandatory)]
        [string] $InstallHint
    )

    $Command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($Command) {
        Write-Host ("ok   {0,-18} {1}" -f $Name, $Command.Source)
    } else {
        Write-Host ("miss {0,-18} {1}" -f $Name, $InstallHint)
        $script:Failures++
    }
}

Test-Command "cargo" "Install Rust with rustup using the stable-x86_64-pc-windows-msvc toolchain."
Test-Command "rustc" "Install Rust with rustup."
Test-Command "node" "Install Node.js 22 LTS."
Test-Command "npm" "Install npm."

$VsWhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $VsWhere -PathType Leaf) {
    $MSBuild = & $VsWhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -find "MSBuild\**\Bin\MSBuild.exe" |
        Select-Object -First 1
} else {
    $MSBuild = $null
}

if ($MSBuild) {
    Write-Host ("ok   {0,-18} {1}" -f "MSBuild", $MSBuild)
} else {
    Write-Host ("miss {0,-18} {1}" -f "MSBuild", "Install Visual Studio 2022 Build Tools with Desktop development with C++.")
    $Failures++
}

if ($Failures -ne 0) {
    throw "$Failures required development dependency check(s) failed."
}

Write-Host ""
Write-Host "Windows desktop development environment is ready."
