{ pkgs }:

{
  # Packs an unpacked extension into a .crx that Chromium's ExtensionInstallForcelist policy can
  # install. `configure` runs inside the extension directory first, to fill in build-time values.
  # The extension id comes from the key generated while packing, so it is read back at evaluation
  # time (import from derivation).
  pack = { name, src, configure ? "" }:
    let
      extension = pkgs.stdenv.mkDerivation {
        pname = name;
        version = "2.0";
        inherit src;
        nativeBuildInputs = [ pkgs.chromium pkgs.jq pkgs.openssl ];
        dontBuild = true;
        installPhase = ''
          runHook preInstall
          mkdir -p $out
          export HOME=$TMPDIR

          cp -r $src ./extdir
          chmod -R u+w ./extdir

          pushd ./extdir
          ${configure}
          popd

          chromium --headless --disable-gpu --no-sandbox --pack-extension=./extdir
          mv ./extdir.crx $out/extension.crx

          openssl rsa -in ./extdir.pem -pubout -outform DER | \
            openssl dgst -sha256 -hex -r | \
            head -c32 | \
            tr '0123456789abcdef' 'abcdefghijklmnop' >$out/extension-id.txt

          runHook postInstall
        '';
      };

      id = builtins.readFile "${extension}/extension-id.txt";

      updateManifest = pkgs.writeText "${name}-update.xml" ''
        <?xml version="1.0" encoding="UTF-8"?>
        <gupdate xmlns="http://www.google.com/update2/response" protocol="2.0">
          <app appid="${id}">
            <updatecheck codebase="file://${extension}/extension.crx" version="2.0" />
          </app>
        </gupdate>
      '';
    in
    {
      inherit extension id;
      forcelistEntry = "${id};file://${updateManifest}";
    };
}
