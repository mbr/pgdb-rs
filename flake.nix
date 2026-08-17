{
  description = "Run temporary PostgreSQL instances";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-26.05";
    fenix = {
      url = "fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        buildToolchain = fenix.packages.${system}.stable.minimalToolchain;
        devToolchain = fenix.packages.${system}.stable.withComponents [
          "cargo"
          "clippy"
          "rust-analyzer"
          "rust-src"
          "rustc"
          "rustfmt"
        ];

        platform = pkgs.makeRustPlatform {
          cargo = buildToolchain;
          rustc = buildToolchain;
        };

        cargoToml = pkgs.lib.importTOML ./Cargo.toml;

        # Fenix's lld doesn't set RPATH; use wrapped lld for native deps.
        # This flag is also needed on macOS, but gated behind -Z unstable-options there.
        rustEnv = {
          RUSTFLAGS =
            pkgs.lib.optionalString pkgs.stdenv.isLinux "-Clink-self-contained=-linker "
            # Avoid runtime references from embedded toolchain source paths.
            + "--remap-path-prefix=${buildToolchain}=/rustc";
          OPENSSL_NO_VENDOR = "1";
        };
      in
      {
        packages.default = platform.buildRustPackage (
          rustEnv
          // rec {
            pname = "pgdb";
            version = cargoToml.workspace.package.version;
            nativeBuildInputs = with pkgs; [
              llvmPackages.bintools
              postgresql
            ];

            src = pkgs.lib.cleanSource ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            meta.mainProgram = pname;
          }
        );

        devShells.default = pkgs.mkShell (
          rustEnv
          // {
            inputsFrom = [ self.packages.${system}.default ];
            nativeBuildInputs = [
              devToolchain
              pkgs.nixfmt
            ];
            RUST_LOG = "debug";
          }
        );
      }
    );
}
