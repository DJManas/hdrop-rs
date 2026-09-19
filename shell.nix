let
  pkgs = import <nixpkgs> {};
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    cargo         # The Rust package manager
    rustc         # The Rust compiler
    rustfmt       # Code formatter
    clippy        # Linter to catch mistakes
    rust-analyzer # CRITICAL: Zed needs this to provide code completion!
  ];

  # Optional: Set environment variables if needed
  RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

  shellHook = ''
    export PS1="{\e[0;33m$(date +%H:%M)\e0} \e[0;31mhdrop-rs\e0 \e[0;32m\W >\e0 "
  '';
}
