[CmdletBinding()]
param()

# Run from an interactive Windows desktop; SSH Session 0 cannot verify focus.
$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $RepoRoot
try {
  cargo build --profile local-install --manifest-path src-tauri/Cargo.toml --features custom-protocol --example popup_visibility_regression
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

  # Tauri's build script embeds resources only into binaries, not Cargo examples.
  # The example also needs Common Controls v6 for Tauri's TaskDialogIndirect import.
  $ManifestTool = Get-Command mt.exe -ErrorAction SilentlyContinue
  if ($null -ne $ManifestTool) {
    $ManifestToolPath = $ManifestTool.Source
  } else {
    $SdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $ManifestToolPath = Get-ChildItem -LiteralPath $SdkRoot -Filter mt.exe -Recurse |
      Where-Object { $_.Directory.Name -eq "x64" } |
      Sort-Object FullName -Descending |
      Select-Object -First 1 -ExpandProperty FullName
  }
  if ([string]::IsNullOrWhiteSpace($ManifestToolPath)) {
    throw "Windows SDK manifest tool mt.exe is required"
  }
  $Binary = Join-Path $RepoRoot "src-tauri\target\local-install\examples\popup_visibility_regression.exe"
  $Manifest = Join-Path $RepoRoot "src-tauri\examples\popup_visibility_regression.manifest"
  & $ManifestToolPath -nologo -manifest $Manifest "-outputresource:$Binary;#1"
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

  & $Binary
  exit $LASTEXITCODE
} finally {
  Pop-Location
}
