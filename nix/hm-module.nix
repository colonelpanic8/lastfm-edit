# Home-manager module exposing scrobble-scrubber as a user systemd service.
#
# `self` is the lastfm-edit flake, threaded in so the default package resolves
# to the build for the activating system.
self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.scrobble-scrubber;
in {
  options.services.scrobble-scrubber = {
    enable =
      lib.mkEnableOption "scrobble-scrubber last.fm scrobble metadata cleanup daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.scrobble-scrubber;
      defaultText =
        lib.literalExpression "lastfm-edit.packages.\${system}.scrobble-scrubber";
      description = "The scrobble-scrubber package to run.";
    };

    username = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "colonelpanic8";
      description = ''
        Last.fm username. Passed as --username and LASTFM_EDIT_USERNAME.
        When null, the CLI falls back to the config file / environment.

        Auth is a *persisted session*, not a password: log in once with the
        `lastfm-edit` CLI (handles MFA) to write
        ~/.local/share/lastfm-edit/users/<username>/session.json before enabling
        this service.
      '';
    };

    interval = lib.mkOption {
      type = lib.types.ints.positive;
      default = 300;
      description = "Seconds between sync+plan cycles in `run` mode.";
    };

    storeDir = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        scrobble-store root (--store-root). When null the CLI defaults to
        ~/.local/share/scrobble-store/<username>.
      '';
    };

    configFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Path to config.toml (--config). When null the CLI defaults to
        ~/.config/scrobble-scrubber/config.toml.
      '';
    };

    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = "RUST_LOG filter for the service.";
    };

    environment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      example = {
        LASTFM_EDIT_API_KEY = "...";
      };
      description = "Extra environment variables for the service.";
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Extra arguments appended to `scrobble-scrubber run`.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.scrobble-scrubber = {
      Unit = {
        Description =
          "scrobble-scrubber last.fm scrobble metadata cleanup daemon";
        Documentation = ["https://github.com/colonelpanic8/lastfm-edit"];
      };

      Service = {
        ExecStart = lib.escapeShellArgs (
          ["${cfg.package}/bin/scrobble-scrubber"]
          ++ lib.optionals (cfg.username != null) ["--username" cfg.username]
          ++ lib.optionals (cfg.storeDir != null) ["--store-root" (toString cfg.storeDir)]
          ++ lib.optionals (cfg.configFile != null) ["--config" (toString cfg.configFile)]
          ++ ["run" "--interval" (toString cfg.interval)]
          ++ cfg.extraArgs
        );
        Restart = "on-failure";
        RestartSec = 30;
        Environment =
          ["RUST_LOG=${cfg.logLevel}"]
          ++ lib.optional (cfg.username != null) "LASTFM_EDIT_USERNAME=${cfg.username}"
          ++ lib.mapAttrsToList (name: value: "${name}=${value}") cfg.environment;
      };

      Install = {
        WantedBy = ["default.target"];
      };
    };
  };
}
