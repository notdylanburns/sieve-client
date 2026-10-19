self:
{ config, lib, pkgs, ... }:

let
  inherit (lib) mkEnableOption mkIf mkOption optional types;

  cfg = config.services.sieve.client;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};
  extensions = import ./extensions.nix { inherit pkgs; };

  serverHost = builtins.head (builtins.match "https?://([^/:]+).*" cfg.server);
  proxyUrl = "https://${serverHost}:${toString cfg.proxyPort}";
  mpvConfigDir = "/etc/sieve/mpv";
  caName = "Sieve Proxy CA";

  proxyExtension = extensions.pack {
    name = "sieve-proxy-extension";
    src = ../extensions/proxy;
  };

  adminExtension = extensions.pack {
    name = "sieve-admin-extension";
    src = ../extensions/admin;
    configure = ''
      cat > config.js <<'EOF'
      const SIEVE_SERVER = ${builtins.toJSON cfg.server};
      const SIEVE_WORKSPACE = ${builtins.toJSON cfg.workspace};
      EOF
      jq --arg origin ${lib.escapeShellArg "${cfg.server}/*"} \
         --arg console ${lib.escapeShellArg "*://${serverHost}/*"} \
         '.host_permissions = [$origin] | .content_scripts[0].exclude_matches = [$console]' \
         manifest.json > manifest.json.new
      mv manifest.json.new manifest.json
    '';
  };
