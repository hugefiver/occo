# DCP (Dynamic Context Pruning) plugin for opencode
# https://github.com/Opencode-DCP/opencode-dynamic-context-pruning
#
# Uses buildNpmPackage from nixpkgs for proper npm sandbox handling.
# Build: npm ci → tsc → dist/
#
# To update npmDepsHash: set to lib.fakeHash, build, replace with printed hash.
{
  lib,
  buildNpmPackage,
  fetchFromGitHub,
}:

buildNpmPackage {
  pname = "opencode-dcp";
  version = "3.1.9";

  src = fetchFromGitHub {
    owner = "Opencode-DCP";
    repo = "opencode-dynamic-context-pruning";
    # If this tag doesn't exist, check:
    #   https://github.com/Opencode-DCP/opencode-dynamic-context-pruning/tags
    # and replace with the correct tag or commit hash.
    rev = "v3.1.9";
    hash = "sha256-a5WrJ6OWgrF/fNmo7Dq6TiyJcsU+utbH5NaCP6wsJFk=";
  };

  npmDepsHash = "sha256-fkvUA81Amp3MhnLEe5+eEnM4s9HGrFxRGkrmmtGnTro=";

  # Plugin name used by the bundle wrapper for symlink naming
  passthru.pluginName = "opencode-dcp";

  # tsc is in devDependencies; npm ci installs it
  buildPhase = ''
    runHook preBuild
    npx tsc
    runHook postBuild
  '';

  # Install as a loadable plugin directory (not a global npm package)
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r dist package.json node_modules $out/
    runHook postInstall
  '';

  # Don't try to run the default npm build/install
  dontNpmBuild = true;
  dontNpmInstall = true;
}
