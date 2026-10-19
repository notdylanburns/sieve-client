# Sieve client

A flake with a NixOS module that sets a machine up for one filtered user:

- **Launcher:** a drawer showing only the workspace's approved sites and videos.
- **Desktop manager:** keeps those launchers up to date over the server's events WebSocket.
- **Chromium:** goes through the Sieve proxy, with the proxy's CA trusted for that user only and
  locked-down browser policies.
- **Admin bar:** an optional extension that lets someone signed in to the Sieve console approve or
  block the site they're on.

It talks to a [Sieve server](../server) over HTTPS and doesn't share code with it.

## Using it

Add the flake as an input and enable the module:

```nix
{
  inputs.sieve-client.url = "git+https://github.com/notdylanburns/sieve-client.git";

  outputs = { nixpkgs, sieve-client, ... }: {
    nixosConfigurations.laptop = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        sieve-client.nixosModules.default
        {
          services.sieve.client = {
            enable = true;
            server = "https://sieve.example.com";
            workspace = "7e2d99a5-6a8f-4937-9fa6-586629dd6ab5";
            user = "amanda";
          };
        }
      ];
    };
  };
}
```

Then enroll the device (the server's **Devices** page, or `sieve device create`) and put its
credentials on the machine by hand. Neither file goes through the Nix store:

```sh
sudo install -d -m 0755 /var/lib/sieve
# The desktop manager runs as the filtered user, so they need to read the token.
echo '<device token>' | sudo install -m 0640 -g amanda /dev/stdin /var/lib/sieve/device-token
echo '<proxy username>:<proxy password>' | sudo install -m 0600 /dev/stdin /var/lib/sieve/proxy-credentials
```

Paths can be changed with `deviceTokenFile` and `proxyCredentialsFile`. Changing the
credentials file is picked up automatically; the token is read when the desktop manager starts.

| Option | Default | |
|---|---|---|
| `server` | — | Base URL of the Sieve server |
| `workspace` | — | Workspace id the device is enrolled in |
| `user` | — | The filtered user |
| `proxyPort` | `3129` | The server's TLS-wrapped proxy port |
| `deviceTokenFile` | `/var/lib/sieve/device-token` | |
| `proxyCredentialsFile` | `/var/lib/sieve/proxy-credentials` | `username:password` on one line |
| `adminExtension.enable` | `true` | Install the admin bar |

## What runs

| Unit | Kind | Does |
|---|---|---|
| `sieve-desktop-manager` | user service, filtered user | Follows `/api/<workspace>/events`, rewrites launcher entries and icons from `/applications` |
| `launcher` | user service, filtered user | The drawer; start it from the session (e.g. labwc autostart) |
| `install-chromium-ca` | user service, filtered user | Downloads `/api/proxy-ca.crt` and trusts it in the user's NSS database; retries every 30 s if offline |
| `sieve-proxy-credentials` | system service + path unit | Writes the proxy credentials into Chromium managed policy for the proxy extension |

Chromium apps launch with `--proxy-server=https://<server>:<proxyPort>`, and the proxy extension
answers the proxy's login prompt. Videos play in mpv with `/etc/sieve/mpv`.

## Security notes

- The filtered user can read the device token and the proxy credentials (Chromium needs the
  latter). That only grants what their workspace allows. What actually enforces filtering is
  that this user can't reach the internet except through the proxy: add a firewall rule that
  blocks their direct outbound traffic.
- Chromium policy is machine-wide, so every user gets the locked-down settings and extensions.
- The proxy CA is trusted only in the filtered user's NSS database, not system-wide.

## Development

```sh
nix build .#sieve-desktop-manager .#launcher
(cd sieve-desktop-manager && cargo test)
```

Run the desktop manager against a server by hand:

```sh
SIEVE_SERVER=https://sieve.example.com SIEVE_WORKSPACE=<id> SIEVE_DEVICE_TOKEN_FILE=./token \
  SIEVE_PROXY=https://sieve.example.com:3129 cargo run
```

The admin extension's `config.js` and manifest host permission are filled in at build time; the
committed values point at a local development server for loading it unpacked.
