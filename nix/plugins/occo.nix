# occo plugin: GitHub Copilot OAuth provider for opencode
# This is a local plugin (not published to npm) — packaged directly from source.
# No build step needed: pure ES module (index.mjs).
{
  lib,
  stdenvNoCC,
  src,
}:

stdenvNoCC.mkDerivation {
  pname = "occo";
  version = "0.1.0";

  inherit src;

  # Plugin name used by the bundle wrapper for symlink naming
  passthru.pluginName = "occo";

  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp index.mjs package.json $out/
    runHook postInstall
  '';
}
