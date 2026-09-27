{
  description = "demogod: records terminal and browser demos from a tape";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forEachSystem (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "demogod";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # The tests start shells and browsers, which the build sandbox does not have.
          doCheck = false;
          meta = {
            description = "Records terminal and browser demos from a tape";
            homepage = "https://github.com/izelnakri/demogod";
            license = pkgs.lib.licenses.mit;
            mainProgram = "demogod";
          };
        };
      });

      apps = forEachSystem (pkgs: {
        default = { type = "app"; program = "${self.packages.${pkgs.system}.default}/bin/demogod"; };
      });

      devShells = forEachSystem (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy pkgs.cargo-llvm-cov
            pkgs.nodejs_24 pkgs.ffmpeg pkgs.git-cliff pkgs.bat pkgs.zsh
          ] ++ pkgs.lib.optional pkgs.stdenv.isLinux pkgs.chromium;
          CHROME_BIN = pkgs.lib.optionalString pkgs.stdenv.isLinux "${pkgs.chromium}/bin/chromium";
        };
      });
    };
}
