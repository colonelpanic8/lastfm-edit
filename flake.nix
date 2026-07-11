{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src" "rust-analyzer"];
        };
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs;
            [
              # Rust toolchain
              rustToolchain

              # System dependencies for reqwest/openssl
              pkg-config
              openssl
              curl

              # Additional useful tools
              cargo-watch
              cargo-edit
              just

              # Python development tools
              python3
              python3Packages.pip
              python3Packages.ipython
              maturin
              uv

              # For TUI development
              libiconv
            ]
            ++ lib.optionals stdenv.isLinux [
              # Dioxus desktop (webview) dependencies + dev tooling
              gtk3
              webkitgtk_4_1
              glib
              glib-networking
              libsoup_3
              libappindicator-gtk3
              xdotool
              dioxus-cli
            ]
            ++ lib.optionals stdenv.isDarwin [
              # macOS specific dependencies
              darwin.apple_sdk.frameworks.Security
              darwin.apple_sdk.frameworks.CoreFoundation
              darwin.apple_sdk.frameworks.SystemConfiguration
            ];

          # Environment variables
          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

          shellHook = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
            export LD_LIBRARY_PATH="${pkgs.libappindicator-gtk3}/lib:${pkgs.gtk3}/lib:$LD_LIBRARY_PATH"
            export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/"
          '';

          # For OpenSSL on some systems
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
        };

        packages.lastfm-edit = pkgs.rustPlatform.buildRustPackage {
          pname = "lastfm-edit";
          version = "7.0.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = with pkgs;
            [
              openssl
              curl
            ]
            ++ lib.optionals stdenv.isDarwin [
              darwin.apple_sdk.frameworks.Security
              darwin.apple_sdk.frameworks.CoreFoundation
              darwin.apple_sdk.frameworks.SystemConfiguration
            ];

          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

          # Skip tests in nix build (some tests require filesystem access)
          doCheck = false;
        };

        packages.scrobble-scrubber-app = pkgs.rustPlatform.buildRustPackage {
          pname = "scrobble-scrubber-app";
          version = "0.1.0";

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = ["-p" "scrobble-scrubber-app"];
          cargoTestFlags = ["-p" "scrobble-scrubber-app"];

          nativeBuildInputs = with pkgs; [
            makeWrapper
            pkg-config
          ];

          buildInputs = with pkgs;
            [
              openssl
              curl
            ]
            ++ lib.optionals stdenv.isLinux [
              gtk3
              webkitgtk_4_1
              glib
              glib-networking
              libsoup_3
            ];

          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

          postInstall = pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
            app="$out/Applications/Scrobble Scrubber.app"
            mkdir -p "$app/Contents/MacOS"
            ln -s "$out/bin/scrobble-scrubber-app" "$app/Contents/MacOS/scrobble-scrubber-app"
            cat > "$app/Contents/Info.plist" <<'EOF'
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
              <key>CFBundleDisplayName</key>
              <string>Scrobble Scrubber</string>
              <key>CFBundleExecutable</key>
              <string>scrobble-scrubber-app</string>
              <key>CFBundleIdentifier</key>
              <string>org.colonelpanic.scrobble-scrubber</string>
              <key>CFBundleName</key>
              <string>Scrobble Scrubber</string>
              <key>CFBundlePackageType</key>
              <string>APPL</string>
              <key>CFBundleShortVersionString</key>
              <string>0.1.0</string>
              <key>LSMinimumSystemVersion</key>
              <string>11.0</string>
            </dict>
            </plist>
            EOF
          '';

          postFixup = ''
            wrapProgram "$out/bin/scrobble-scrubber-app" \
              --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.pass pkgs.gnupg]}
          '';
        };

        packages.default = self.packages.${system}.lastfm-edit;
      }
    );
}
