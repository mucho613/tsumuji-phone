[CmdletBinding()]
param(
    [ValidateSet("all", "arm64-v8a", "x86_64")]
    [string]$Abi = "all",
    [ValidateRange(1, 64)]
    [int]$Jobs = 1
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$SourceDir = Join-Path $Root "vendor\pjproject"
$ConfigSource = Join-Path $Root "native\pjsip-config\android\config_site.h"
$ConfigDestination = Join-Path $SourceDir "pjlib\include\pj\config_site.h"
$TargetDir = Join-Path $Root "target"
$DependencyDir = Join-Path $TargetDir "android-deps"
$RuntimeDir = Join-Path $TargetDir "android-jni-libs"
$NdkWrapperDir = Join-Path $TargetDir "android-ndk-wrapper"
$OboeVersion = "1.10.0"
$OboeArchive = Join-Path $DependencyDir "oboe-$OboeVersion.aar"
$OboeDir = Join-Path $DependencyDir "oboe-$OboeVersion"
$OboeSha256 = "34f5a2b43923029bd9ba3180a7e5fb6d030964c25feb767c99d340039df79292"
$OboeUrl = "https://github.com/google/oboe/releases/download/$OboeVersion/oboe-$OboeVersion.aar"

if (-not (Test-Path (Join-Path $SourceDir "configure-android") -PathType Leaf)) {
    throw "PJSIP submodule is missing. Run: git submodule update --init"
}
if (-not (Test-Path $ConfigSource -PathType Leaf)) {
    throw "Android config_site.h was not found: $ConfigSource"
}

$GitBash = Join-Path $env:ProgramFiles "Git\bin\bash.exe"
if (-not (Test-Path $GitBash -PathType Leaf)) {
    throw "Git Bash was not found. Install Git for Windows."
}

$AndroidSdkRoot = if ($env:ANDROID_HOME) {
    $env:ANDROID_HOME
} elseif ($env:ANDROID_SDK_ROOT) {
    $env:ANDROID_SDK_ROOT
} else {
    Join-Path $env:LOCALAPPDATA "Android\Sdk"
}
$NdkParent = Join-Path $AndroidSdkRoot "ndk"
if (-not (Test-Path $NdkParent -PathType Container)) {
    throw "Android NDK was not found under: $NdkParent"
}
$NdkRoot = Get-ChildItem $NdkParent -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName "ndk-build.cmd") -PathType Leaf } |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1 -ExpandProperty FullName
if (-not $NdkRoot) {
    throw "Android NDK with ndk-build.cmd was not found under: $NdkParent"
}
$Make = Join-Path $NdkRoot "prebuilt\windows-x86_64\bin\make.exe"
if (-not (Test-Path $Make -PathType Leaf)) {
    throw "The NDK make executable was not found: $Make"
}

