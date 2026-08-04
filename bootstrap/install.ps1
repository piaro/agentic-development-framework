# Bootstrap only a binary whose GitHub Artifact Attestation matches the
# repository, build workflow, source revision, branch, and hosted runner.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^framework-[A-Za-z0-9][A-Za-z0-9._-]*$')]
    [string]$Tag,

    [ValidatePattern('^[^/]+/[^/]+$')]
    [string]$Repository = 'piaro/agentic-development-framework',

    [string]$InstallRoot = $(if ($env:ADF_INSTALL_ROOT) {
        $env:ADF_INSTALL_ROOT
    } else {
        Join-Path $env:LOCALAPPDATA 'Agentic'
    }),

    [string]$GitHubCli = $(if ($env:ADF_GH_CLI) {
        $env:ADF_GH_CLI
    } else {
        'gh'
    })
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($InstallRoot) -or
    [IO.Path]::GetFullPath($InstallRoot) -eq [IO.Path]::GetPathRoot($InstallRoot)) {
    throw '-InstallRoot must identify a dedicated directory.'
}
if (-not (Get-Command $GitHubCli -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI is required to verify binary provenance: $GitHubCli"
}
if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne
    [Runtime.InteropServices.Architecture]::X64) {
    throw 'No published Agentic binary is available for this Windows architecture.'
}

$target = 'x86_64-pc-windows-msvc'
$binary = "adf-$target.exe"
$buildRecord = "$binary.build.json"
$sourceRevision = (& $GitHubCli release view $Tag `
    --repo $Repository `
    --json targetCommitish `
    --jq '.targetCommitish').Trim()
if ($LASTEXITCODE -ne 0 -or $sourceRevision -notmatch '^[0-9a-f]{40}$') {
    throw 'Release target is not a 40-character lowercase Git commit SHA.'
}
$isDraft = (& $GitHubCli release view $Tag `
    --repo $Repository `
    --json isDraft `
    --jq '.isDraft').Trim()
if ($LASTEXITCODE -ne 0 -or $isDraft -ne 'false') {
    throw 'Refusing to install a draft GitHub Release.'
}
$defaultBranch = (& $GitHubCli repo view $Repository `
    --json defaultBranchRef `
    --jq '.defaultBranchRef.name').Trim()
if ($LASTEXITCODE -ne 0 -or
    $defaultBranch -notmatch '^[A-Za-z0-9][A-Za-z0-9._/-]*$') {
    throw 'Repository returned an invalid default branch.'
}

$staging = Join-Path ([IO.Path]::GetTempPath()) (
    'adf-bootstrap-' + [Guid]::NewGuid().ToString('N')
)
[IO.Directory]::CreateDirectory($staging) | Out-Null
try {
    & $GitHubCli release download $Tag `
        --repo $Repository `
        --dir $staging `
        --pattern $binary `
        --pattern $buildRecord `
        --pattern SHA256SUMS `
        --pattern publication-record.json `
        --pattern distribution-trust.json `
        --pattern candidate-framework.lock `
        --pattern framework-release.tar `
        --pattern publish-receipt.json
    if ($LASTEXITCODE -ne 0) {
        throw 'Downloading the Framework Release assets failed.'
    }

    # Checksums are checked again by the verified binary, but are not the
    # bootstrap trust root because an attacker could replace both files.
    & $GitHubCli attestation verify (Join-Path $staging $binary) `
        --repo $Repository `
        --signer-workflow "$Repository/.github/workflows/release.yml" `
        --source-digest $sourceRevision `
        --source-ref "refs/heads/$defaultBranch" `
        --deny-self-hosted-runners | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'GitHub Artifact Attestation verification failed.'
    }
    & $GitHubCli attestation verify (Join-Path $staging 'distribution-trust.json') `
        --repo $Repository `
        --signer-workflow "$Repository/.github/workflows/release.yml" `
        --source-digest $sourceRevision `
        --source-ref "refs/heads/$defaultBranch" `
        --deny-self-hosted-runners | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Distribution Trust Artifact Attestation verification failed.'
    }

    & (Join-Path $staging $binary) binary install $staging `
        --tag $Tag `
        --source-revision $sourceRevision `
        --install-root $InstallRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'Installing the verified Agentic binary failed.'
    }
} finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}

Write-Output "Add $(Join-Path $InstallRoot 'bin') to PATH to invoke adf."
Write-Output 'Then run: adf project init --project C:\path\to\project'