in
{
  options.services.sieve.client = {
    enable = mkEnableOption "the Sieve client for a filtered user";

    server = mkOption {
      type = types.strMatching "https?://[^/]+";
      example = "https://sieve.example.com";
      description = "Base URL of the Sieve server, without a trailing slash.";
    };

    workspace = mkOption {
      type = types.strMatching "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
      description = "Id of the workspace this device is enrolled in.";
    };

    user = mkOption {
      type = types.str;
      example = "amanda";
      description = ''
        The filtered user. Their launcher shows only the workspace's approved sites and
        videos, and their browser goes through the Sieve proxy.
      '';
    };

    proxyPort = mkOption {
      type = types.port;
      default = 3129;
      description = "Port of the server's TLS-wrapped proxy listener.";
    };

    deviceTokenFile = mkOption {
      type = types.str;
      default = "/var/lib/sieve/device-token";
      description = ''
        File holding the device token from enrolling this device. Placed by hand, outside
        the Nix store, and readable by the filtered user.
      '';
    };

    proxyCredentialsFile = mkOption {
      type = types.str;
      default = "/var/lib/sieve/proxy-credentials";
      description = ''
        File holding the device's proxy credentials as `username:password` on one line.
        Placed by hand, outside the Nix store; root reads it.
      '';
    };

    adminExtension.enable = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Install the admin bar, which lets someone signed in to the Sieve console approve or
        block the site they are on. Chromium policy is machine-wide, but the bar only appears
        with a valid console session, which the filtered user doesn't have.
      '';
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [
      packages.launcher
      # The launcher runs /run/current-system/sw/bin/nwg-drawer, so this must be the patched one.
      packages.nwg-drawer
      pkgs.adwaita-icon-theme
      pkgs.chromium
      pkgs.ffmpeg
      pkgs.hicolor-icon-theme
      pkgs.mpv
      pkgs.yt-dlp
    ];

    programs.chromium = {
      enable = true;
      extraOpts = {
        ExtensionInstallBlocklist = [ "*" ];
        ExtensionInstallForcelist =
          [ proxyExtension.forcelistEntry ]
          ++ optional cfg.adminExtension.enable adminExtension.forcelistEntry;
        BlockExternalExtensions = true;

        DeveloperToolsAvailability = 2;
        IncognitoModeAvailability = 1;
        BrowserGuestModeEnabled = false;
        BrowserAddPersonEnabled = false;
        BrowserSignin = 0;
        SyncDisabled = true;

        DownloadRestrictions = 3;
        AllowFileSelectionDialogs = false;
        PrintingEnabled = false;
        ExternalProtocolDialogShowAlwaysOpenCheckbox = false;

        URLBlocklist = [
          "file://*"
          "data:*"
          "javascript://*"
          "view-source:*"
          "filesystem:*"
          "chrome://*"
        ];
      };
    };

    environment.etc."chromium/policies/managed/sieve-proxy-credentials.json".source = cfg.proxyCredentialsFile;

    # The proxy inspects HTTPS, so the filtered user's browser must trust its CA. Only their
    # NSS database gets it; trusting it system-wide would let the CA impersonate sites to
    # everyone on the machine.
    systemd.user.services.install-chromium-ca = {
      description = "Install the Sieve proxy CA into Chromium's certificate store";
      wantedBy = [ "default.target" ];
      unitConfig = {
        ConditionUser = cfg.user;
        StartLimitIntervalSec = 0;
      };
      path = [ pkgs.coreutils pkgs.curl pkgs.nssTools pkgs.openssl ];
      serviceConfig = {
        Type = "oneshot";
        # Offline at login: keep whatever is installed and try again shortly.
        Restart = "on-failure";
        RestartSec = 30;
      };
      script = ''
        set -euo pipefail

        ca=$(mktemp)
        trap 'rm -f "$ca"' EXIT
        curl -fsS --max-time 30 -o "$ca" ${lib.escapeShellArg "${cfg.server}/api/proxy-ca.crt"}
        openssl x509 -in "$ca" -noout

        db="$HOME/.local/share/pki/nssdb"
        mkdir -p "$db"
        if [ ! -f "$db/pkcs11.txt" ]; then
          certutil -N -d "sql:$db" --empty-password
        fi

        for name in ${lib.escapeShellArg caName} "Squid Proxy CA"; do
          certutil -D -n "$name" -d "sql:$db" 2>/dev/null || true
        done
        certutil -A -n ${lib.escapeShellArg caName} -t "TCu,cu,cu" -i "$ca" -d "sql:$db"
      '';
    };

    systemd.user.services.sieve-desktop-manager = {
      description = "Keep the launcher in step with the Sieve workspace";
      wantedBy = [ "default.target" ];
      unitConfig = {
        ConditionUser = cfg.user;
        StartLimitIntervalSec = 0;
      };
      environment = {
        SIEVE_SERVER = cfg.server;
        SIEVE_WORKSPACE = cfg.workspace;
        SIEVE_DEVICE_TOKEN_FILE = cfg.deviceTokenFile;
        SIEVE_PROXY = proxyUrl;
        CHROMIUM_BIN = "${pkgs.chromium}/bin/chromium";
        LAUNCHER_BIN = "${packages.launcher}/bin/launcher";
        MPV_BIN = "${pkgs.mpv}/bin/mpv";
        MPV_CONFIG_DIR = mpvConfigDir;
        SHELL_BIN = "${pkgs.bash}/bin/bash";
      };
      serviceConfig = {
        ExecStart = "${packages.sieve-desktop-manager}/bin/sieve-desktop-manager";
        Restart = "always";
        RestartSec = 5;
      };
    };

    systemd.user.services.launcher = {
      description = "Application launcher";
      wants = [ "sieve-desktop-manager.service" ];
      after = [ "sieve-desktop-manager.service" ];
      unitConfig.ConditionUser = cfg.user;
      environment.XDG_DATA_DIRS = lib.concatStringsSep ":" [
        "%h/.local/share/sieve/desktop-manager"
        "${pkgs.adwaita-icon-theme}/share"
        "${pkgs.hicolor-icon-theme}/share"
      ];
      serviceConfig = {
        # nwg-drawer's logout button runs `logout` from PATH.
        ExecStart = "${pkgs.bash}/bin/bash -c ${lib.escapeShellArg
          "PATH=${pkgs.writeScriptBin "logout" "${packages.launcher}/bin/launcher logout"}/bin:$PATH exec ${packages.launcher}/bin/launcher"}";
        Restart = "always";
        RestartSec = 0;
      };
    };

    environment.etc."sieve/mpv/mpv.conf".text = ''
      load-scripts=no
      load-auto-profiles=no
      load-commands=no
      load-console=no
      load-context-menu=no
      load-positioning=no
      load-select=no
      load-stats-overlay=no
      input-default-bindings=no
      input-builtin-bindings=no
      drag-and-drop=no
      ytdl-raw-options-append=ignore-config=
      ytdl-raw-options-append=no-plugin-dirs=

      hwdec=auto-safe
      ytdl-format=bestvideo[height<=?1080][fps<=?30][vcodec^=avc1]+bestaudio/bestvideo[height<=?1080][fps<=?30]+bestaudio/best[height<=?1080]/best
    '';

    environment.etc."sieve/mpv/input.conf".text = ''
      CLOSE_WIN quit
      q quit
      ESC set fullscreen no
      f cycle fullscreen
      MBTN_LEFT_DBL cycle fullscreen
      SPACE cycle pause
      p cycle pause
      MBTN_RIGHT cycle pause
      LEFT seek -5
      RIGHT seek 5
      UP add volume 5
      DOWN add volume -5
      WHEEL_UP add volume 5
      WHEEL_DOWN add volume -5
      m cycle mute
    '';
  };
}
