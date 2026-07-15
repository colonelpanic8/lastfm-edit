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
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };

        lib = pkgs.lib;
        isLinux = pkgs.stdenv.hostPlatform.isLinux;
        isDarwin = pkgs.stdenv.hostPlatform.isDarwin;
        appVersion = (builtins.fromTOML (builtins.readFile ./crates/scrobble-scrubber-app/Cargo.toml)).package.version;
        appVersionParts = map builtins.fromJSON (lib.splitString "." appVersion);
        androidVersionCode = builtins.toString (
          (builtins.elemAt appVersionParts 0)
          * 1000000
          + (builtins.elemAt appVersionParts 1) * 1000
          + builtins.elemAt appVersionParts 2
        );

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src" "rust-analyzer"];
          targets = lib.optionals isLinux [
            "aarch64-linux-android"
            "x86_64-linux-android"
          ];
        };

        androidBuildToolsVersion = "36.1.0";
        androidCmdLineToolsVersion = "19.0";
        androidCompileSdkVersion = "36";
        androidGradlePluginVersion = "8.13.2";
        androidKotlinPluginVersion = "2.2.21";
        androidNdkVersion = "29.0.14206865";
        androidPlatformToolsVersion = "36.0.2";
        androidTargetSdkVersion = "36";
        androidComposition = pkgs.androidenv.composeAndroidPackages {
          cmdLineToolsVersion = androidCmdLineToolsVersion;
          platformToolsVersion = androidPlatformToolsVersion;
          buildToolsVersions = ["34.0.0" androidBuildToolsVersion];
          platformVersions = ["33" "34" androidCompileSdkVersion];
          includeEmulator = false;
          includeSources = false;
          includeSystemImages = false;
          includeNDK = true;
          ndkVersions = [androidNdkVersion];
          cmakeVersions = ["3.22.1"];
        };
        androidHome = "${androidComposition.androidsdk}/libexec/android-sdk";
        androidNdkHome = "${androidHome}/ndk/${androidNdkVersion}";
        androidAapt2 = "${androidHome}/build-tools/${androidBuildToolsVersion}/aapt2";
        androidLlvmBin = "${androidNdkHome}/toolchains/llvm/prebuilt/linux-x86_64/bin";
        androidPageSizeRustFlags = "-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384";
        dioxusAndroidEnv = {
          ANDROID_HOME = androidHome;
          ANDROID_SDK_ROOT = androidHome;
          ANDROID_NDK_HOME = androidNdkHome;
          NDK_HOME = androidNdkHome;
          JAVA_HOME = pkgs.jdk17.home;
          CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = "${androidLlvmBin}/aarch64-linux-android24-clang";
          CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS = androidPageSizeRustFlags;
          CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER = "${androidLlvmBin}/x86_64-linux-android24-clang";
          CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS = androidPageSizeRustFlags;
          CC_aarch64_linux_android = "${androidLlvmBin}/aarch64-linux-android24-clang";
          CC_x86_64_linux_android = "${androidLlvmBin}/x86_64-linux-android24-clang";
          AR_aarch64_linux_android = "${androidLlvmBin}/llvm-ar";
          AR_x86_64_linux_android = "${androidLlvmBin}/llvm-ar";
          GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidAapt2}";
        };
        dioxusAndroidBuildScript = release:
          pkgs.writeShellApplication {
            name = "scrobble-scrubber-android-${
              if release
              then "release"
              else "debug"
            }";
            runtimeInputs = [pkgs.coreutils pkgs.findutils pkgs.gnused pkgs.nix];
            text = ''
              set -euo pipefail

              repo="''${LASTFM_EDIT_ROOT:-$PWD}"
              cd "$repo"
              profile=${
                if release
                then "release"
                else "debug"
              }

              args=(
                dx build --android
                --target aarch64-linux-android
                --package scrobble-scrubber-app
                --no-default-features
                --features android
                --locked
              )
              ${lib.optionalString release ''args+=(--release)''}

              patch_android_project() {
                local gradle_root="target/dx/scrobble-scrubber-app/$profile/android/app"
                local root_gradle="$gradle_root/build.gradle.kts"
                local app_gradle="$gradle_root/app/build.gradle.kts"
                local gradle_properties="$gradle_root/gradle.properties"
                local manifest="$gradle_root/app/src/main/AndroidManifest.xml"

                if [[ -f "$root_gradle" ]]; then
                  sed -i \
                    -e 's/com\.android\.tools\.build:gradle:[^"]*/com.android.tools.build:gradle:${androidGradlePluginVersion}/' \
                    -e 's/org\.jetbrains\.kotlin:kotlin-gradle-plugin:[^"]*/org.jetbrains.kotlin:kotlin-gradle-plugin:${androidKotlinPluginVersion}/' \
                    "$root_gradle"
                fi

                if [[ -f "$app_gradle" ]]; then
                  sed -i \
                    -e 's/compileSdk = [0-9][0-9]*/compileSdk = ${androidCompileSdkVersion}\n    buildToolsVersion = "${androidBuildToolsVersion}"/' \
                    -e 's/targetSdk = [0-9][0-9]*/targetSdk = ${androidTargetSdkVersion}/' \
                    -e 's/versionCode = [0-9][0-9]*/versionCode = ${androidVersionCode}/' \
                    -e 's/versionName = "[^"]*"/versionName = "${appVersion}"/' \
                    -e '/^[[:space:]]*kotlinOptions[[:space:]]*{/,/^[[:space:]]*}/c\    kotlin {\n        compilerOptions {\n            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)\n        }\n    }' \
                    "$app_gradle"
                fi

                if [[ -f "$gradle_properties" ]]; then
                  sed -i '/^android\.defaults\.buildfeatures\.buildconfig=/d' "$gradle_properties"
                fi
                if [[ -f "$manifest" ]]; then
                  sed -i '/android:extractNativeLibs=/d' "$manifest"
                fi
              }

              sign_release_artifacts() {
                if [[ -z "''${ANDROID_SIGNING_KEYSTORE_BASE64:-}" && -z "''${ANDROID_SIGNING_KEYSTORE_FILE:-}" ]]; then
                  echo "Android release signing skipped: no signing keystore was provided"
                  return
                fi

                local required=(
                  ANDROID_SIGNING_KEY_ALIAS
                  ANDROID_SIGNING_KEYSTORE_PASSWORD
                  ANDROID_SIGNING_KEY_PASSWORD
                )
                for var in "''${required[@]}"; do
                  if [[ -z "''${!var:-}" ]]; then
                    echo "Android release signing requires $var" >&2
                    exit 1
                  fi
                done

                local signing_dir
                signing_dir="$(mktemp -d)"
                trap 'rm -rf "$signing_dir"' RETURN
                local keystore="$signing_dir/scrobble-scrubber-release.keystore"
                if [[ -n "''${ANDROID_SIGNING_KEYSTORE_FILE:-}" ]]; then
                  cp "$ANDROID_SIGNING_KEYSTORE_FILE" "$keystore"
                else
                  printf '%s' "$ANDROID_SIGNING_KEYSTORE_BASE64" | base64 -d > "$keystore"
                fi

                local apk_dir="target/dx/scrobble-scrubber-app/release/android/app/app/build/outputs/apk/release"
                shopt -s nullglob
                local unsigned_apks=("$apk_dir"/*-unsigned.apk)
                shopt -u nullglob
                if ((''${#unsigned_apks[@]} == 0)); then
                  echo "No unsigned release APKs were found in $apk_dir" >&2
                  exit 1
                fi

                for unsigned_apk in "''${unsigned_apks[@]}"; do
                  local apk_base="''${unsigned_apk%-unsigned.apk}"
                  local aligned_apk
                  aligned_apk="$signing_dir/$(basename "$apk_base")-aligned.apk"
                  local signed_apk="$apk_base-signed.apk"
                  # shellcheck disable=SC2016
                  nix develop "$repo#android" --command bash -lc '
                    set -euo pipefail
                    "$ANDROID_HOME/build-tools/${androidBuildToolsVersion}/zipalign" -p -f 4 "$1" "$2"
                    "$ANDROID_HOME/build-tools/${androidBuildToolsVersion}/apksigner" sign \
                      --ks "$4" \
                      --ks-key-alias "$5" \
                      --ks-pass env:ANDROID_SIGNING_KEYSTORE_PASSWORD \
                      --key-pass env:ANDROID_SIGNING_KEY_PASSWORD \
                      --out "$3" \
                      "$2"
                    "$ANDROID_HOME/build-tools/${androidBuildToolsVersion}/apksigner" verify --verbose "$3"
                  ' bash \
                    "$unsigned_apk" \
                    "$aligned_apk" \
                    "$signed_apk" \
                    "$keystore" \
                    "$ANDROID_SIGNING_KEY_ALIAS"
                done

                local aab_dir="target/dx/scrobble-scrubber-app/release/android/app/app/build/outputs/bundle/release"
                local unsigned_aab="$aab_dir/app-release.aab"
                local signed_aab="$aab_dir/app-release-signed.aab"
                if [[ ! -f "$unsigned_aab" ]]; then
                  echo "No release AAB was found at $unsigned_aab" >&2
                  exit 1
                fi
                nix develop "$repo#android" --command jarsigner \
                  -keystore "$keystore" \
                  -storepass:env ANDROID_SIGNING_KEYSTORE_PASSWORD \
                  -keypass:env ANDROID_SIGNING_KEY_PASSWORD \
                  -signedjar "$signed_aab" \
                  "$unsigned_aab" \
                  "$ANDROID_SIGNING_KEY_ALIAS"
                nix develop "$repo#android" --command jarsigner \
                  -verify "$signed_aab"
              }

              rm -rf "target/dx/scrobble-scrubber-app/$profile/android"
              nix develop "$repo#android" --command "''${args[@]}" "$@"
              patch_android_project

              if ${
                if release
                then "true"
                else "false"
              }; then
                nix develop "$repo#android" --command bash -lc \
                  'cd target/dx/scrobble-scrubber-app/release/android/app && ./gradlew :app:bundleRelease :app:assembleRelease --no-daemon --console plain'
                sign_release_artifacts
                find "$repo/target/dx/scrobble-scrubber-app/release/android" \
                  \( -path '*/build/outputs/apk/release/*.apk' -o -path '*/build/outputs/bundle/release/*.aab' \) \
                  -type f -print
              else
                nix develop "$repo#android" --command bash -lc \
                  'cd target/dx/scrobble-scrubber-app/debug/android/app && ./gradlew :app:assembleDebug --no-daemon --console plain'
                find "$repo/target/dx/scrobble-scrubber-app/debug/android" \
                  -path '*/build/outputs/apk/debug/*.apk' \
                  -type f -print
              fi
            '';
          };
        dioxusDesktopBuildScript = pkgs.writeShellApplication {
          name = "scrobble-scrubber-desktop-release";
          runtimeInputs = [pkgs.coreutils pkgs.findutils pkgs.nix];
          text = ''
            set -euo pipefail

            repo="''${LASTFM_EDIT_ROOT:-$PWD}"
            out_dir="''${SCROBBLE_SCRUBBER_DESKTOP_OUT_DIR:-$repo/target/release-artifacts/desktop}"
            cd "$repo"
            rm -rf "$out_dir"
            mkdir -p "$out_dir"

            nix develop "$repo" --command dx bundle \
              ${
              if isDarwin
              then "--macos"
              else "--desktop"
            } \
              --package-types ${
              if isDarwin
              then "macos"
              else "deb"
            } \
              --out-dir "$out_dir" \
              --package scrobble-scrubber-app \
              --no-default-features \
              --features desktop \
              --release \
              --locked \
              "$@"

            find "$out_dir" -type f -print
          '';
        };

        rustSource = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type: let
            relPath = pkgs.lib.removePrefix "${toString ./.}/" (toString path);
            firstComponent = builtins.head (pkgs.lib.splitString "/" relPath);
            baseName = baseNameOf path;
          in
            pkgs.lib.cleanSourceFilter path type
            && !(builtins.elem firstComponent [
              ".direnv"
              ".git"
              ".github"
              "coverage"
              "python"
              "target"
            ])
            && !(builtins.elem baseName [
              ".envrc"
              "result"
            ])
            && !(pkgs.lib.hasPrefix "result-" baseName);
        };
      in {
        devShells =
          {
            default = pkgs.mkShell {
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
          }
          // lib.optionalAttrs isLinux {
            android = pkgs.mkShell (dioxusAndroidEnv
              // {
                buildInputs = with pkgs; [
                  rustToolchain
                  dioxus-cli
                  jdk17
                  gradle_9
                  cmake
                  gnumake
                  perl
                  pkg-config
                  just
                ];

                shellHook = ''
                  export PATH=${androidHome}/platform-tools:${androidHome}/cmdline-tools/${androidCmdLineToolsVersion}/bin:$PATH
                '';
              });
          };

        apps =
          lib.optionalAttrs (isLinux || isDarwin) {
            dioxus-desktop-release = {
              type = "app";
              program = "${dioxusDesktopBuildScript}/bin/scrobble-scrubber-desktop-release";
            };
            scrobble-scrubber-desktop-release = {
              type = "app";
              program = "${dioxusDesktopBuildScript}/bin/scrobble-scrubber-desktop-release";
            };
          }
          // lib.optionalAttrs isLinux {
            dioxus-android-debug = {
              type = "app";
              program = "${dioxusAndroidBuildScript false}/bin/scrobble-scrubber-android-debug";
            };
            dioxus-android-release = {
              type = "app";
              program = "${dioxusAndroidBuildScript true}/bin/scrobble-scrubber-android-release";
            };
            scrobble-scrubber-android-debug = {
              type = "app";
              program = "${dioxusAndroidBuildScript false}/bin/scrobble-scrubber-android-debug";
            };
            scrobble-scrubber-android-release = {
              type = "app";
              program = "${dioxusAndroidBuildScript true}/bin/scrobble-scrubber-android-release";
            };
          };

        packages.lastfm-edit = pkgs.rustPlatform.buildRustPackage {
          pname = "lastfm-edit";
          version = "7.0.1";

          src = rustSource;

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

        packages.scrobble-store = pkgs.rustPlatform.buildRustPackage {
          pname = "scrobble-store";
          version = "0.1.2";

          src = rustSource;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          cargoBuildFlags = ["-p" "scrobble-store"];

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

        packages.scrobble-scrubber = pkgs.rustPlatform.buildRustPackage {
          pname = "scrobble-scrubber";
          version = "0.1.2";

          src = rustSource;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          cargoBuildFlags = ["-p" "scrobble-scrubber"];

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
          version = appVersion;

          src = rustSource;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          cargoBuildFlags = ["-p" "scrobble-scrubber-app"];

          nativeBuildInputs = with pkgs;
            [
              makeWrapper
              pkg-config
            ]
            ++ lib.optionals stdenv.isLinux [
              copyDesktopItems
              wrapGAppsHook3
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
              libappindicator-gtk3
              xdotool
            ]
            ++ lib.optionals stdenv.isDarwin [
              darwin.apple_sdk.frameworks.Security
              darwin.apple_sdk.frameworks.CoreFoundation
              darwin.apple_sdk.frameworks.SystemConfiguration
            ];

          desktopItems = pkgs.lib.optionals pkgs.stdenv.isLinux [
            (pkgs.makeDesktopItem {
              name = "scrobble-scrubber";
              desktopName = "Scrobble Scrubber";
              genericName = "Last.fm Scrobble Editor";
              comment = "Review and drive Last.fm scrobble metadata cleanup";
              exec = "scrobble-scrubber-app";
              icon = "audio-x-generic";
              terminal = false;
              categories = ["AudioVideo" "Audio"];
              startupNotify = true;
            })
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
              <string>org.colonelpanic.scrobblescrubber</string>
              <key>CFBundleName</key>
              <string>Scrobble Scrubber</string>
              <key>CFBundlePackageType</key>
              <string>APPL</string>
              <key>CFBundleShortVersionString</key>
              <string>${appVersion}</string>
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

          # Skip tests in nix build (some tests require filesystem access)
          doCheck = false;
        };

        packages.default = self.packages.${system}.lastfm-edit;
      }
    )
    // {
      # System-independent outputs (modules) live outside eachDefaultSystem.
      homeManagerModules.scrobble-scrubber = import ./nix/hm-module.nix self;
      homeManagerModules.default = self.homeManagerModules.scrobble-scrubber;
    };
}
