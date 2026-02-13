{
  description = "Run temporary PostgreSQL instances";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-25.05";
    fenix = {
      url = "fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "flake-utils";
  };

  outputs = { self, nixpkgs, fenix, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        
        toolchain = fenix.packages.${system}.stable.withComponents [
          "cargo"
          "clippy"
          "rust-src"
          "rustc"
          "rustfmt"
        ];
        
        platform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };
        
        cargoToml = pkgs.lib.importTOML ./Cargo.toml;
      in
      {
        packages.default = platform.buildRustPackage rec {
          pname = "pgdb";
          version = cargoToml.workspace.package.version;
          nativeBuildInputs = with pkgs; [ postgresql ];

          # Tests require spawning a PostgreSQL instance and connecting to it, which is
          # impossible in the nix sandbox. The sandbox runs builds in an isolated network
          # namespace where even localhost is unreachable, and Unix sockets cannot be
          # placed in shared locations.
          #
          # See:
          # - https://discourse.nixos.org/t/spin-up-postgres-for-testing-and-connect-to-it-during-build-test/17804
          # - https://github.com/NixOS/nix/issues/4584
          # - https://discourse.nixos.org/t/nix-build-sandbox-networking/45448
          #
          # Run tests locally with `cargo test` instead.
          doCheck = false;

          src = pkgs.lib.cleanSource ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };
      });
}
