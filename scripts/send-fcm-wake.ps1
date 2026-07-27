[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function ConvertTo-Base64Url {
    param([Parameter(Mandatory)][byte[]]$Bytes)
    [Convert]::ToBase64String($Bytes).TrimEnd("=").Replace("+", "-").Replace("/", "_")
}

try {
    $credentialPath = $env:TSUMUJI_FCM_SERVICE_ACCOUNT
    if ([string]::IsNullOrWhiteSpace($credentialPath) -or
        -not (Test-Path -LiteralPath $credentialPath -PathType Leaf)) {
        throw "Missing service account"
    }

    $credential = Get-Content -LiteralPath $credentialPath -Raw | ConvertFrom-Json
    if ($credential.type -ne "service_account" -or
        [string]::IsNullOrWhiteSpace($credential.project_id) -or
        [string]::IsNullOrWhiteSpace($credential.client_email) -or
        [string]::IsNullOrWhiteSpace($credential.private_key) -or
        [string]::IsNullOrWhiteSpace($credential.token_uri)) {
        throw "Invalid service account"
    }

    $secureFid = Read-Host "FCMテスト用FIDを貼り付けてください" -AsSecureString
    $fid = [Net.NetworkCredential]::new("", $secureFid).Password
    if ([string]::IsNullOrWhiteSpace($fid)) {
        throw "Missing FID"
    }

    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $header = @{
        alg = "RS256"
        typ = "JWT"
    } | ConvertTo-Json -Compress
    $claims = @{
        iss   = $credential.client_email
        scope = "https://www.googleapis.com/auth/firebase.messaging"
        aud   = $credential.token_uri
        iat   = $now
        exp   = $now + 3600
    } | ConvertTo-Json -Compress
    $headerPart = ConvertTo-Base64Url ([Text.Encoding]::UTF8.GetBytes($header))
    $claimsPart = ConvertTo-Base64Url ([Text.Encoding]::UTF8.GetBytes($claims))
    $unsignedJwt = "$headerPart.$claimsPart"

    $rsa = [Security.Cryptography.RSA]::Create()
    try {
        $rsa.ImportFromPem([string]$credential.private_key)
        $signature = $rsa.SignData(
            [Text.Encoding]::UTF8.GetBytes($unsignedJwt),
            [Security.Cryptography.HashAlgorithmName]::SHA256,
            [Security.Cryptography.RSASignaturePadding]::Pkcs1
        )
    } finally {
        $rsa.Dispose()
    }
    $assertion = "$unsignedJwt.$(ConvertTo-Base64Url $signature)"

    $tokenResponse = Invoke-RestMethod `
        -Method Post `
        -Uri $credential.token_uri `
        -ContentType "application/x-www-form-urlencoded" `
        -Body @{
            grant_type = "urn:ietf:params:oauth:grant-type:jwt-bearer"
            assertion  = $assertion
        }
    if ([string]::IsNullOrWhiteSpace($tokenResponse.access_token)) {
        throw "Missing access token"
    }

    $randomBytes = [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
    $callToken = ConvertTo-Base64Url $randomBytes
    $request = @{
        message = @{
            fid     = $fid.Trim()
            data    = @{
                type      = "wake"
                callToken = $callToken
            }
            android = @{
                priority = "high"
                ttl      = "30s"
            }
        }
    } | ConvertTo-Json -Depth 8 -Compress

    $null = Invoke-RestMethod `
        -Method Post `
        -Uri "https://fcm.googleapis.com/v1/projects/$($credential.project_id)/messages:send" `
        -Headers @{ Authorization = "Bearer $($tokenResponse.access_token)" } `
        -ContentType "application/json; charset=utf-8" `
        -Body $request

    Write-Host "FCM wake sent successfully."
} catch {
    Write-Host "FCM wake send failed."
    exit 1
}
