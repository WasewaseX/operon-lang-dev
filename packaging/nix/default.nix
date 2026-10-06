# W61 COMMUNITY DRAFT — Nix derivation for the release archives. Not
# CI-validated. Publish-time TODO: replace lib.fakeSha256 with the real
# hash from the release's companion .sha256 file, and extend the arch
# map as more release targets ship.
{
  lib,
  stdenv,
  fetchurl,
}:
let
  version = "2.9.0";
  archMap = {
    "x86_64-linux" = {
      target = "x86_64-unknown-linux-gnu";
      sha256 = lib.fakeSha256;
    };
    "aarch64-linux" = {
      target = "aarch64-unknown-linux-gnu";
      sha256 = lib.fakeSha256;
    };
  };
  arch = archMap.${stdenv.hostPlatform.system}
    or (throw "operon-lang: unsupported system ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "operon-lang";
  inherit version;

  src = fetchurl {
    url = "https://github.com/WasewaseX/operon-lang-dev/releases/download/v${version}/operon-${version}-${arch.target}.tar.gz";
    sha256 = arch.sha256;
  };

  # release archives keep the versioned top directory; strip it so the
  # output layout is flat: bin/operon, bin/operon-ls, std/
  sourceRoot = "operon-${version}";

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin $out/lib/operon $out/share/licenses/operon-lang
    install -m755 operon    $out/bin/operon
    install -m755 operon-ls $out/bin/operon-ls
    [ -d std ] && cp -r std $out/lib/operon/std
    [ -f LICENSE ] && install -m644 LICENSE $out/share/licenses/operon-lang/LICENSE
    runHook postInstall
  '';

  meta = with lib; {
    description = "The gene-expression language — Total Grammar toolchain (Rust core, C++ algorithm kernel)";
    homepage = "https://github.com/WasewaseX/operon-lang-dev";
    license = licenses.mit;
    platforms = builtins.attrNames archMap;
    mainProgram = "operon";
  };
}
