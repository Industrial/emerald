{
  description = "Emerald — a statically-typed, AOT-compiled Ruby derivative (LLVM native + WASM)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {inherit system;};

        # Mirrors devenv.nix's own proven build environment exactly —
        # LLVM 21 (inkwell's `llvm21-1` feature, crates/emerald-codegen/
        # Cargo.toml), libffi + libxml2 as LLVM's own link dependencies,
        # LLVM_SYS_211_PREFIX because llvm-sys looks for this exact env
        # var name to find llvm-config without needing it on PATH.
        emerald = pkgs.rustPlatform.buildRustPackage {
          pname = "emerald";
          version = "0.1.0";
          src = self;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            llvmPackages_21.llvm
          ];

          buildInputs = with pkgs; [
            llvmPackages_21.llvm
            libffi
            libxml2
          ];

          env.LLVM_SYS_211_PREFIX = "${pkgs.llvmPackages_21.llvm.dev}";

          # Only the two binaries this package actually ships — not
          # emerald-mcp (an editor/agent-integration server, not part
          # of "install the language") and not the lib-only crates.
          cargoBuildFlags = ["-p" "emerald-cli" "-p" "emerald-lsp"];
          # cargo test/nextest needs a real toolchain + runtime linking
          # this sandboxed build doesn't set up (see AGENTS.md for the
          # real gate: `cargo nextest run --workspace`, run in CI/
          # devenv, not the Nix build itself) — this package proves it
          # *builds*, not that it passes the test suite.
          doCheck = false;

          meta = {
            description = "Statically-typed, ahead-of-time compiled Ruby derivative — LLVM native + WASM codegen, actor-model concurrency with supervision trees";
            homepage = "https://github.com/Industrial/emerald";
            license = with pkgs.lib.licenses; [mit asl20];
            mainProgram = "emerald";
          };
        };
      in {
        packages = {
          default = emerald;
          inherit emerald;
          # Same derivation — both `emerald` and `emerald-lsp` land in
          # its $out/bin. Aliased so `nix build .#emerald-lsp` and
          # editor tooling that expects a same-named package both work.
          emerald-lsp = emerald;
        };

        apps.default = flake-utils.lib.mkApp {drv = emerald;};

        formatter = pkgs.alejandra;
      }
    );
}
