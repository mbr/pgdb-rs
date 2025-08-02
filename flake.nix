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
          
          src = pkgs.lib.cleanSource ./.;
          
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            toolchain
            postgresql
            # Add any other development tools you need
            cargo-watch
            rust-analyzer
          ];
        };
      });
}
