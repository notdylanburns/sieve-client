{
  description = "Sieve client: launcher, browser and proxy integration for a filtered NixOS user";

  inputs = {
    # The admin extension is a submodule, which flakes leave out unless asked.
    self.submodules = true;
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs:
        let packages = import ./nix/packages.nix { inherit pkgs crane; };
        in packages // { default = packages.sieve-desktop-manager; });

      nixosModules.default = import ./nix/module.nix self;

      checks = forAllSystems (pkgs: {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) sieve-desktop-manager launcher;
      });
    };
}