New-Item -ItemType Directory -Force $DependencyDir, $RuntimeDir, $NdkWrapperDir | Out-Null
if (-not (Test-Path $OboeArchive -PathType Leaf) -or
    (Get-FileHash $OboeArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $OboeSha256) {
    Invoke-WebRequest -Uri $OboeUrl -OutFile $OboeArchive
}
$ActualOboeHash = (Get-FileHash $OboeArchive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ActualOboeHash -ne $OboeSha256) {
    throw "Oboe archive hash mismatch. Expected $OboeSha256, got $ActualOboeHash."
}
if (-not (Test-Path (Join-Path $OboeDir "prefab\modules\oboe") -PathType Container)) {
    if (Test-Path $OboeDir) {
        Remove-Item -LiteralPath $OboeDir -Recurse -Force
    }
    [System.IO.Compression.ZipFile]::ExtractToDirectory($OboeArchive, $OboeDir)
}

Copy-Item -Force (Join-Path $NdkRoot "source.properties") $NdkWrapperDir
New-Item -ItemType Directory -Force (Join-Path $NdkWrapperDir "meta") | Out-Null
Copy-Item -Force (Join-Path $NdkRoot "meta\platforms.json") (Join-Path $NdkWrapperDir "meta")
Copy-Item -Force (Join-Path $PSScriptRoot "ndk-build-windows.sh") (Join-Path $NdkWrapperDir "ndk-build")
Copy-Item -Force $ConfigSource $ConfigDestination

function Convert-ToBashPath([string]$Path) {
    $FullPath = [System.IO.Path]::GetFullPath($Path).Replace("\", "/")
    if ($FullPath -match "^([A-Za-z]):/(.*)$") {
        return "/$($Matches[1].ToLowerInvariant())/$($Matches[2])"
    }
    return $FullPath
}

function Invoke-Native([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory) {
    Push-Location $WorkingDirectory
    try {
        & $Command @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$Command failed with exit code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
}

$RequestedAbis = if ($Abi -eq "all") { @("arm64-v8a", "x86_64") } else { @($Abi) }
$BashSourceDir = Convert-ToBashPath $SourceDir
$BashNdkWrapperDir = Convert-ToBashPath $NdkWrapperDir
$WindowsNdkRoot = $NdkRoot.Replace("\", "/")
$WindowsOboeDir = $OboeDir.Replace("\", "/")
$GitTools = Join-Path $env:ProgramFiles "Git\usr\bin"
$env:PATH = "$GitTools;$env:PATH"

foreach ($CurrentAbi in $RequestedAbis) {
    $Prefix = Join-Path $TargetDir "pjproject-android\$CurrentAbi"
    $WindowsPrefix = $Prefix.Replace("\", "/")
    $ConfigureCommand = @"
set -euo pipefail
cd '$BashSourceDir'
export ANDROID_NDK_ROOT='$BashNdkWrapperDir'
export ANDROID_NDK_WINDOWS_ROOT='$WindowsNdkRoot'
export APP_PLATFORM='android-31'
export TARGET_ABI='$CurrentAbi'
./configure-android --use-ndk-cflags \
  --prefix='$WindowsPrefix' \
  --with-oboe='$WindowsOboeDir' \
  --disable-video \
  --disable-sdl \
  --disable-ffmpeg \
  --disable-v4l2 \
  --disable-openh264 \
  --disable-libyuv \
  --disable-opus \
  --disable-gsm-codec \
  --disable-g722-codec \
  --disable-g7221-codec \
  --disable-ilbc-codec \
  --disable-l16-codec
"@
    Invoke-Native $GitBash @("-lc", $ConfigureCommand) $Root

    $BuildMak = Join-Path $SourceDir "build.mak"
    $BuildMakContents = [System.IO.File]::ReadAllText($BuildMak)
    $BuildMakContents = $BuildMakContents.Replace(
        "export PJDIR := $BashSourceDir",
        "export PJDIR := $($SourceDir.Replace('\', '/'))"
    )
    [System.IO.File]::WriteAllText($BuildMak, $BuildMakContents)

    $ShellArgument = "SHELL=$($GitBash.Replace('\', '/'))"
    Invoke-Native $Make @($ShellArgument, "clean") $SourceDir
    Invoke-Native $Make @($ShellArgument, "-j$Jobs", "lib") $SourceDir
    Invoke-Native $Make @($ShellArgument, "install") $SourceDir

    $OboeAbiDir = Join-Path $OboeDir "prefab\modules\oboe\libs\android.$CurrentAbi"
    $RuntimeAbiDir = Join-Path $RuntimeDir $CurrentAbi
    New-Item -ItemType Directory -Force $RuntimeAbiDir | Out-Null
    Copy-Item -Force (Join-Path $OboeAbiDir "liboboe.so") $RuntimeAbiDir

    Write-Host ""
    Write-Host "PJSIP Android libraries are ready for ${CurrentAbi}:"
    Write-Host "  $Prefix"
}
