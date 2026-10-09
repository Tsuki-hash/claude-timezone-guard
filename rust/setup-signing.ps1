# 一次性脚本：创建本机代码签名证书并导入信任（v2）
# v1 的 UnknownError 根因：New-SelfSignedCertificate 默认走 CNG（Key Storage
# Provider），Set-AuthenticodeSignature 不支持 CNG 私钥。v2 强制传统 CSP。
$ErrorActionPreference = 'Stop'

# 清掉 v1 的旧证书
Get-ChildItem Cert:\CurrentUser\My |
    Where-Object { $_.Subject -match 'ClaudeFingerprint' } |
    Remove-Item -Force
Get-ChildItem Cert:\CurrentUser\TrustedPeople |
    Where-Object { $_.Subject -match 'ClaudeFingerprint' } |
    Remove-Item -Force

$cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject 'CN=ClaudeFingerprint Dev' `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -KeySpec Signature `
    -Provider 'Microsoft Enhanced Cryptographic Provider v1.0' `
    -KeyUsage DigitalSignature `
    -FriendlyName 'ClaudeFingerprint Dev' `
    -CertStoreLocation Cert:\CurrentUser\My `
    -NotAfter (Get-Date).AddYears(5)
Write-Output "新证书已创建（传统 CSP）: $($cert.Thumbprint)"

$cer = Join-Path $env:TEMP 'claude-fp-dev.cer'
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation Cert:\CurrentUser\TrustedPeople | Out-Null
Remove-Item $cer -Force
Write-Output '证书已导入 CurrentUser\TrustedPeople'

# 立即自测签名能否通过验证
$test = Join-Path $env:TEMP 'sign-test.exe'
Copy-Item 'D:\Agent-Project\Hermes\claude-tools\rust\target\release\claude-fingerprint.exe' $test -Force
$sig = Set-AuthenticodeSignature -FilePath $test -Certificate $cert
Write-Output "签名自测: $($sig.Status)"
Remove-Item $test -Force
