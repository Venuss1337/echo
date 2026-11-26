# A process-local TLS listener with a throwaway certificate chain and no trust-store writes.
param([Parameter(Mandatory)][string] $Certificate, [int] $RedirectPort = 0)
$ErrorActionPreference = 'Stop'
$before = [DateTimeOffset]::UtcNow.AddMinutes(-5)
$after = $before.AddHours(1)
$issuerKey = [Security.Cryptography.RSA]::Create(2048)
$leafKey = [Security.Cryptography.RSA]::Create(2048)
$sha = [Security.Cryptography.HashAlgorithmName]::SHA256
$padding = [Security.Cryptography.RSASignaturePadding]::Pkcs1
$issuer = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=ECHO Fixture CA', $issuerKey, $sha, $padding)
$issuer.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true, $false, 0, $true))
$ca = $issuer.CreateSelfSigned($before, $after)
$leaf = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=fixture.invalid', $leafKey, $sha, $padding)
$san = [Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
$san.AddDnsName('fixture.invalid')
$leaf.CertificateExtensions.Add($san.Build())
$unsigned = $leaf.Create($ca, $before.AddSeconds(1), $after.AddSeconds(-1), [byte[]](1, 2, 3, 4))
$server = [Security.Cryptography.X509Certificates.RSACertificateExtensions]::CopyWithPrivateKey($unsigned, $leafKey)
[IO.File]::WriteAllBytes($Certificate, $ca.RawData)
$pfx = [IO.Path]::ChangeExtension($Certificate, 'pfx')
try {
    [IO.File]::WriteAllBytes($pfx, $server.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx, ''))
    node "$PSScriptRoot/tls-radio.mjs" $pfx $RedirectPort
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    $server.Dispose()
    $unsigned.Dispose()
    $ca.Dispose()
    $leafKey.Dispose()
    $issuerKey.Dispose()
}
