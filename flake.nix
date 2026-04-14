{
  description = "opencode-occo: Patched opencode with OCCO provider and configurable plugins";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    opencode-upstream = {
      url = "github:anomalyco/opencode/v1.4.0";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      opencode-upstream,
    }:
    let
      # Only Linux targets — Darwin untested with these patches
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forEachSystem = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};

          # Rebuild node_modules with correct hash (upstream hashes.json can go stale)
          # Per-system hashes — auto-updated by .github/workflows/update-node-modules-hash.yml
          nodeModulesHashes = builtins.fromJSON (builtins.readFile ./nix/node-modules-hashes.json);
          node_modules = opencode-upstream.packages.${system}.opencode.node_modules.override {
            hash = nodeModulesHashes.${system};
          };

          # Patched opencode: upstream build + our occo patches + fixed node_modules
          opencode =
            (opencode-upstream.packages.${system}.opencode.override {
              inherit node_modules;
            }).overrideAttrs
              (old: {
                pname = "opencode-occo";
                patches = (old.patches or [ ]) ++ [ ./patches/opencode-occo-v1.4.0.patch ];
              });

          # Plugin derivations
          dcp = pkgs.callPackage ./nix/plugins/dcp.nix { };
          occo = pkgs.callPackage ./nix/plugins/occo.nix { src = self; };

          # Bundle: patched opencode + plugins with runtime symlink setup
          bundle = pkgs.callPackage ./nix/bundle.nix {
            inherit opencode;
            plugins = [
              dcp
              occo
            ];
          };
        in
        {
          default = bundle;
          inherit opencode dcp occo bundle;
        }
      );

      # Overlay for use in NixOS configurations
      overlays.default = _final: _prev: {
        opencode-occo = self.packages.${_prev.system}.default;
      };
    };
}
