{
  description = "hdrop-rs: a Rust port of hdrop, a Hyprland dropdown-window helper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          cargoToml = pkgs.lib.importTOML ./Cargo.toml;
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = cargoToml.package.name;
            version = cargoToml.package.version;

            src = ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
              # hyprland/hyprland-macros are pinned to git branch=master in Cargo.toml (no
              # published release supports Hyprland's post-0.55 Lua dispatch protocol yet - see
              # src/lib.rs), so their fetch needs an explicit hash. Both come from the same repo
              # revision, so one entry covers both. Re-run `cargo update -p hyprland` + re-prefetch
              # (`nix run nixpkgs#nix-prefetch-git -- --url <repo> --rev <new rev>`) whenever the
              # pin moves.
              outputHashes = {
                "hyprland-0.4.0-beta.3" = "sha256-uhCZylXf81fvlBd6ToIOrlr3InVXLzv25LyvxxpmLWo=";
              };
            };

            meta = {
              description = "A Rust port of hdrop, a dropdown-window helper for Hyprland";
              mainProgram = "hdrop-rs";
              platforms = pkgs.lib.platforms.linux;
              license = pkgs.lib.licenses.gpl3Plus;
            };
          };
        });

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
            ];

            RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          };
        });
    };
}
