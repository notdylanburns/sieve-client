{ pkgs, crane }:

let
  craneLib = crane.mkLib pkgs;

  desktopManagerArgs = {
    src = craneLib.cleanCargoSource ../sieve-desktop-manager;
    strictDeps = true;
    # aws-lc-sys, behind rustls, compiles its C sources with these.
    nativeBuildInputs = [ pkgs.cmake pkgs.perl ];
  };
in
{
  sieve-desktop-manager = craneLib.buildPackage (desktopManagerArgs // {
    cargoArtifacts = craneLib.buildDepsOnly desktopManagerArgs;
  });

  launcher = pkgs.buildGoModule {
    pname = "launcher";
    version = "1.0.0";
    src = ../launcher;
    vendorHash = null;
  };

  # The launcher runs nwg-drawer for the filtered user; its ':' command prompt would let them run
  # arbitrary commands, so it is patched out.
  nwg-drawer = pkgs.nwg-drawer.overrideAttrs (old: {
    postPatch = (old.postPatch or "") + ''
      substituteInPlace main.go --replace-fail "if s[0] == ':' {" "if false {"
      substituteInPlace uicomponents.go --replace-fail "if phrase[0] == ':' {" "if false {"
    '';
  });
}
