# Bundle: patched opencode wrapped with plugin symlink management.
#
# On each invocation the wrapper:
#   1. Creates ~/.local/share/opencode-occo/plugins/ (respects XDG_DATA_HOME)
#   2. Symlinks each plugin from the Nix store into that directory
#   3. Execs the real opencode binary
#
# The user configures .opencode.json once with stable file:// paths:
#   {
#     "plugins": [
#       "file:///home/<user>/.local/share/opencode-occo/plugins/opencode-dcp",
#       "file:///home/<user>/.local/share/opencode-occo/plugins/occo"
#     ]
#   }
#
# On rebuild (e.g. nix profile upgrade), the wrapper updates the symlinks
# automatically — no .opencode.json changes needed.
#
# Run `opencode-occo-setup` to print the correct config snippet.
{
  lib,
  symlinkJoin,
  makeWrapper,
  writeShellScriptBin,
  opencode,
  plugins ? [ ],
}:

let
  # Generate ln commands for each plugin
  pluginLinkCommands = lib.concatMapStringsSep "\n" (
    p:
    let
      name = p.pluginName or p.pname;
    in
    ''ln -sfn ${p} "$_occo_plugin_dir/${name}"''
  ) plugins;

  # Helper script: prints .opencode.json plugin config with resolved paths
  setupScript = writeShellScriptBin "opencode-occo-setup" ''
    _dir="''${XDG_DATA_HOME:-$HOME/.local/share}/opencode-occo/plugins"

    echo "opencode-occo plugin setup"
    echo ""
    echo "Plugin directory: $_dir"
    echo ""
    echo "Add the following to your .opencode.json (project root or global):"
    echo ""
    echo '{'
    echo '  "plugins": ['
    ${lib.concatStringsSep "\n" (lib.imap1 (
      i: p:
      let
        name = p.pluginName or p.pname;
        comma = if i < builtins.length plugins then "," else "";
      in
      ''echo "    \"file://$_dir/${name}\"${comma}"''
    ) plugins)}
    echo '  ]'
    echo '}'
  '';

in
symlinkJoin {
  name = "opencode-occo-${opencode.version}";
  paths = [
    opencode
    setupScript
  ];

  nativeBuildInputs = [ makeWrapper ];

  postBuild = ''
    # Replace the opencode symlink with a wrapper that manages plugin symlinks
    wrapProgram $out/bin/opencode \
      --run '
        _occo_plugin_dir="''${XDG_DATA_HOME:-$HOME/.local/share}/opencode-occo/plugins"
        mkdir -p "$_occo_plugin_dir"
        ${pluginLinkCommands}
      '
  '';

  passthru = {
    inherit plugins opencode;
  };

  meta = (opencode.meta or { }) // {
    description = "opencode with OCCO provider patches and bundled plugins";
    mainProgram = "opencode";
  };
}
